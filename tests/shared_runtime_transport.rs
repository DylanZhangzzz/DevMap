use devmap::runtime::protocol::{Hello, VERSION, Welcome};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Fixture {
    temp: tempfile::TempDir,
    exe: PathBuf,
    repo: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join(if cfg!(windows) {
            "devmap.exe"
        } else {
            "devmap"
        });
        fs::copy(env!("CARGO_BIN_EXE_devmap"), &exe).unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        Self { temp, exe, repo }
    }
    fn ping(&self, source: &Path, idle: u64) -> Welcome {
        ping(&self.exe, source, idle)
    }
    fn owner(&self, instance: &str) -> OwnedChild {
        OwnedChild(
            Command::new(&self.exe)
                .args(["runtime", "--source"])
                .arg(&self.repo)
                .args(["--owner", "--instance", instance, "--idle-seconds", "10"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn git(source: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn ping(exe: &Path, source: &Path, idle: u64) -> Welcome {
    let output = Command::new(exe)
        .args(["runtime", "--source"])
        .arg(source)
        .args(["--ping", "--idle-seconds", &idle.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn reactor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;
fn endpoint(repository: &str) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\devmap-{repository}")
    }
    #[cfg(unix)]
    {
        let user = format!(
            "{:x}",
            Sha256::digest(unsafe { libc::geteuid() }.to_string().as_bytes())
        );
        fs::canonicalize("/tmp")
            .unwrap()
            .join(format!("devmap-runtime-{}", &user[..16]))
            .join(&repository[..32])
            .join("ipc")
            .to_str()
            .unwrap()
            .to_owned()
    }
}
async fn raw_connect(repository: &str) -> Stream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        #[cfg(windows)]
        let result =
            tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint(repository));
        #[cfg(unix)]
        let result = tokio::net::UnixStream::connect(endpoint(repository)).await;
        match result {
            Ok(stream) => return stream,
            Err(e) => {
                assert!(Instant::now() < deadline, "connect: {e}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}
async fn send(stream: &mut Stream, value: &impl serde::Serialize) {
    let bytes = serde_json::to_vec(value).unwrap();
    stream.write_u32(bytes.len() as u32).await.unwrap();
    stream.write_all(&bytes).await.unwrap();
}
async fn receive<T: serde::de::DeserializeOwned>(stream: &mut Stream) -> std::io::Result<T> {
    tokio::time::timeout(Duration::from_secs(6), async {
        let length = stream.read_u32().await?;
        assert!(length < 4 * 1024 * 1024);
        let mut bytes = vec![0; length as usize];
        stream.read_exact(&mut bytes).await?;
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    })
    .await
    .map_err(std::io::Error::other)?
}
fn hello(f: &Fixture, welcome: &Welcome) -> Hello {
    Hello {
        protocol: VERSION,
        repository: welcome.repository.clone(),
        build: format!("{:x}", Sha256::digest(fs::read(&f.exe).unwrap())),
        source: fs::canonicalize(&f.repo).unwrap(),
        git_dir: fs::canonicalize(git(&f.repo, &["rev-parse", "--absolute-git-dir"])).unwrap(),
        client_instance: "11111111111111111111111111111111".into(),
    }
}
#[test]
fn hidden_runtime_ping_starts_an_owner_without_database_or_stdio_lease() {
    let f = Fixture::new();
    let mut child = Command::new(&f.exe)
        .args(["runtime", "--source"])
        .arg(&f.repo)
        .args(["--ping", "--idle-seconds", "10"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (finished, received) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut out = Vec::new();
        let mut err = Vec::new();
        stdout.read_to_end(&mut out).unwrap();
        stderr.read_to_end(&mut err).unwrap();
        let _ = finished.send((out, err));
    });
    let status = child.wait().unwrap();
    // Judge pipe closure after the client exits, independent of hashing/startup cost.
    let (out, err) = received
        .recv_timeout(Duration::from_secs(2))
        .expect("background owner retained captured client stdio after client exit");
    reader.join().unwrap();
    assert!(status.success(), "{}", String::from_utf8_lossy(&err));
    let welcome: Welcome = serde_json::from_slice(&out).unwrap();
    assert!(welcome.owner_instance.len() >= 32);
    assert!(!f.repo.join(".git/devmap").exists());
    // Copied executable isolates the detached owner's lifetime from cargo builds.
    std::thread::sleep(Duration::from_secs(12));
}
#[test]
fn four_simultaneous_processes_and_linked_worktree_share_one_owner() {
    let f = Fixture::new();
    git(
        &f.repo,
        &[
            "-c",
            "user.name=Runtime Test",
            "-c",
            "user.email=runtime@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    let linked = f.temp.path().join("linked");
    git(
        &f.repo,
        &["worktree", "add", "--quiet", linked.to_str().unwrap()],
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let barrier = barrier.clone();
            let exe = f.exe.clone();
            let source = if i % 2 == 0 {
                f.repo.clone()
            } else {
                linked.clone()
            };
            std::thread::spawn(move || {
                barrier.wait();
                ping(&exe, &source, 3)
            })
        })
        .collect();
    let welcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    for item in &welcomes {
        assert_eq!(item.owner_instance, welcomes[0].owner_instance);
        assert_eq!(item.owner_pid, welcomes[0].owner_pid);
        assert_eq!(item.repository, welcomes[0].repository);
    }
    assert!(!f.repo.join(".git/devmap").exists());
    std::thread::sleep(Duration::from_secs(5));
}
#[test]
fn wrong_identity_partial_and_oversize_are_bounded_and_other_client_survives() {
    let f = Fixture::new();
    let _owner = f.owner("22222222222222222222222222222222");
    // Wait for the explicitly owned child rather than racing autostart.
    std::thread::sleep(Duration::from_millis(750));
    let welcome = f.ping(&f.repo, 10);
    assert_eq!(welcome.owner_pid, _owner.0.id());
    let valid = hello(&f, &welcome);
    reactor().block_on(async {
        let mut survivor = raw_connect(&welcome.repository).await;
        send(&mut survivor, &valid).await;
        assert!(matches!(
            receive::<devmap::runtime::protocol::HelloReply>(&mut survivor)
                .await
                .unwrap(),
            devmap::runtime::protocol::HelloReply::Accepted { .. }
        ));
        for alteration in 0..4 {
            let mut invalid = valid.clone();
            match alteration {
                0 => invalid.protocol += 1,
                1 => invalid.repository = "wrong-repo".into(),
                2 => invalid.build = "wrong-build".into(),
                _ => invalid.source = f.temp.path().to_path_buf(),
            }
            let mut stream = raw_connect(&welcome.repository).await;
            send(&mut stream, &invalid).await;
            assert_rejected(receive::<devmap::runtime::protocol::HelloReply>(&mut stream).await);
        }
        let mut stream = raw_connect(&welcome.repository).await;
        stream
            .write_u32((devmap::runtime::protocol::MAX_FRAME + 1) as u32)
            .await
            .unwrap();
        assert_rejected(receive::<devmap::runtime::protocol::HelloReply>(&mut stream).await);
        let mut partial = raw_connect(&welcome.repository).await;
        partial.write_u32(100).await.unwrap();
        partial.write_all(b"{").await.unwrap();
        assert_rejected(receive::<devmap::runtime::protocol::HelloReply>(&mut partial).await);
        send(
            &mut survivor,
            &devmap::runtime::protocol::Request::Ping {
                protocol: VERSION,
                repository: welcome.repository.clone(),
                client_instance: valid.client_instance.clone(),
                request_id: 1,
            },
        )
        .await;
        let response: devmap::runtime::protocol::Response = receive(&mut survivor).await.unwrap();
        assert_eq!(response.owner_instance, welcome.owner_instance);
    });
}
#[test]
fn owned_child_termination_allows_new_authenticated_instance() {
    let f = Fixture::new();
    let mut owner = f.owner("33333333333333333333333333333333");
    std::thread::sleep(Duration::from_millis(750));
    let first = f.ping(&f.repo, 10);
    assert_eq!(first.owner_pid, owner.0.id());
    #[cfg(windows)]
    {
        let netstat = Command::new("netstat").arg("-ano").output().unwrap();
        assert!(netstat.status.success());
        let text = String::from_utf8_lossy(&netstat.stdout);
        let pid = owner.0.id().to_string();
        assert!(
            !text.lines().any(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                fields.first() == Some(&"TCP") && fields.last() == Some(&pid.as_str())
            }),
            "diagnostic owner opened a TCP socket"
        );
    }
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let second = f.ping(&f.repo, 2);
    assert_ne!(first.owner_instance, second.owner_instance);
    assert_ne!(first.owner_pid, second.owner_pid);
    assert_eq!(first.repository, second.repository);
    std::thread::sleep(Duration::from_secs(4));
}

#[test]
fn incompatible_build_fails_promptly_without_starting_second_owner() {
    use std::io::Write;
    let f = Fixture::new();
    let owner = f.owner("44444444444444444444444444444444");
    std::thread::sleep(Duration::from_millis(750));
    let initial = f.ping(&f.repo, 10);
    assert_eq!(initial.owner_pid, owner.0.id());
    let other = f
        .temp
        .path()
        .join(if cfg!(windows) { "other.exe" } else { "other" });
    fs::copy(&f.exe, &other).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&other)
        .unwrap()
        .write_all(b"different-build")
        .unwrap();
    let started = Instant::now();
    let output = Command::new(&other)
        .args(["runtime", "--source"])
        .arg(&f.repo)
        .arg("--ping")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "incompatible owner should reject promptly"
    );
    assert_eq!(f.ping(&f.repo, 10).owner_instance, initial.owner_instance);
}

#[test]
fn saturated_owner_rejects_extra_connection_with_finite_backpressure() {
    let f = Fixture::new();
    let owner = f.owner("55555555555555555555555555555555");
    std::thread::sleep(Duration::from_millis(750));
    let welcome = f.ping(&f.repo, 10);
    assert_eq!(welcome.owner_pid, owner.0.id());
    let valid = hello(&f, &welcome);
    reactor().block_on(async {
        let mut held = Vec::new();
        for _ in 0..devmap::runtime::protocol::MAX_CONNECTIONS {
            let mut stream = raw_connect(&welcome.repository).await;
            send(&mut stream, &valid).await;
            assert!(matches!(
                receive::<devmap::runtime::protocol::HelloReply>(&mut stream)
                    .await
                    .unwrap(),
                devmap::runtime::protocol::HelloReply::Accepted { .. }
            ));
            held.push(stream);
        }
        let start = Instant::now();
        let mut extra = raw_connect(&welcome.repository).await;
        let bytes = serde_json::to_vec(&valid).unwrap();
        if extra.write_u32(bytes.len() as u32).await.is_ok()
            && extra.write_all(&bytes).await.is_ok()
        {
            assert!(
                receive::<devmap::runtime::protocol::HelloReply>(&mut extra)
                    .await
                    .is_err()
            );
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "saturated accept must reject rather than retain an unbounded waiter"
        );
    });
}

#[cfg(windows)]
#[test]
fn runtime_artifacts_never_enter_git_administration_even_with_temp_override() {
    let f = Fixture::new();
    let admin = f.repo.join(".git");
    let output = Command::new(&f.exe)
        .args(["runtime", "--source"])
        .arg(&f.repo)
        .args(["--ping", "--idle-seconds", "1"])
        .env("TEMP", &admin)
        .env("TMP", &admin)
        .env("USERPROFILE", &admin)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "runtime must reject Git-admin temporary root"
    );
    assert!(!fs::read_dir(&admin).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("devmap-runtime-")
    }));
}

#[test]
fn idle_owner_exits_finitely_and_different_repository_has_distinct_owner() {
    let f = Fixture::new();
    let mut owner = f.owner("66666666666666666666666666666666");
    std::thread::sleep(Duration::from_millis(750));
    let first = f.ping(&f.repo, 10);
    assert_eq!(first.owner_pid, owner.0.id());
    let other = f.temp.path().join("other-repository");
    fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    let second = f.ping(&other, 1);
    assert_ne!(first.repository, second.repository);
    assert_ne!(first.owner_instance, second.owner_instance);
    assert_ne!(first.owner_pid, second.owner_pid);
    let deadline = Instant::now() + Duration::from_secs(14);
    loop {
        if let Some(status) = owner.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "idle owner did not exit");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn assert_rejected(reply: std::io::Result<devmap::runtime::protocol::HelloReply>) {
    match reply {
        Ok(devmap::runtime::protocol::HelloReply::Rejected { .. }) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
            ) => {}
        other => panic!("expected explicit rejection or EOF, got {other:?}"),
    }
}
#[test]
#[should_panic(expected = "expected explicit rejection")]
fn rejection_oracle_refuses_a_valid_accepted_reply() {
    assert_rejected(Ok(devmap::runtime::protocol::HelloReply::Accepted {
        welcome: Welcome {
            protocol: VERSION,
            repository: "repo".into(),
            build: "build".into(),
            owner_instance: "11111111111111111111111111111111".into(),
            owner_pid: 1,
            client_instance: "22222222222222222222222222222222".into(),
        },
    }));
}

#[cfg(windows)]
#[test]
fn untrusted_runtime_parents_are_rejected_before_creating_artifacts() {
    let f = Fixture::new();
    let parent = f.temp.path().join("unsafe-parent");
    fs::create_dir(&parent).unwrap();
    assert!(
        Command::new("icacls")
            .arg(&parent)
            .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let out = Command::new(&f.exe)
        .args(["runtime", "--source"])
        .arg(&f.repo)
        .arg("--ping")
        .env("TEMP", &parent)
        .env("TMP", &parent)
        .env("USERPROFILE", &parent)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(fs::read_dir(&parent).unwrap().next().is_none());
}

#[cfg(windows)]
#[test]
fn safe_runtime_leaf_below_untrusted_grandparent_is_rejected() {
    let f = Fixture::new();
    let grandparent = f.temp.path().join("unsafe-grandparent");
    let middle = grandparent.join("middle");
    let leaf = middle.join("safe-leaf");
    fs::create_dir_all(&leaf).unwrap();
    let user_sid = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    assert!(user_sid.status.success());
    let user_sid = String::from_utf8(user_sid.stdout).unwrap();
    let sid = user_sid.split(',').nth(1).unwrap().trim().trim_matches('"');
    for path in [&middle, &leaf] {
        assert!(
            Command::new("icacls")
                .arg(path)
                .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(OI)(CI)F")])
                .stdout(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    assert!(
        Command::new("icacls")
            .arg(&grandparent)
            .args(["/grant", "*S-1-1-0:(DC)"])
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(&f.exe)
        .args(["runtime", "--source"])
        .arg(&f.repo)
        .args(["--ping", "--idle-seconds", "1"])
        .env("TEMP", &leaf)
        .env("TMP", &leaf)
        .env("USERPROFILE", &leaf)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "an owner-only leaf must not conceal an ancestor granting untrusted DELETE_CHILD"
    );
    assert!(
        fs::read_dir(&leaf).unwrap().next().is_none(),
        "unsafe ancestor was checked after runtime artifact creation"
    );
}
