//! Same-user, repository-scoped local owner. This slice only exposes diagnostics.
mod owner;
pub mod protocol;
mod transport;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
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
use unix as platform;
#[cfg(windows)]
use windows as platform;
const START_BUDGET: Duration = Duration::from_secs(15);
#[derive(Clone)]
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
    let base = std::env::temp_dir();
    #[cfg(unix)]
    let base = fs::canonicalize("/tmp")?;
    // Reject link/reparse ancestors, including a redirected temporary root.
    for ancestor in base.ancestors() {
        let meta = fs::symlink_metadata(ancestor)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(invalid("runtime ancestor is a reparse point"));
            }
        }
        if meta.file_type().is_symlink() {
            return Err(invalid("runtime ancestor is a symlink"));
        }
    }
    let base = fs::canonicalize(base)?;
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
        let id = identity(source)?;
        let location = location(&id)?;
        let reactor = reactor()?;
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
        loop {
            match connect(expected.as_deref()) {
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
            if expected.is_none() {
                match owner_lock.try_lock_exclusive() {
                    Ok(()) => {
                        FileExt::unlock(&owner_lock)?;
                        let instance = nonce()?;
                        platform::spawn_owner(
                            &std::env::current_exe()?,
                            &hello.source,
                            &instance,
                            idle_seconds,
                        )?;
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
        if args.owner {
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
