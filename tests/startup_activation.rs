mod support;
use devmap::git::SourceGitInspector;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"startup-fixture","version":"1"}}})
}
fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Fixture {
    repo: tempfile::TempDir,
    _files: tempfile::TempDir,
    exe: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let repo = support::committed_repo();
        let files = tempfile::tempdir().unwrap();
        let exe = files.path().join(if cfg!(windows) {
            "devmap.exe"
        } else {
            "devmap"
        });
        fs::copy(env!("CARGO_BIN_EXE_devmap"), &exe).unwrap();
        let state = files.path().join("user-state");
        fs::create_dir(&state).unwrap();
        Self {
            repo,
            _files: files,
            exe,
            state,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.exe);
        // Standard per-process state roots keep future default backups inside this fixture.
        command
            .env("LOCALAPPDATA", &self.state)
            .env("XDG_STATE_HOME", &self.state);
        command
    }
    fn owner(&self) -> OwnedChild {
        let nonce = "dddddddddddddddddddddddddddddddd";
        let owner = OwnedChild(
            self.command()
                .args(["runtime", "--owner", "--source"])
                .arg(self.repo.path())
                .args(["--instance", nonce, "--idle-seconds", "60"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        authenticate_owner(self, &self.exe, nonce, owner.0.id());
        owner
    }
    fn database(&self) -> PathBuf {
        self.repo.path().join(".git/devmap/devmap.db")
    }
    fn assert_never_activated(&self) {
        assert!(!self.database().exists(), "read created main database");
        assert!(
            !self
                .repo
                .path()
                .join(".git/devmap/activation-intent.json")
                .exists()
        );
        assert!(
            !self
                .repo
                .path()
                .join(".git/devmap/backend-transition.lock")
                .exists()
        );
        assert!(
            !self
                .repo
                .path()
                .join(".git/devmap/backend-transition")
                .exists(),
            "read or invalid input created a backend transition directory"
        );
        assert!(
            fs::read_dir(&self.state).unwrap().next().is_none(),
            "read created a backup/state artifact"
        );
    }
    fn active_sql(&self) -> rusqlite::Connection {
        assert!(
            self.database().is_file(),
            "first explicit shared write did not activate SQLite"
        );
        let db = rusqlite::Connection::open_with_flags(
            self.database(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let backend: String = db
            .query_row("SELECT backend_state FROM store_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(backend, "active");
        let provenance: String = db
            .query_row(
                "SELECT record_json FROM migration_sources WHERE source_path='@activation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let provenance: Value = serde_json::from_str(&provenance).unwrap();
        let snapshot = PathBuf::from(provenance["snapshot_path"].as_str().unwrap());
        assert!(
            snapshot.starts_with(fs::canonicalize(&self.state).unwrap()),
            "default backup escaped injected durable state: {}",
            snapshot.display()
        );
        assert!(snapshot.join("manifest.json").is_file());
        assert!(!snapshot.starts_with(fs::canonicalize(self.repo.path().join(".git")).unwrap()));
        db
    }
    fn hook(&self) -> std::process::Output {
        let mut child = OwnedChild(
            self.command()
                .args(["hook", "handle", "--source"])
                .arg(self.repo.path())
                .args(["--host", "claude", "--event", "SessionStart"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        serde_json::to_writer(
            child.0.stdin.take().unwrap(),
            &json!({"session_id":"startup-hook","event_id":"startup-hook-event"}),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                use std::io::Read;
                let mut stdout = vec![];
                let mut stderr = vec![];
                child
                    .0
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_end(&mut stdout)
                    .unwrap();
                child
                    .0
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_end(&mut stderr)
                    .unwrap();
                return std::process::Output {
                    status,
                    stdout,
                    stderr,
                };
            }
            assert!(Instant::now() < deadline, "owned hook deadline");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
fn authenticate_owner(f: &Fixture, exe: &Path, nonce: &str, pid: u32) {
    use devmap::runtime::protocol::{Hello, HelloReply, VERSION};
    let identity = Command::new(exe)
        .args(["runtime", "--identity", "--source"])
        .arg(f.repo.path())
        .output()
        .unwrap();
    assert!(identity.status.success());
    let identity: Value = serde_json::from_slice(&identity.stdout).unwrap();
    let repository = identity["repository"].as_str().unwrap();
    let hello = Hello {
        protocol: VERSION,
        repository: repository.into(),
        build: format!("{:x}", Sha256::digest(fs::read(exe).unwrap())),
        source: fs::canonicalize(f.repo.path()).unwrap(),
        git_dir: fs::canonicalize(f.repo.path().join(".git")).unwrap(),
        client_instance: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                #[cfg(windows)]
                let result = tokio::net::windows::named_pipe::ClientOptions::new()
                    .open(format!(r"\\.\pipe\devmap-{repository}"));
                #[cfg(unix)]
                let result = {
                    let user = format!(
                        "{:x}",
                        Sha256::digest(unsafe { libc::geteuid() }.to_string().as_bytes())
                    );
                    let path = fs::canonicalize("/tmp")
                        .unwrap()
                        .join(format!("devmap-runtime-{}", &user[..16]))
                        .join(&repository[..32])
                        .join("ipc");
                    tokio::net::UnixStream::connect(path).await
                };
                match result {
                    Ok(s) => break s,
                    Err(e) => {
                        assert!(Instant::now() < deadline, "owner startup {e}");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            };
            let encoded = serde_json::to_vec(&hello).unwrap();
            stream.write_u32(encoded.len() as u32).await.unwrap();
            stream.write_all(&encoded).await.unwrap();
            let reply = tokio::time::timeout(Duration::from_secs(5), async {
                let len = stream.read_u32().await.unwrap() as usize;
                assert!(len <= 16384);
                let mut bytes = vec![0; len];
                stream.read_exact(&mut bytes).await.unwrap();
                serde_json::from_slice::<HelloReply>(&bytes).unwrap()
            })
            .await
            .unwrap();
            let HelloReply::Accepted { welcome } = reply else {
                panic!("owner rejected test hello")
            };
            assert_eq!(welcome.owner_pid, pid);
            assert_eq!(welcome.owner_instance, nonce);
            assert_eq!(welcome.build, hello.build);
            assert_eq!(welcome.repository, repository);
            assert_eq!(welcome.protocol, VERSION);
            assert_eq!(welcome.client_instance, hello.client_instance);
        });
}
struct PublicMcp {
    _child: OwnedChild,
    stdin: ChildStdin,
    replies: mpsc::Receiver<Value>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Drop for PublicMcp {
    fn drop(&mut self) {
        let _ = self._child.0.kill();
        let _ = self._child.0.wait();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}
impl PublicMcp {
    fn new(f: &Fixture) -> Self {
        let mut child = OwnedChild(
            f.command()
                .args(["mcp", "--source"])
                .arg(f.repo.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let stdin = child.0.stdin.take().unwrap();
        let stdout = child.0.stdout.take().unwrap();
        let (tx, replies) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        let mut mcp = Self {
            _child: child,
            stdin,
            replies,
            reader: Some(reader),
        };
        assert!(mcp.request(&initialize())["result"].is_object());
        mcp
    }
    fn request(&mut self, value: &Value) -> Value {
        serde_json::to_writer(&mut self.stdin, value).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
        self.replies
            .recv_timeout(Duration::from_secs(60))
            .expect("MCP reply deadline")
    }
}

#[test]
fn fresh_query_is_read_only_then_first_route_activates_sqlite() {
    let f = Fixture::new();
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    let query = mcp.request(&call(2, "devmap_read_map", json!({})));
    assert_ne!(query["result"]["isError"], true, "{query}");
    f.assert_never_activated();
    let workspace = SourceGitInspector::open(f.repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let worktree = devmap::worktrees::WorktreeScanner::scan(&workspace).unwrap()[0]
        .worktree_id
        .clone();
    let route = json!({"request_id":"startup-route","route_id":null,"expected_revision":0,"worktree_id":worktree,"goal":"fresh SQL","target_ref":null,"source":"user"});
    let response = mcp.request(&call(3, "devmap_set_route_plan", route.clone()));
    assert_ne!(response["result"]["isError"], true, "{response}");
    let db = f.active_sql();
    let saved: String = db
        .query_row("SELECT plan_json FROM route_records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap(),
        response["result"]["structuredContent"]
    );
    assert_eq!(
        db.query_row("SELECT generation FROM store_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        mcp.request(&call(4, "devmap_set_route_plan", route))["result"]["structuredContent"],
        response["result"]["structuredContent"]
    );
    assert_eq!(
        db.query_row("SELECT generation FROM store_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!f.repo.path().join(".git/devmap/route-plans.jsonl").exists());
}

#[test]
fn first_shared_semantic_write_activates_and_retains_sha() {
    let f = Fixture::new();
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    let response=mcp.request(&call(2,"devmap_record_requirement",json!({"session_id":"startup-semantic","agent_id":"actor","event_id":"startup-semantic-event","source_kind":"user","quoted_text":"one database"})));
    assert_ne!(response["result"]["isError"], true, "{response}");
    let db = f.active_sql();
    let sha: String = db
        .query_row(
            "SELECT json_extract(record_json,'$.sha256') FROM journal_records",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        response["result"]["structuredContent"],
        json!({"sha256":sha})
    );
    assert!(!f.repo.path().join(".git/devmap/sessions").exists());
}

#[test]
fn first_shared_hook_activates_and_retains_empty_object_receipt() {
    let f = Fixture::new();
    let _owner = f.owner();
    let response = f.hook();
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    assert_eq!(response.stdout, b"{}\n");
    let db = f.active_sql();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM journal_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!f.repo.path().join(".git/devmap/sessions").exists());
}

#[test]
fn first_explicit_inventory_activates_but_invalid_inventory_does_not() {
    let f = Fixture::new();
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    let invalid = mcp.request(&call(
        2,
        "devmap_read_map",
        json!({"codex_tasks":[{"id":"broken"}]}),
    ));
    assert_eq!(invalid["result"]["isError"], true);
    f.assert_never_activated();
    let result=mcp.request(&call(3,"devmap_read_map",json!({"codex_tasks":[{"id":"01a081a1-751a-7473-aa3b-91995c128f7e","title":"fresh","cwd":f.repo.path().to_string_lossy(),"status":"active","lifecycle":"present","hostId":"local","kind":"codex","updatedAt":time::OffsetDateTime::now_utc().unix_timestamp()}],"codex_tasks_complete":true})));
    assert_ne!(result["result"]["isError"], true, "{result}");
    let db = f.active_sql();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM binding_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/task-bindings.jsonl")
            .exists()
    );
}

#[test]
fn existing_legacy_shared_write_preserves_legacy_without_automatic_cutover() {
    let f = Fixture::new();
    let mut direct = devmap::mcp::McpRuntime::open(f.repo.path()).unwrap();
    direct.handle(&initialize());
    let first=direct.handle(&call(2,"devmap_record_requirement",json!({"session_id":"old","agent_id":"actor","event_id":"old-event","source_kind":"user","quoted_text":"old history"}))).unwrap();
    assert_ne!(first["result"]["isError"], true, "{first}");
    let old = f.repo.path().join(".git/devmap/sessions/old/events.ndjson");
    let original = fs::read(&old).unwrap();
    assert!(!f.database().exists());
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    let next=mcp.request(&call(3,"devmap_record_requirement",json!({"session_id":"new","agent_id":"actor","event_id":"new-event","source_kind":"user","quoted_text":"preserve old writer compatibility"})));
    assert_ne!(next["result"]["isError"], true, "{next}");
    assert_eq!(fs::read(old).unwrap(), original);
    assert!(!f.database().exists());
    assert!(
        f.repo
            .path()
            .join(".git/devmap/sessions/new/events.ndjson")
            .is_file()
    );
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/activation-intent.json")
            .exists()
    );
    assert!(fs::read_dir(&f.state).unwrap().next().is_none());
}

#[test]
fn domain_invalid_semantic_input_leaves_fresh_repository_for_next_valid_write() {
    let f = Fixture::new();
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    // Structurally valid MCP arguments; only the domain evidence-target validator
    // rejects this. This must happen before JournalStore::open or auto setup.
    let invalid = mcp.request(&call(
        2,
        "devmap_record_evidence",
        json!({
            "session_id":"invalid-first", "agent_id":"actor", "event_id":"invalid-event",
            "kind":"test", "target":"commit:not-a-digest", "outcome":"passed"
        }),
    ));
    assert_eq!(invalid["result"]["isError"], true, "{invalid}");
    assert!(
        invalid["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid evidence target"),
        "wrong error: {invalid}"
    );
    f.assert_never_activated();
    assert!(
        !f.repo.path().join(".git/devmap/sessions").exists(),
        "domain-invalid capture created legacy session directories and spoiled fresh-empty eligibility"
    );

    let workspace = SourceGitInspector::open(f.repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let valid = mcp.request(&call(
        3,
        "devmap_record_evidence",
        json!({
            "session_id":"invalid-first", "agent_id":"actor", "event_id":"valid-event",
            "kind":"test", "target":format!("commit:{}", workspace.head), "outcome":"passed"
        }),
    ));
    assert_ne!(valid["result"]["isError"], true, "{valid}");
    let db = f.active_sql();
    let rows = db.prepare("SELECT event_id,json_extract(record_json,'$.sha256') FROM journal_records ORDER BY sequence")
        .unwrap().query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))
        .unwrap().map(Result::unwrap).collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "valid-event");
    assert_eq!(
        valid["result"]["structuredContent"],
        json!({"sha256":rows[0].1})
    );
    assert_eq!(
        db.query_row("SELECT generation FROM store_meta", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!f.repo.path().join(".git/devmap/sessions").exists());
}

#[test]
fn four_simultaneous_first_writes_share_one_activation_and_retain_every_receipt() {
    let f = Fixture::new();
    let _owner = f.owner();
    let barrier = std::sync::Barrier::new(4);
    let receipts = std::thread::scope(|scope| {
        (0..4).map(|index| {
            let f = &f;
            let barrier = &barrier;
            scope.spawn(move || {
                let mut mcp = PublicMcp::new(f);
                barrier.wait();
                let event_id = format!("first-event-{index}");
                let response = mcp.request(&call(2, "devmap_record_requirement", json!({
                    "session_id":format!("first-session-{index}"), "agent_id":format!("actor-{index}"),
                    "event_id":event_id, "source_kind":"user", "quoted_text":format!("first write {index}")
                })));
                assert_ne!(response["result"]["isError"], true, "{response}");
                (event_id, response["result"]["structuredContent"]["sha256"].as_str().unwrap().to_owned())
            })
        }).collect::<Vec<_>>().into_iter().map(|thread| thread.join().unwrap()).collect::<std::collections::BTreeMap<_,_>>()
    });
    let db = f.active_sql();
    let persisted = db
        .prepare("SELECT event_id,json_extract(record_json,'$.sha256') FROM journal_records")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(persisted, receipts);
    assert_eq!(
        db.query_row("SELECT generation FROM store_meta", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM migration_sources WHERE source_path='@activation'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(!f.repo.path().join(".git/devmap/sessions").exists());
}

#[cfg(windows)]
fn fixture_acl_command(path: &Path, original: &str, script: &str) -> Result<String, String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("DEVMAP_TEST_ACL_PATH", path)
        .env("DEVMAP_TEST_ACL_ORIGINAL", original)
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = OwnedChild(command.spawn().map_err(|e| e.to_string())?);
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err("fixture ACL helper deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .map_err(|e| e.to_string())?;
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("fixture ACL helper failed: {stderr}"));
    }
    Ok(stdout.trim().to_owned())
}

#[cfg(windows)]
struct FixtureDirectoryDeny {
    path: PathBuf,
    canonical: PathBuf,
    creation_time: u64,
    original: String,
    restored: bool,
}

#[cfg(windows)]
impl FixtureDirectoryDeny {
    fn install(f: &Fixture) -> Self {
        use std::os::windows::fs::MetadataExt;
        let canonical = fs::canonicalize(&f.state).unwrap();
        assert!(canonical.starts_with(fs::canonicalize(f._files.path()).unwrap()));
        let metadata = fs::symlink_metadata(&f.state).unwrap();
        assert!(metadata.is_dir() && metadata.file_attributes() & 0x400 == 0);
        let original = fixture_acl_command(&f.state, "", r#"
$ErrorActionPreference = 'Stop'
$acl = [System.IO.Directory]::GetAccessControl($env:DEVMAP_TEST_ACL_PATH)
[Console]::WriteLine($acl.GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access))
"#).unwrap();
        let guard = Self {
            path: f.state.clone(),
            canonical,
            creation_time: metadata.creation_time(),
            original,
            restored: false,
        };
        // The guard exists before the mutation, so an assertion or helper failure
        // still restores only this fixture's original access descriptor.
        fixture_acl_command(&guard.path, "", r#"
$ErrorActionPreference = 'Stop'
$acl = [System.IO.Directory]::GetAccessControl($env:DEVMAP_TEST_ACL_PATH)
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, [System.Security.AccessControl.FileSystemRights]::CreateDirectories, [System.Security.AccessControl.InheritanceFlags]::None, [System.Security.AccessControl.PropagationFlags]::None, [System.Security.AccessControl.AccessControlType]::Deny)
$acl.AddAccessRule($rule)
[System.IO.Directory]::SetAccessControl($env:DEVMAP_TEST_ACL_PATH, $acl)
"#).unwrap();
        guard
    }

    fn restore(&mut self) -> Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        if self.restored {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_attributes() & 0x400 != 0
            || metadata.creation_time() != self.creation_time
            || fs::canonicalize(&self.path).map_err(|e| e.to_string())? != self.canonical
        {
            return Err("fixture ACL target identity changed; refusing restore elsewhere".into());
        }
        fixture_acl_command(&self.path, &self.original, r#"
$ErrorActionPreference = 'Stop'
$acl = [System.IO.Directory]::GetAccessControl($env:DEVMAP_TEST_ACL_PATH)
$acl.SetSecurityDescriptorSddlForm($env:DEVMAP_TEST_ACL_ORIGINAL, [System.Security.AccessControl.AccessControlSections]::Access)
[System.IO.Directory]::SetAccessControl($env:DEVMAP_TEST_ACL_PATH, $acl)
[Console]::WriteLine(([System.IO.Directory]::GetAccessControl($env:DEVMAP_TEST_ACL_PATH)).GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access))
"#).and_then(|restored| if restored == self.original { Ok(()) } else { Err("fixture ACL restoration differs from original".into()) })?;
        self.restored = true;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for FixtureDirectoryDeny {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!(
                "fixture-only ACL cleanup failed for {}: {error}",
                self.path.display()
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn backup_create_permission_denied_preserves_legacy_receipt_and_later_writes() {
    let f = Fixture::new();
    let mut denied = FixtureDirectoryDeny::install(&f);
    let failure = fs::create_dir(f.state.join("denied-probe")).unwrap_err();
    assert_eq!(
        failure.kind(),
        std::io::ErrorKind::PermissionDenied,
        "fault did not produce PermissionDenied: {failure}"
    );
    let evidence = serde_json::to_vec(&json!({"observed_error_kind":format!("{:?}", failure.kind()),"denied_right":"CreateDirectories","inherited":false})).unwrap();
    let marker = f.state.join("permission-denied-evidence.json");
    fs::write(&marker, &evidence).unwrap();
    let _owner = f.owner();
    let mut mcp = PublicMcp::new(&f);
    let args = json!({"session_id":"backup-denied","agent_id":"actor","event_id":"denied-first","source_kind":"user","quoted_text":"retain legacy on denied backup"});
    let first = mcp.request(&call(2, "devmap_record_requirement", args.clone()));
    assert_ne!(first["result"]["isError"], true, "{first}");
    assert!(!f.database().exists());
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/activation-intent.json")
            .exists()
    );
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/backend-transition/attempt.json")
            .exists()
    );
    let journal = f
        .repo
        .path()
        .join(".git/devmap/sessions/backup-denied/events.ndjson");
    let original = fs::read(&journal).unwrap();
    let verify_record = |line: &[u8], receipt: &Value| {
        let record: Value = serde_json::from_slice(line).unwrap();
        assert_eq!(devmap::canonical::canonical_json(&record).unwrap(), line);
        let unsigned = json!({"sequence":record["sequence"],"event":record["event"],"previous_sha256":record["previous_sha256"]});
        let sha = format!(
            "{:x}",
            Sha256::digest(devmap::canonical::canonical_json(&unsigned).unwrap())
        );
        assert_eq!(record["sha256"], sha);
        assert_eq!(
            receipt["result"]["structuredContent"],
            json!({"sha256":sha})
        );
        record
    };
    let first_lines = original
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(first_lines.len(), 1);
    let first_record = verify_record(first_lines[0], &first);
    assert_eq!(first_record["sequence"], 1);
    assert_eq!(first_record["previous_sha256"], Value::Null);
    assert_eq!(first_record["event"]["event_id"], "denied-first");
    assert_eq!(first_record["event"]["actor"], json!({"agent_id":"actor"}));
    assert_eq!(
        first_record["event"]["context"]["session_id"],
        "backup-denied"
    );
    assert_eq!(
        first_record["event"]["payload"]["requirement_trace"],
        json!({"source":{"kind":"user","locator":null},"approved_quotation":"retain legacy on denied backup"})
    );
    let retry = mcp.request(&call(3, "devmap_record_requirement", args));
    assert_eq!(
        retry["result"]["structuredContent"],
        first["result"]["structuredContent"]
    );
    assert_eq!(fs::read(&journal).unwrap(), original);
    assert_eq!(
        fs::create_dir(f.state.join("still-denied-probe"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    denied.restore().unwrap();

    // Once legacy history exists, restoring backup access must not silently
    // change its authority on the next invocation.
    let second = mcp.request(&call(4, "devmap_record_requirement", json!({"session_id":"backup-denied","agent_id":"actor","event_id":"denied-second","source_kind":"user","quoted_text":"continue retained legacy"})));
    assert_ne!(second["result"]["isError"], true, "{second}");
    let final_bytes = fs::read(&journal).unwrap();
    assert!(final_bytes.starts_with(&original));
    let lines = final_bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    let second_record = verify_record(lines[1], &second);
    assert_eq!(second_record["sequence"], 2);
    assert_eq!(second_record["previous_sha256"], first_record["sha256"]);
    assert_eq!(second_record["event"]["event_id"], "denied-second");
    assert_eq!(second_record["event"]["actor"], json!({"agent_id":"actor"}));
    assert_eq!(
        second_record["event"]["context"]["session_id"],
        "backup-denied"
    );
    assert_eq!(
        second_record["event"]["payload"]["requirement_trace"],
        json!({"source":{"kind":"user","locator":null},"approved_quotation":"continue retained legacy"})
    );
    assert!(!f.database().exists());
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/activation-intent.json")
            .exists()
    );
    assert!(
        !f.repo
            .path()
            .join(".git/devmap/backend-transition/attempt.json")
            .exists()
    );
    assert!(
        f.repo
            .path()
            .join(".git/devmap/backend-transition/lock")
            .is_file()
    );
    assert_eq!(fs::read(&marker).unwrap(), evidence);
    assert_eq!(
        fs::read_dir(&f.state)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![marker.file_name().unwrap().to_os_string()]
    );
}
