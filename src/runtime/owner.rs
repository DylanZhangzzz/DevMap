use super::{
    location, platform,
    protocol::{self, Hello, Request, Welcome},
    transport::{self, invalid},
};
use fs2::FileExt;
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
pub(super) fn run(source: &Path, instance: String, idle_seconds: u64) -> io::Result<()> {
    #[cfg(windows)]
    platform::owner_lifetime::install()?;
    let reactor = super::reactor()?;
    let id = reactor.block_on(super::identity_async(source))?;
    let location = location(&id)?;
    let lock = platform::lock_file(&location.directory.join("owner.lock"))?;
    lock.try_lock_exclusive()?;
    // Rebind after the acquired lock: panic unwinding also drops the reactor first.
    let executor = super::executor::Executor::start()?;
    let admission = executor.admission();
    let owned_reactor = reactor;
    let result = super::build().and_then(|build| {
        owned_reactor.block_on(async move {
            let mut listener = platform::Listener::bind(&location.endpoint)?;
            let permits = Arc::new(tokio::sync::Semaphore::new(protocol::MAX_CONNECTIONS));
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                if super::IDENTITY_SHUTDOWN_FAILED.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(invalid("identity cleanup failed; owner stopping"));
                }
                if !crate::git_process::healthy() {
                    return Err(invalid("Git cleanup unconfirmed; owner quarantined"));
                }
                // Idle timer starts only once all client tasks have completed. Each
                // client task owns bounded frames; accepted work retains its reservation.
                while jobs.try_join_next().is_some() {}
                let accept_budget = if jobs.is_empty() {
                    Duration::from_secs(idle_seconds)
                } else {
                    Duration::from_secs(1)
                };

                match tokio::time::timeout(accept_budget, listener.accept()).await {
                    Ok(Ok(stream)) => {
                        let Ok(permit) = permits.clone().try_acquire_owned() else {
                            drop(stream);
                            continue;
                        };
                        let repo = id.repository.clone();
                        let build = build.clone();
                        let instance = instance.clone();
                        let admission = admission.clone();
                        jobs.spawn(async move {
                            let _permit = permit;
                            let _ = serve(stream, repo, build, instance, admission).await;
                        });
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_) if jobs.is_empty() => return Ok(()),
                    Err(_) => {}
                }
            }
        })
    });
    finish_owner(owned_reactor, lock, result, executor)
}

pub(super) fn finish_owner(
    reactor: tokio::runtime::Runtime,
    lock: std::fs::File,
    result: io::Result<()>,
    executor: impl Sized,
) -> io::Result<()> {
    // Pending task destructors may still own child/process/IPC resources. Keep
    // repository ownership until they have been dropped on either result path.
    drop(reactor);
    drop(executor);
    drop(lock);
    result
}

async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    repository: String,
    build: String,
    instance: String,
    admission: super::executor::Admission,
) -> io::Result<()> {
    let hello: Hello = transport::read(&mut stream, Duration::from_secs(2)).await?;
    if !crate::git_process::healthy() {
        return Err(invalid("Git cleanup unconfirmed; hello refused"));
    }
    if hello.protocol != protocol::VERSION
        || hello.repository != repository
        || hello.build != build
        || !super::valid_nonce(&hello.client_instance)
    {
        transport::write(
            &mut stream,
            &protocol::HelloReply::Rejected {
                reason: "protocol, repository, build, or client identity mismatch".into(),
            },
        )
        .await?;
        return Err(invalid("runtime hello rejected"));
    }
    let id = super::identity_async(&hello.source).await?;
    if id.source != hello.source || id.git_dir != hello.git_dir || id.repository != repository {
        transport::write(
            &mut stream,
            &protocol::HelloReply::Rejected {
                reason: "source identity mismatch".into(),
            },
        )
        .await?;
        return Err(invalid("runtime source identity rejected"));
    }
    // A failed physical observation permanently refuses Query on this connection.
    // Ping and existing fresh mutation/inventory paths are unaffected. The seal
    // is never serialized or supplied by the peer.
    let query_origin = Some(super::query_validation::QueryOrigin::for_connection(&id));
    let welcome = Welcome {
        protocol: protocol::VERSION,
        repository: repository.clone(),
        build,
        owner_instance: instance.clone(),
        owner_pid: std::process::id(),
        client_instance: hello.client_instance.clone(),
    };
    transport::write(&mut stream, &protocol::HelloReply::Accepted { welcome }).await?;
    let mut last_request = 0;
    loop {
        let request: Request = transport::read(&mut stream, transport::IO_DEADLINE).await?;
        if !crate::git_process::healthy() {
            return Err(invalid("Git cleanup unconfirmed; admission refused"));
        }
        match request {
            Request::Begin {
                protocol: version,
                repository: repo,
                client_instance,
                request_id,
                owner_instance,
                total,
                digest,
            } => {
                if version != protocol::VERSION
                    || repo != repository
                    || client_instance != hello.client_instance
                    || request_id <= last_request
                    || owner_instance != instance
                {
                    return Err(invalid("runtime request identity rejected"));
                }
                last_request = request_id;
                let tag = protocol::Transfer {
                    request_id,
                    owner_instance,
                    total,
                    digest,
                };
                transport::validate_transfer(&tag, protocol::MAX_REQUEST)?;
                let Ok(reservation) = admission
                    .memory
                    .clone()
                    .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
                else {
                    transport::write(
                        &mut stream,
                        &protocol::ExchangeReply::Busy {
                            request_id,
                            owner_instance: instance.clone(),
                        },
                    )
                    .await?;
                    continue;
                };
                tokio::time::timeout(transport::EXCHANGE_DEADLINE, async {
                    transport::write(
                        &mut stream,
                        &protocol::ExchangeReply::Ready {
                            request_id,
                            owner_instance: instance.clone(),
                        },
                    )
                    .await?;
                    let bytes =
                        transport::receive_bytes(&mut stream, &tag, protocol::MAX_REQUEST).await?;
                    let (reply, receiver) = tokio::sync::oneshot::channel();
                    let job = super::executor::Job {
                        identity: id.clone(),
                        query_origin: query_origin.clone(),
                        bytes,
                        reservation,
                        reply,
                    };
                    if !crate::git_process::healthy() {
                        return Err(invalid("Git cleanup unconfirmed; dispatch refused"));
                    }
                    match admission.sender.try_send(job) {
                        Ok(()) => {}
                        Err(std::sync::mpsc::TrySendError::Full(_)) => {
                            transport::write(
                                &mut stream,
                                &protocol::ExchangeReply::Busy {
                                    request_id,
                                    owner_instance: instance.clone(),
                                },
                            )
                            .await?;
                            return Ok(());
                        }
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                            return Err(io::Error::other("runtime executor unavailable"));
                        }
                    }
                    let completed = receiver
                        .await
                        .map_err(|_| io::Error::other("runtime executor stopped"))?;
                    let result_tag = transport::transfer(&completed.bytes, request_id, &instance);
                    transport::write(
                        &mut stream,
                        &protocol::ExchangeReply::Result {
                            transfer: result_tag.clone(),
                        },
                    )
                    .await?;
                    transport::send_bytes(&mut stream, &result_tag, &completed.bytes).await?;
                    drop(completed);
                    Ok(())
                })
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "runtime exchange deadline")
                })??;
            }
            Request::Ping {
                protocol,
                repository: repo,
                client_instance,
                request_id,
            } => {
                if protocol != protocol::VERSION
                    || repo != repository
                    || client_instance != hello.client_instance
                    || request_id <= last_request
                {
                    return Err(invalid("runtime request identity rejected"));
                }
                last_request = request_id;
                transport::write(
                    &mut stream,
                    &protocol::Response {
                        request_id,
                        owner_instance: instance.clone(),
                    },
                )
                .await?;
            }
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use std::{
        fs::OpenOptions,
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
    };
    struct ObserveLockOnDrop {
        path: PathBuf,
        retained: Arc<AtomicBool>,
    }
    impl Drop for ObserveLockOnDrop {
        fn drop(&mut self) {
            let contender = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.path)
                .unwrap();
            self.retained.store(matches!(contender.try_lock_exclusive(), Err(error) if super::super::contended(&error)), Ordering::SeqCst);
        }
    }
    #[test]
    fn owner_lock_remains_held_during_reactor_cleanup_on_success_and_error() {
        for fail in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let path = temporary.path().join("owner.lock");
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            lock.try_lock_exclusive().unwrap();
            let reactor = super::super::reactor().unwrap();
            let retained = Arc::new(AtomicBool::new(false));
            let observer = ObserveLockOnDrop {
                path: path.clone(),
                retained: retained.clone(),
            };
            reactor.spawn(async move {
                let _observer = observer;
                std::future::pending::<()>().await;
            });
            reactor.block_on(tokio::task::yield_now());
            let result = if fail {
                Err(io::Error::other("simulated accept failure"))
            } else {
                Ok(())
            };
            assert_eq!(finish_owner(reactor, lock, result, ()).is_err(), fail);
            assert!(
                retained.load(Ordering::SeqCst),
                "reactor task cleanup ran after ownership lock release"
            );
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap()
                .try_lock_exclusive()
                .unwrap();
        }
    }
}
