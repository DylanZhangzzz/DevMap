//! Fixed-membership query execution. Admission and transport keep ownership.
use super::*;

pub(super) fn run(
    jobs: impl IntoIterator<Item = Job>,
    execute: &mut impl FnMut(&Identity, &[u8]) -> Result<ApplicationResult, DevMapError>,
) {
    let mut jobs = jobs.into_iter().peekable();
    while let Some(first) = jobs.next() {
        let mut group = vec![first];
        while group.len() < protocol::MAX_EXCHANGES
            && jobs.peek().is_some_and(|next| equivalent(&group[0], next))
        {
            group.push(jobs.next().unwrap());
        }
        // Membership is closed before any live filesystem validation begins.
        if group.len() == 1 {
            single(group.pop().unwrap(), execute);
        } else {
            shared(group, execute);
        }
    }
}

fn equivalent(first: &Job, next: &Job) -> bool {
    use super::super::query_validation::QueryOrigin;
    matches!(first.query_origin, Some(QueryOrigin::Verified { .. }))
        && first.query_origin == next.query_origin
        && first.identity.source == next.identity.source
        && first.identity.git_dir == next.identity.git_dir
        && first.identity.common == next.identity.common
        && first.identity.repository == next.identity.repository
        && first.bytes == next.bytes
        && matches!(
            serde_json::from_slice::<ApplicationRequest>(&first.bytes),
            Ok(ApplicationRequest::Query { .. })
        )
}

fn encode(result: &ApplicationResult) -> Vec<u8> {
    transport::bounded_json(result, protocol::MAX_RESULT).unwrap_or_else(|e| {
        transport::bounded_json(
            &ApplicationResult::Error {
                error: DomainError::ResponseLimit {
                    message: e.to_string(),
                },
            },
            protocol::MAX_RESULT,
        )
        .expect("bounded diagnostic")
    })
}

fn send(job: Job, bytes: Vec<u8>) {
    let _ = job.reply.send(Completed {
        bytes,
        _reservation: job.reservation,
    });
}

fn send_error(mut job: Job, error: DevMapError) {
    drop(std::mem::take(&mut job.bytes));
    send(
        job,
        encode(&ApplicationResult::Error {
            error: error.into(),
        }),
    );
}

fn single(
    mut job: Job,
    execute: &mut impl FnMut(&Identity, &[u8]) -> Result<ApplicationResult, DevMapError>,
) {
    let result = if crate::git_process::healthy() {
        super::super::query_validation::with_query_origin(
            &job.bytes,
            job.query_origin.as_ref(),
            || execute(&job.identity, &job.bytes),
        )
    } else {
        Err(crate::git_process::GitProcessError::CleanupFailed.into())
    }
    .unwrap_or_else(|e| ApplicationResult::Error { error: e.into() });
    drop(std::mem::take(&mut job.bytes));
    send(job, encode(&result));
}

fn shared(
    jobs: Vec<Job>,
    execute: &mut impl FnMut(&Identity, &[u8]) -> Result<ApplicationResult, DevMapError>,
) {
    let mut valid = Vec::with_capacity(jobs.len());
    for job in jobs {
        let precheck = if crate::git_process::healthy() {
            job.query_origin
                .as_ref()
                .expect("eligible origin")
                .validate()
        } else {
            Err(crate::git_process::GitProcessError::CleanupFailed.into())
        };
        match precheck {
            Ok(()) => valid.push(job),
            Err(error) => send_error(job, error),
        }
    }
    let Some(first) = valid.first() else {
        return;
    };
    let result = execute(&first.identity, &first.bytes);
    // As in the original single-request path, an operation error short-circuits
    // closing validation. It belongs only to this closed cohort, never a cache.
    let checks = valid
        .iter()
        .map(|job| {
            if result.is_ok() {
                job.query_origin.as_ref().unwrap().validate()
            } else {
                Ok(())
            }
        })
        .collect::<Vec<_>>();
    for job in &mut valid {
        drop(std::mem::take(&mut job.bytes));
    }
    let result = result.unwrap_or_else(|error| ApplicationResult::Error {
        error: error.into(),
    });
    let mut bytes = encode(&result);
    drop(result);
    let count = valid.len();
    for (index, (job, check)) in valid.into_iter().zip(checks).enumerate() {
        match check {
            Ok(()) => send(
                job,
                if index + 1 == count {
                    std::mem::take(&mut bytes)
                } else {
                    bytes.clone()
                },
            ),
            Err(error) => send_error(job, error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_pair() -> (
        tempfile::TempDir,
        Vec<Job>,
        Vec<oneshot::Receiver<Completed>>,
        Arc<Semaphore>,
    ) {
        let fixture = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(fixture.path())
                .args(["init", "--quiet"])
                .output()
                .unwrap()
                .status
                .success()
        );
        let identity = super::super::super::identity(fixture.path()).unwrap();
        let origin =
            super::super::super::query_validation::QueryOrigin::capture(&identity).unwrap();
        let bytes = serde_json::to_vec(&ApplicationRequest::Query {
            query: crate::application::ClientQuery {
                tasks: vec![],
                inventory_observed_at: None,
                complete: false,
                previous_heads: vec![],
            },
        })
        .unwrap();
        let memory = Arc::new(Semaphore::new(2 * protocol::EXCHANGE_RESERVATION));
        let mut receivers = Vec::new();
        let jobs = (0..2)
            .map(|_| {
                let (reply, receiver) = oneshot::channel();
                receivers.push(receiver);
                Job {
                    identity: identity.clone(),
                    query_origin: Some(origin.clone()),
                    bytes: bytes.clone(),
                    reservation: memory
                        .clone()
                        .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                        .unwrap(),
                    reply,
                }
            })
            .collect();
        (fixture, jobs, receivers, memory)
    }

    #[test]
    fn disconnected_member_does_not_cancel_a_shared_real_projection() {
        let (_fixture, jobs, mut receivers, memory) = real_pair();
        drop(receivers.remove(0));
        let mut app = None;
        let mut queries = super::super::super::query_validation::QueryValidation::default();
        let mut calls = 0;
        run(jobs, &mut |identity, bytes| {
            calls += 1;
            super::super::execute_with_queries(&mut app, &mut queries, identity, bytes)
        });
        let completed = receivers.remove(0).blocking_recv().unwrap();
        let result: ApplicationResult = serde_json::from_slice(&completed.bytes).unwrap();
        assert!(matches!(result, ApplicationResult::Snapshot { snapshot }
            if snapshot.model.schema_version == "devmap/dock/4"));
        assert_eq!(calls, 1);
        assert_eq!(memory.available_permits(), protocol::EXCHANGE_RESERVATION);
        drop(completed);
        assert_eq!(
            memory.available_permits(),
            2 * protocol::EXCHANGE_RESERVATION
        );
    }

    #[test]
    fn origin_change_after_real_projection_refuses_every_member() {
        let (fixture, jobs, receivers, memory) = real_pair();
        let root = fixture.path().canonicalize().unwrap();
        let admin = root.join(".git");
        let saved = root.join("owned-saved-git");
        // Both move targets belong to this exact disposable fixture.
        assert_eq!(admin.parent(), Some(root.as_path()));
        assert_eq!(saved.parent(), Some(root.as_path()));
        assert!(!saved.exists());
        let mut app = None;
        let mut queries = super::super::super::query_validation::QueryValidation::default();
        let mut calls = 0;
        run(jobs, &mut |identity, bytes| {
            calls += 1;
            let result =
                super::super::execute_with_queries(&mut app, &mut queries, identity, bytes)?;
            assert!(matches!(result, ApplicationResult::Snapshot { .. }));
            std::fs::rename(&admin, &saved).unwrap();
            Ok(result)
        });
        std::fs::rename(&saved, &admin).unwrap();
        for receiver in receivers {
            let completed = receiver.blocking_recv().unwrap();
            let result: ApplicationResult = serde_json::from_slice(&completed.bytes).unwrap();
            assert!(matches!(result, ApplicationResult::Error { error }
                if error.message().contains("authenticated source changed")));
        }
        assert_eq!(calls, 1);
        assert_eq!(
            memory.available_permits(),
            2 * protocol::EXCHANGE_RESERVATION
        );
    }

    #[test]
    fn differing_query_bytes_and_later_cohorts_never_reuse_results() {
        let fixture = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(fixture.path())
                .args(["init", "--quiet"])
                .output()
                .unwrap()
                .status
                .success()
        );
        let identity = super::super::super::identity(fixture.path()).unwrap();
        let origin =
            super::super::super::query_validation::QueryOrigin::capture(&identity).unwrap();
        // This tests the executor core directly, not transport admission capacity.
        let memory = Arc::new(Semaphore::new(3 * protocol::EXCHANGE_RESERVATION));
        let mut calls = 0;
        for _ in 0..2 {
            let mut receivers = Vec::new();
            let jobs = [false, true, false].map(|complete| {
                let bytes = serde_json::to_vec(&ApplicationRequest::Query {
                    query: crate::application::ClientQuery {
                        tasks: vec![],
                        inventory_observed_at: None,
                        complete,
                        previous_heads: vec![],
                    },
                })
                .unwrap();
                let (reply, receiver) = oneshot::channel();
                receivers.push(receiver);
                Job {
                    identity: identity.clone(),
                    query_origin: Some(origin.clone()),
                    bytes,
                    reservation: memory
                        .clone()
                        .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                        .unwrap(),
                    reply,
                }
            });
            run(jobs, &mut |_, _| {
                calls += 1;
                Err(DevMapError::Store(format!(
                    "independent observation {calls}"
                )))
            });
            let responses = receivers
                .into_iter()
                .map(|r| r.blocking_recv().unwrap())
                .collect::<Vec<_>>();
            assert_ne!(
                responses[0].bytes, responses[2].bytes,
                "nonmatching query is a FIFO barrier"
            );
            drop(responses);
            assert_eq!(
                memory.available_permits(),
                3 * protocol::EXCHANGE_RESERVATION
            );
        }
        assert_eq!(calls, 6, "a later invocation must make fresh observations");
    }

    #[test]
    fn missing_or_refused_connection_proofs_cannot_join_a_query_cohort() {
        use super::super::super::query_validation::QueryOrigin;
        let identity = Identity {
            source: ".".into(),
            git_dir: ".".into(),
            common: ".".into(),
            repository: "unsealed-test".into(),
        };
        let bytes = serde_json::to_vec(&ApplicationRequest::Query {
            query: crate::application::ClientQuery {
                tasks: vec![],
                inventory_observed_at: None,
                complete: false,
                previous_heads: vec![],
            },
        })
        .unwrap();
        for refused in [false, true] {
            let memory = Arc::new(Semaphore::new(2 * protocol::EXCHANGE_RESERVATION));
            let mut receivers = Vec::new();
            let jobs = (0..2)
                .map(|_| {
                    let (reply, receiver) = oneshot::channel();
                    receivers.push(receiver);
                    Job {
                        identity: identity.clone(),
                        query_origin: refused.then_some(QueryOrigin::Refused),
                        bytes: bytes.clone(),
                        reservation: memory
                            .clone()
                            .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                            .unwrap(),
                        reply,
                    }
                })
                .collect::<Vec<_>>();
            let mut calls = 0;
            run(jobs, &mut |_, _| {
                calls += 1;
                Err(DevMapError::Store("unsealed call".into()))
            });
            for receiver in receivers {
                let completed = receiver.blocking_recv().unwrap();
                let result: ApplicationResult = serde_json::from_slice(&completed.bytes).unwrap();
                assert!(
                    matches!(result, ApplicationResult::Error { error } if error.message().contains(
                    if refused { "authenticated source changed" } else { "unsealed call" }))
                );
            }
            assert_eq!(calls, if refused { 0 } else { 2 });
            assert_eq!(
                memory.available_permits(),
                2 * protocol::EXCHANGE_RESERVATION
            );
        }
    }

    #[test]
    fn fixed_identical_queries_execute_once_with_independent_response_reservations() {
        let fixture = tempfile::tempdir().unwrap();
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.path())
            .args(["init", "--quiet"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let identity = super::super::super::identity(fixture.path()).unwrap();
        let origin =
            super::super::super::query_validation::QueryOrigin::capture(&identity).unwrap();
        let bytes = serde_json::to_vec(&ApplicationRequest::Query {
            query: crate::application::ClientQuery {
                tasks: vec![],
                inventory_observed_at: None,
                complete: false,
                previous_heads: vec![],
            },
        })
        .unwrap();
        let memory = Arc::new(Semaphore::new(2 * protocol::EXCHANGE_RESERVATION));
        let mut jobs = Vec::new();
        let mut receivers = Vec::new();
        for _ in 0..2 {
            let reservation = memory
                .clone()
                .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                .unwrap();
            let (reply, receiver) = oneshot::channel();
            jobs.push(Job {
                identity: identity.clone(),
                query_origin: Some(origin.clone()),
                bytes: bytes.clone(),
                reservation,
                reply,
            });
            receivers.push(receiver);
        }
        let mut calls = 0;
        run(jobs, &mut |_, _| {
            calls += 1;
            Err(DevMapError::Store("fixed observation failed".into()))
        });
        let first = receivers.remove(0).blocking_recv().unwrap();
        let second = receivers.remove(0).blocking_recv().unwrap();
        assert_eq!(first.bytes, second.bytes);
        let response: ApplicationResult = serde_json::from_slice(&first.bytes).unwrap();
        assert!(matches!(response, ApplicationResult::Error { error }
            if error.message().contains("fixed observation failed")));
        assert_eq!(memory.available_permits(), 0);
        drop(first);
        assert_eq!(memory.available_permits(), protocol::EXCHANGE_RESERVATION);
        drop(second);
        assert_eq!(
            memory.available_permits(),
            2 * protocol::EXCHANGE_RESERVATION
        );
        assert_eq!(
            calls, 1,
            "one closed cohort must share its single observation error"
        );
    }
}
