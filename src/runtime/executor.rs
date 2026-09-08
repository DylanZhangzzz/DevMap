//! One bounded, serial application thread. Disconnect never cancels accepted work.
use super::{
    Identity,
    protocol::{self, ApplicationRequest, ApplicationResult, DomainError},
    transport,
};
use crate::{application::RepositoryApplication, error::DevMapError, git::SourceGitInspector};
use std::{
    io,
    sync::{Arc, mpsc},
    thread,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

pub(super) struct Completed {
    pub bytes: Vec<u8>,
    pub _reservation: OwnedSemaphorePermit,
}
pub(super) struct Job {
    pub identity: Identity,
    pub bytes: Vec<u8>,
    pub reservation: OwnedSemaphorePermit,
    pub reply: oneshot::Sender<Completed>,
}
#[derive(Clone)]
pub(super) struct Admission {
    pub sender: mpsc::SyncSender<Job>,
    pub memory: Arc<Semaphore>,
}
pub(super) struct Executor {
    admission: Option<Admission>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Executor {
    pub fn start() -> io::Result<Self> {
        let mut app = None;
        Self::start_with(move |identity, bytes| execute(&mut app, identity, bytes))
    }
    fn start_with(
        mut execute: impl FnMut(&Identity, &[u8]) -> Result<ApplicationResult, DevMapError>
        + Send
        + 'static,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let memory = Arc::new(Semaphore::new(
            protocol::EXCHANGE_RESERVATION * protocol::MAX_EXCHANGES,
        ));
        let worker = thread::Builder::new()
            .name("devmap-application".into())
            .spawn(move || {
                for job in receiver {
                    let result = execute(&job.identity, &job.bytes)
                        .unwrap_or_else(|e| ApplicationResult::Error { error: e.into() });
                    // Release the request before encoding the fixed result. The reservation
                    // moves into the response and survives a disconnected receiver until now.
                    drop(job.bytes);
                    let bytes = transport::bounded_json(&result, protocol::MAX_RESULT)
                        .unwrap_or_else(|e| {
                            transport::bounded_json(
                                &ApplicationResult::Error {
                                    error: DomainError::ResponseLimit {
                                        message: e.to_string(),
                                    },
                                },
                                protocol::MAX_RESULT,
                            )
                            .expect("bounded diagnostic")
                        });
                    let _ = job.reply.send(Completed {
                        bytes,
                        _reservation: job.reservation,
                    });
                }
            })?;
        Ok(Self {
            admission: Some(Admission { sender, memory }),
            worker: Some(worker),
        })
    }
    pub fn admission(&self) -> Admission {
        self.admission.as_ref().unwrap().clone()
    }
}
impl Drop for Executor {
    fn drop(&mut self) {
        // Caller drops the reactor first, so every remaining sender belongs to
        // this object. A stalled application keeps ownership: threads cannot be
        // cancelled safely while they might still commit repository transactions.
        self.admission.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn execute(
    app: &mut Option<RepositoryApplication>,
    identity: &Identity,
    bytes: &[u8],
) -> Result<ApplicationResult, DevMapError> {
    let request: ApplicationRequest = serde_json::from_slice(bytes)?;
    let workspace = SourceGitInspector::open(&identity.source)?.workspace_allow_unborn()?;
    if std::fs::canonicalize(&workspace.root)? != identity.source
        || std::fs::canonicalize(&workspace.git_dir)? != identity.git_dir
        || std::fs::canonicalize(&workspace.git_common_dir)? != identity.common
    {
        return Err(DevMapError::Store("authenticated source changed".into()));
    }
    if let ApplicationRequest::Mutate { command } = request {
        let mutation = command.execute(&workspace)?;
        if let Some(app) = app {
            app.reconcile();
        }
        return Ok(ApplicationResult::Mutation { mutation });
    }
    if app.is_none() {
        *app = Some(RepositoryApplication::open(&workspace)?);
    }
    let app = app.as_mut().unwrap();
    match request {
        ApplicationRequest::Mutate { .. } => unreachable!(),
        ApplicationRequest::Query { query } => Ok(ApplicationResult::Snapshot {
            snapshot: Box::new(app.project(&workspace, &query, OffsetDateTime::now_utc())?),
        }),
        ApplicationRequest::AcceptInventory {
            prior,
            tasks,
            complete,
            observed_at,
        } => {
            let observed_at = OffsetDateTime::parse(&observed_at, &Rfc3339)
                .map_err(|_| DevMapError::InvalidDomain("inventory observation time"))?;
            Ok(ApplicationResult::Inventory {
                query: app.accept_inventory_query(
                    &workspace,
                    prior,
                    tasks,
                    complete,
                    observed_at,
                )?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::{fs::OpenOptions, time::Duration};
    fn dummy_identity() -> Identity {
        Identity {
            source: ".".into(),
            git_dir: ".".into(),
            common: ".".into(),
            repository: "test".into(),
        }
    }
    #[test]
    fn disconnected_running_job_retains_reservation_and_owner_lock_until_completion() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let executor = Executor::start_with(move |_, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(ApplicationResult::Error {
                error: DomainError::Domain {
                    message: "completed".into(),
                },
            })
        })
        .unwrap();
        let admission = executor.admission();
        let reservation = admission
            .memory
            .clone()
            .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        admission
            .sender
            .try_send(Job {
                identity: dummy_identity(),
                bytes: vec![1],
                reservation,
                reply,
            })
            .ok()
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(receiver);
        assert_eq!(
            admission.memory.available_permits(),
            protocol::EXCHANGE_RESERVATION
        );
        let reservation = admission
            .memory
            .clone()
            .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
            .unwrap();
        let (queued_reply, queued_receiver) = oneshot::channel();
        admission
            .sender
            .try_send(Job {
                identity: dummy_identity(),
                bytes: vec![2],
                reservation,
                reply: queued_reply,
            })
            .ok()
            .unwrap();
        drop(queued_receiver);
        assert_eq!(admission.memory.available_permits(), 0);
        assert!(
            admission
                .memory
                .clone()
                .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                .is_err(),
            "third exchange admitted while running and queued jobs are outstanding"
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("owner.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let memory = admission.memory.clone();
        drop(admission);
        let (done_tx, done_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            super::super::owner::finish_owner(
                super::super::reactor().unwrap(),
                lock,
                Ok(()),
                executor,
            )
            .unwrap();
            done_tx.send(()).unwrap();
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert!(probe.try_lock_exclusive().is_err());
        assert_eq!(memory.available_permits(), 0);
        release_tx.send(()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(probe.try_lock_exclusive().is_err());
        assert_eq!(memory.available_permits(), protocol::EXCHANGE_RESERVATION);
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        join.join().unwrap();
        assert_eq!(
            memory.available_permits(),
            protocol::EXCHANGE_RESERVATION * protocol::MAX_EXCHANGES
        );
        probe.try_lock_exclusive().unwrap();
    }
}

#[cfg(test)]
mod sharing_tests {
    use super::*;
    use std::{path::Path, process::Command, time::Duration};
    fn git(path: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[test]
    fn four_serial_clients_share_git_cycle_with_explicit_sixty_second_test_policy() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "initial",
            ],
        );
        let linked = temp.path().join("linked");
        git(
            &root,
            &["worktree", "add", "--quiet", linked.to_str().unwrap()],
        );
        let mut app = None;
        let executor = Executor::start_with(move |id, bytes| {
            if app.is_none() {
                let w = SourceGitInspector::open(&id.source)?.workspace_allow_unborn()?;
                app = Some(
                    RepositoryApplication::open(&w)?.with_git_max_age(Duration::from_secs(60))?,
                );
            }
            execute(&mut app, id, bytes)
        })
        .unwrap();
        let admission = executor.admission();
        let mut current_ids = vec![];
        for i in 0..4 {
            let source = if i % 2 == 0 { &root } else { &linked };
            let workspace = SourceGitInspector::open(source)
                .unwrap()
                .workspace_allow_unborn()
                .unwrap();
            let id = Identity {
                source: std::fs::canonicalize(&workspace.root).unwrap(),
                git_dir: std::fs::canonicalize(&workspace.git_dir).unwrap(),
                common: std::fs::canonicalize(&workspace.git_common_dir).unwrap(),
                repository: "fixture".into(),
            };
            let stamp = format!("2026-09-08T00:00:0{i}Z");
            let task = crate::dock::ObservedTask {
                working_directory: None,
                subagents: None,
                lifecycle: crate::dock::TaskLifecycle::Present,
                session_id: format!("isolated-client-task-{i}"),
                display_title: format!("isolated client {i}"),
                host: "codex".into(),
                host_status: "running".into(),
                workspace_path: workspace.root.to_string_lossy().into(),
                status: crate::presence::PresenceStatus::Working,
                updated_at: stamp.clone(),
            };
            let query = crate::application::ClientQuery {
                tasks: vec![task],
                inventory_observed_at: Some(stamp.clone()),
                complete: i % 2 == 0,
                previous_heads: vec![],
            };
            let bytes = serde_json::to_vec(&ApplicationRequest::Query { query }).unwrap();
            let reservation = admission
                .memory
                .clone()
                .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                .unwrap();
            let (reply, receiver) = oneshot::channel();
            admission
                .sender
                .try_send(Job {
                    identity: id,
                    bytes,
                    reservation,
                    reply,
                })
                .ok()
                .unwrap();
            let completed = receiver.blocking_recv().unwrap();
            let result: ApplicationResult = serde_json::from_slice(&completed.bytes).unwrap();
            let ApplicationResult::Snapshot { snapshot } = result else {
                panic!("{result:?}")
            };
            assert_eq!(snapshot.git_cycle, 1);
            current_ids.push(snapshot.model.current_worktree_id.clone());
            let encoded = serde_json::to_value(&snapshot.model).unwrap();
            let text = encoded.to_string();
            assert!(
                text.contains(&format!("isolated-client-task-{i}")),
                "own inventory missing: {text}"
            );
            for other in 0..4 {
                if other != i {
                    assert!(
                        !text.contains(&format!("isolated-client-task-{other}")),
                        "another client inventory leaked: {text}"
                    );
                }
            }
            assert!(
                encoded.to_string().contains(&stamp),
                "client original timestamp disappeared: {encoded}"
            );
        }
        assert_eq!(current_ids[0], current_ids[2]);
        assert_eq!(current_ids[1], current_ids[3]);
        assert_ne!(current_ids[0], current_ids[1]);
        drop(admission);
        drop(executor);
    }
}
