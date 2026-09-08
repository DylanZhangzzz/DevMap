use super::{
    identity, location, platform,
    protocol::{self, Hello, Request, Welcome},
    transport::{self, invalid},
};
use fs2::FileExt;
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
pub(super) fn run(source: &Path, instance: String, idle_seconds: u64) -> io::Result<()> {
    let id = identity(source)?;
    let location = location(&id)?;
    let lock = platform::lock_file(&location.directory.join("owner.lock"))?;
    lock.try_lock_exclusive()?;
    let build = super::build()?;
    super::reactor()?.block_on(async move {
        let mut listener = platform::Listener::bind(&location.endpoint)?;
        let permits = Arc::new(tokio::sync::Semaphore::new(protocol::MAX_CONNECTIONS));
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            // Idle timer starts only once all client tasks have completed. Each
            // client task owns one bounded frame, so no unbounded executor queue.
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
                    jobs.spawn(async move {
                        let _permit = permit;
                        let _ = serve(stream, repo, build, instance).await;
                    });
                }
                Ok(Err(e)) => return Err(e),
                Err(_) if jobs.is_empty() => return Ok(()),
                Err(_) => {}
            }
        }
    })
}
async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    repository: String,
    build: String,
    instance: String,
) -> io::Result<()> {
    let hello: Hello = transport::read(&mut stream, Duration::from_secs(2)).await?;
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
    let source = hello.source.clone();
    let id = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || identity(&source)),
    )
    .await
    .map_err(|_| invalid("runtime identity deadline"))?
    .map_err(|_| invalid("runtime identity worker failed"))??;
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
        match request {
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
