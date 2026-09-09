//! Same-user, repository-scoped owner with bounded typed application exchanges.
mod executor;
mod owner;
pub(crate) mod query_validation;
mod retry;
pub(crate) use retry::retry_application;
pub mod protocol;
mod transport;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;
use fs2::FileExt;
use protocol::{Hello, VERSION, Welcome};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use transport::invalid;
#[cfg(unix)]
pub(crate) use unix as platform;
#[cfg(windows)]
pub(crate) use windows as platform;
#[cfg(windows)]
pub(crate) use windows::{IdentityTree as GitChildTree, prepare_identity as prepare_git_child};
static IDENTITY_SHUTDOWN_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
const START_BUDGET: Duration = Duration::from_secs(15);
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    source: PathBuf,
    git_dir: PathBuf,
    common: PathBuf,
    repository: String,
}
fn identity(path: &Path) -> io::Result<Identity> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--git-dir",
            "--show-toplevel",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide(&mut command);
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(invalid("runtime source is not a Git worktree"));
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Git identity deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(32769)
        .read_to_string(&mut out)?;
    if out.len() > 32768 {
        return Err(invalid("Git identity too large"));
    }
    let lines: Vec<_> = out.lines().collect();
    if lines.len() != 3 {
        return Err(invalid("Git identity has unexpected fields"));
    }
    let common = fs::canonicalize(lines[0])?;
    let git_dir = fs::canonicalize(lines[1])?;
    let source = fs::canonicalize(lines[2])?;
    let repository = crate::canonical::sha256_hex(
        format!("{}\n{}", common.display(), platform::user_id()?).as_bytes(),
    );
    Ok(Identity {
        source,
        git_dir,
        common,
        repository,
    })
}
fn build() -> io::Result<String> {
    let mut f = fs::File::open(std::env::current_exe()?)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn nonce() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_nonce(n: &str) -> bool {
    n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit())
}
fn hide(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}
struct Location {
    directory: PathBuf,
    endpoint: String,
}
fn location(id: &Identity) -> io::Result<Location> {
    #[cfg(windows)]
    let base = platform::runtime_base()?;
    #[cfg(unix)]
    let base = fs::canonicalize("/tmp")?;
    platform::validate_chain(&base)?;
    let base = fs::canonicalize(base)?;
    platform::validate_chain(&base)?;
    let user = crate::canonical::sha256_hex(platform::user_id()?.as_bytes());
    let root = base.join(format!("devmap-runtime-{}", &user[..16]));
    let directory = root.join(&id.repository[..32]);
    if directory.starts_with(&id.common) {
        return Err(invalid("runtime directory is inside Git administration"));
    }
    platform::private_dir(&root)?;
    platform::private_dir(&directory)?;
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\devmap-{}", id.repository);
    #[cfg(unix)]
    let endpoint = directory
        .join("ipc")
        .to_str()
        .ok_or_else(|| invalid("non-UTF8 runtime path"))?
        .to_owned();
    Ok(Location {
        directory,
        endpoint,
    })
}
fn reactor() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}
/// Synchronous facade for current synchronous MCP/hook callers. One in-flight call
/// per client; a disconnected client may connect again without affecting peers.
pub struct RuntimeClient {
    reactor: tokio::runtime::Runtime,
    stream: platform::Client,
    hello: Hello,
    pub welcome: Welcome,
    next_request: u64,
}
impl RuntimeClient {
    pub fn connect(source: &Path) -> io::Result<Self> {
        Self::connect_with_idle(source, 60)
    }
    fn connect_with_idle(source: &Path, idle_seconds: u64) -> io::Result<Self> {
        let reactor = reactor()?;
        let id = reactor.block_on(identity_async(source))?;
        let location = location(&id)?;
        let hello = Hello {
            protocol: VERSION,
            repository: id.repository,
            build: build()?,
            source: id.source,
            git_dir: id.git_dir,
            client_instance: nonce()?,
        };
        let connect = |expected: Option<&str>| {
            reactor.block_on(connect_once(&location.endpoint, &hello, expected))
        };
        match connect(None) {
            Ok((stream, welcome)) => {
                return Ok(Self {
                    reactor,
                    stream,
                    hello,
                    welcome,
                    next_request: 0,
                });
            }
            Err(e) if unavailable(&e) => {}
            Err(e) => return Err(e),
        }
        let deadline = Instant::now() + START_BUDGET;
        let startup = platform::lock_file(&location.directory.join("startup.lock"))?;
        loop {
            match startup.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) if contended(&e) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "runtime startup lock deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let owner_lock = platform::lock_file(&location.directory.join("owner.lock"))?;
        let mut expected = None;
        let mut spawned: Option<platform::SpawnedOwner> = None;
        loop {
            match connect(expected.as_deref()) {
                Ok((stream, welcome)) => {
                    if let Some(child) = spawned.take() {
                        child.detach();
                    }
                    return Ok(Self {
                        reactor,
                        stream,
                        hello,
                        welcome,
                        next_request: 0,
                    });
                }
                Err(e) if unavailable(&e) => {}
                Err(e) => return Err(e),
            }
            if expected.is_none() {
                match owner_lock.try_lock_exclusive() {
                    Ok(()) => {
                        FileExt::unlock(&owner_lock)?;
                        let instance = nonce()?;
                        spawned = Some(platform::spawn_owner(
                            &std::env::current_exe()?,
                            &hello.source,
                            &instance,
                            idle_seconds,
                        )?);
                        expected = Some(instance);
                    }
                    Err(e) if contended(&e) => {}
                    Err(e) => return Err(e),
                }
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "runtime owner unavailable; no takeover while owner lock is held",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    pub fn ping(&mut self) -> io::Result<()> {
        self.next_request += 1;
        let request = protocol::Request::Ping {
            protocol: VERSION,
            repository: self.hello.repository.clone(),
            client_instance: self.hello.client_instance.clone(),
            request_id: self.next_request,
        };
        self.reactor.block_on(async {
            transport::write(&mut self.stream, &request).await?;
            let response: protocol::Response =
                transport::read(&mut self.stream, transport::IO_DEADLINE).await?;
            if response.request_id != self.next_request
                || response.owner_instance != self.welcome.owner_instance
            {
                return Err(invalid("runtime response identity mismatch"));
            }
            Ok(())
        })
    }
}
fn contended(e: &io::Error) -> bool {
    e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}
fn unavailable(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
    ) || e.raw_os_error() == Some(231)
}
async fn connect_once(
    endpoint: &str,
    hello: &Hello,
    expected: Option<&str>,
) -> io::Result<(platform::Client, Welcome)> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = platform::connect(endpoint).await?;
        transport::write(&mut stream, hello).await?;
        let welcome = match transport::read(&mut stream, Duration::from_secs(2)).await? {
            protocol::HelloReply::Accepted { welcome } => welcome,
            protocol::HelloReply::Rejected { reason } => {
                return Err(invalid(format!("runtime handshake rejected: {reason}")));
            }
        };
        if welcome.protocol != VERSION
            || welcome.repository != hello.repository
            || welcome.build != hello.build
            || welcome.client_instance != hello.client_instance
            || !valid_nonce(&welcome.owner_instance)
            || expected.is_some_and(|e| e != welcome.owner_instance)
        {
            return Err(invalid("runtime handshake mismatch"));
        }
        Ok((stream, welcome))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "runtime handshake deadline"))?
}
pub(crate) fn dispatch(
    args: crate::cli::RuntimeArgs,
) -> Result<crate::CommandOutput, crate::error::DevMapError> {
    let result = (|| -> io::Result<String> {
        if args.identity {
            serde_json::to_string(&identity(&args.source)?).map_err(|e| invalid(e.to_string()))
        } else if args.owner {
            let instance = args
                .instance
                .ok_or_else(|| invalid("owner instance required"))?;
            if !valid_nonce(&instance) {
                return Err(invalid("invalid owner instance"));
            }
            owner::run(&args.source, instance, args.idle_seconds)?;
            Ok(String::new())
        } else {
            let mut client = RuntimeClient::connect_with_idle(&args.source, args.idle_seconds)?;
            client.ping()?;
            serde_json::to_string(&client.welcome).map_err(|e| invalid(e.to_string()))
        }
    })();
    result
        .map(|stdout| crate::CommandOutput {
            stdout,
            exit_code: 0,
        })
        .map_err(|e| crate::error::DevMapError::Store(format!("runtime: {e}")))
}

#[cfg(test)]
mod bounded_identity_tests {
    use super::*;
    #[test]
    // Intentional fault fixture: the parent exits while its descendant retains
    // the pipe, so the external identity supervisor must clean the owned tree.
    #[allow(clippy::zombie_processes)]
    fn identity_stall_fixture() {
        if std::env::var_os("DEVMAP_TEST_STALL_IDENTITY").is_some() {
            let mut descendant = Command::new(std::env::current_exe().unwrap());
            descendant
                .args([
                    "--exact",
                    "runtime::bounded_identity_tests::identity_descendant_fixture",
                    "--nocapture",
                ])
                .env("DEVMAP_TEST_IDENTITY_DESCENDANT", "1");
            let child = descendant.spawn().unwrap();
            if let Some(path) = std::env::var_os("DEVMAP_TEST_IDENTITY_PID_FILE") {
                fs::write(path, child.id().to_string()).unwrap();
            }
        }
    }
    #[test]
    fn identity_descendant_fixture() {
        if std::env::var_os("DEVMAP_TEST_IDENTITY_DESCENDANT").is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
    #[test]
    fn stalled_identity_children_are_bounded_reaped_and_do_not_delay_shutdown() {
        let started = Instant::now();
        let fixture = tempfile::tempdir().unwrap();
        let rt = reactor().unwrap();
        rt.block_on(async {
            let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
            let mut jobs = tokio::task::JoinSet::new();
            let mut admitted = 0;
            for index in 0..32 {
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                admitted += 1;
                let marker = fixture.path().join(format!("child-{index}"));
                jobs.spawn(async move {
                    let _permit = permit;
                    let mut command =
                        tokio::process::Command::new(std::env::current_exe().unwrap());
                    command
                        .args([
                            "--exact",
                            "runtime::bounded_identity_tests::identity_stall_fixture",
                            "--nocapture",
                        ])
                        .env("DEVMAP_TEST_STALL_IDENTITY", "1")
                        .env("DEVMAP_TEST_IDENTITY_PID_FILE", &marker)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .kill_on_drop(true);
                    platform::prepare_identity(&mut command);
                    let child = command.spawn().unwrap();
                    assert!(!marker.exists(), "child ran before job assignment");
                    assert!(
                        identity_child_output(child, Duration::from_secs(1))
                            .await
                            .is_err()
                    );
                });
            }
            assert_eq!(admitted, 4);
            while let Some(result) = jobs.join_next().await {
                result.unwrap();
            }
            assert_eq!(permits.available_permits(), 4);
            let markers: Vec<_> = fs::read_dir(fixture.path()).unwrap().collect();
            assert_eq!(markers.len(), 4, "all stalled descendants actually started");
            #[cfg(windows)]
            for marker in markers {
                use windows_sys::Win32::{
                    Foundation::{CloseHandle, WAIT_TIMEOUT},
                    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
                };
                let pid: u32 = fs::read_to_string(marker.unwrap().path())
                    .unwrap()
                    .parse()
                    .unwrap();
                unsafe {
                    let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                    if !process.is_null() {
                        let status = WaitForSingleObject(process, 0);
                        CloseHandle(process);
                        assert_ne!(
                            status, WAIT_TIMEOUT,
                            "owned descendant survived job timeout"
                        );
                    }
                }
            }
        });
        drop(rt);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stalled identity work retained runtime shutdown"
        );
    }
}

async fn identity_async(source: &Path) -> io::Result<Identity> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .arg("runtime")
        .arg("--identity")
        .arg("--source")
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    platform::prepare_identity(&mut command);
    let bytes = identity_child_output(command.spawn()?, Duration::from_secs(3)).await?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))
}

/// All source-dependent filesystem and Git work stays in an owned process tree.
/// No blocking runtime task survives a deadline, including inherited stdout stalls.
async fn identity_child_output(
    mut child: tokio::process::Child,
    budget: Duration,
) -> io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut tree = match platform::IdentityTree::attach(&child) {
        Ok(tree) => Some(tree),
        Err(error) => {
            let _ = child.start_kill();
            reap_identity(&mut child).await?;
            return Err(error);
        }
    };
    let result = tokio::time::timeout(budget, async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| invalid("identity stdout missing"))?
            .take(32769)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 32768 {
            return Err(invalid("identity output exceeds limit"));
        }
        #[cfg(unix)]
        drop(tree.take()); // Leader is still unreaped, so PGID cannot be reused.
        let status = child.wait().await?;
        if !status.success() {
            return Err(invalid("identity process failed"));
        }
        Ok(bytes)
    })
    .await;
    if let Some(tree) = tree.as_mut()
        && let Err(error) = tree.terminate_and_wait().await
    {
        IDENTITY_SHUTDOWN_FAILED.store(true, std::sync::atomic::Ordering::SeqCst);
        return Err(error);
    }
    drop(tree.take()); // Kernel-confirmed Windows tree termination before releasing admission.
    if !matches!(result, Ok(Ok(_))) {
        let _ = child.start_kill();
        reap_identity(&mut child).await?;
    }
    result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "identity process deadline"))?
}

async fn reap_identity(child: &mut tokio::process::Child) -> io::Result<()> {
    match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        result => {
            IDENTITY_SHUTDOWN_FAILED.store(true, std::sync::atomic::Ordering::SeqCst);
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err(invalid("identity process reap deadline")),
            }
        }
    }
}

#[derive(Debug)]
pub enum RuntimeCallError {
    Transport(io::Error),
    Busy,
    Domain(protocol::DomainError),
}
impl std::fmt::Display for RuntimeCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => e.fmt(f),
            Self::Busy => f.write_str("runtime application busy"),
            Self::Domain(e) => f.write_str(e.message()),
        }
    }
}
impl std::error::Error for RuntimeCallError {}
impl From<io::Error> for RuntimeCallError {
    fn from(e: io::Error) -> Self {
        Self::Transport(e)
    }
}
/// An immutable, bounded application payload. Reuse across owner replacement;
/// transfer identifiers change, but prepared domain identity and bytes do not.
pub struct PreparedMutation {
    bytes: Vec<u8>,
    family: MutationFamily,
}
#[derive(Clone, Copy)]
enum MutationFamily {
    Route,
    Capture,
    Hook,
}
impl PreparedMutation {
    pub fn new(command: &crate::mutation::MutationCommand) -> Result<Self, RuntimeCallError> {
        use crate::mutation::MutationCommand;
        #[derive(serde::Serialize)]
        struct BorrowedMutation<'a> {
            operation: &'static str,
            command: &'a MutationCommand,
        }
        let family = match command {
            MutationCommand::SetRoute { .. } => MutationFamily::Route,
            MutationCommand::CaptureHook { .. } => MutationFamily::Hook,
            _ => MutationFamily::Capture,
        };
        Ok(Self {
            bytes: transport::bounded_json(
                &BorrowedMutation {
                    operation: "Mutate",
                    command,
                },
                protocol::MAX_REQUEST,
            )?,
            family,
        })
    }
    fn accept(
        &self,
        result: protocol::ApplicationResult,
    ) -> Result<crate::mutation::MutationResult, RuntimeCallError> {
        use crate::mutation::MutationResult;
        match result {
            protocol::ApplicationResult::Mutation { mutation }
                if matches!(
                    (&self.family, &mutation),
                    (MutationFamily::Route, MutationResult::Route { .. })
                        | (
                            MutationFamily::Capture,
                            MutationResult::CaptureAccepted { .. }
                        )
                        | (MutationFamily::Hook, MutationResult::HookAccepted { .. })
                ) =>
            {
                Ok(mutation)
            }
            _ => Err(invalid("runtime mutation response kind mismatch").into()),
        }
    }
}
impl RuntimeClient {
    pub fn mutate(
        &mut self,
        command: &crate::mutation::MutationCommand,
    ) -> Result<crate::mutation::MutationResult, RuntimeCallError> {
        self.mutate_prepared(&PreparedMutation::new(command)?)
    }
    pub fn mutate_prepared(
        &mut self,
        prepared: &PreparedMutation,
    ) -> Result<crate::mutation::MutationResult, RuntimeCallError> {
        prepared.accept(self.application_call_bytes(prepared.bytes.clone())?)
    }
    pub fn query(
        &mut self,
        query: &crate::application::ClientQuery,
    ) -> Result<crate::application::ApplicationSnapshot, RuntimeCallError> {
        match self.application_call(&protocol::ApplicationRequest::Query {
            query: query.clone(),
        })? {
            protocol::ApplicationResult::Snapshot { snapshot } => Ok(*snapshot),
            _ => Err(invalid("runtime query response kind mismatch").into()),
        }
    }
    pub fn accept_inventory(
        &mut self,
        prior: &crate::application::ClientQuery,
        tasks: &[crate::dock::ObservedTask],
        complete: bool,
        observed_at: &str,
    ) -> Result<crate::application::ClientQuery, RuntimeCallError> {
        match self.application_call(&protocol::ApplicationRequest::AcceptInventory {
            prior: prior.clone(),
            tasks: tasks.to_vec(),
            complete,
            observed_at: observed_at.to_owned(),
        })? {
            protocol::ApplicationResult::Inventory { query } => Ok(query),
            _ => Err(invalid("runtime inventory response kind mismatch").into()),
        }
    }
    fn application_call(
        &mut self,
        request: &protocol::ApplicationRequest,
    ) -> Result<protocol::ApplicationResult, RuntimeCallError> {
        let bytes = transport::bounded_json(request, protocol::MAX_REQUEST)?;
        self.application_call_bytes(bytes)
    }
    fn application_call_bytes(
        &mut self,
        bytes: Vec<u8>,
    ) -> Result<protocol::ApplicationResult, RuntimeCallError> {
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| invalid("runtime request counter exhausted"))?;
        self.reactor.block_on(transport::exchange(
            &mut self.stream,
            &self.hello,
            &self.welcome,
            self.next_request,
            bytes,
        ))
    }
}

#[cfg(test)]
mod prepared_mutation_tests {
    use super::*;
    use crate::mutation::{MutationCommand, MutationResult};

    fn command() -> MutationCommand {
        MutationCommand::SetRoute {
            input: crate::route_plan::PlanInput {
                delivery: Default::default(),
                request_id: "fixed".into(),
                route_id: None,
                expected_revision: 0,
                worktree_id: "worktree".into(),
                goal: "goal".into(),
                target_ref: None,
                milestones: vec![],
                source: "user".into(),
                abandoned: false,
            },
        }
    }

    #[test]
    fn opaque_preparation_retains_exact_bounded_bytes_and_rejects_wrong_result_family() {
        let mut command = command();
        let prepared = PreparedMutation::new(&command).unwrap();
        let expected = serde_json::to_vec(&protocol::ApplicationRequest::Mutate {
            command: Box::new(command.clone()),
        })
        .unwrap();
        assert_eq!(prepared.bytes, expected);
        let MutationCommand::SetRoute { input } = &mut command else {
            unreachable!()
        };
        input.goal = "changed after preparation".into();
        assert_eq!(prepared.bytes, expected);
        assert!(matches!(
            prepared.accept(protocol::ApplicationResult::Mutation {
                mutation: MutationResult::HookAccepted { sha256: vec![] },
            }),
            Err(RuntimeCallError::Transport(_))
        ));
        input.goal = "x".repeat(protocol::MAX_REQUEST + 1);
        assert!(matches!(
            PreparedMutation::new(&command),
            Err(RuntimeCallError::Transport(_))
        ));

        for (family, mutation) in [
            (
                MutationFamily::Capture,
                MutationResult::CaptureAccepted {
                    sha256: "hash".into(),
                },
            ),
            (
                MutationFamily::Hook,
                MutationResult::HookAccepted {
                    sha256: vec!["hash".into()],
                },
            ),
        ] {
            let expected = mutation.clone();
            let prepared = PreparedMutation {
                bytes: vec![],
                family,
            };
            assert_eq!(
                prepared
                    .accept(protocol::ApplicationResult::Mutation { mutation })
                    .unwrap(),
                expected
            );
            let wrong = if matches!(family, MutationFamily::Hook) {
                MutationResult::CaptureAccepted {
                    sha256: "hash".into(),
                }
            } else {
                MutationResult::HookAccepted { sha256: vec![] }
            };
            assert!(matches!(
                prepared.accept(protocol::ApplicationResult::Mutation { mutation: wrong }),
                Err(RuntimeCallError::Transport(_))
            ));
        }
    }

    #[test]
    fn structured_domain_conversion_keeps_null_plan_revision_and_exact_display() {
        let original = crate::error::DevMapError::RoutePlanConflict {
            revision: 3,
            current_plan: None,
        };
        let message = original.to_string();
        let error = RuntimeCallError::Domain(original.into());
        assert_eq!(error.to_string(), message);
        let RuntimeCallError::Domain(error) = error else {
            unreachable!()
        };
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"code":"revision_conflict", "message":message, "current_revision":3, "current_plan":null})
        );
        let decoded: protocol::DomainError = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.message(), message);
        let ordinary: protocol::DomainError =
            crate::error::DevMapError::Store("original".into()).into();
        assert_eq!(
            serde_json::to_value(&ordinary).unwrap(),
            serde_json::json!({"code":"domain","message":"repository store: original"})
        );
    }
}
