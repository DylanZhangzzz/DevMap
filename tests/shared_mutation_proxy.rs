mod support;
use devmap::{git::SourceGitInspector, mcp::McpRuntime};
use serde_json::{Value, json};

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"mutation-fixture","version":"1"}}})
}
fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}

#[test]
fn each_new_public_capture_invocation_observes_fresh_head() {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let frozen = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&workspace, &frozen.path().join("frozen")).unwrap();
    let mut mcp = McpRuntime::open(repo.path()).unwrap();
    mcp.handle(&initialize()).unwrap();
    support::git(repo.path(), ["commit", "--allow-empty", "-m", "advance"]);
    let moved = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert_ne!(workspace.head, moved.head);
    let result=mcp.handle(&call(2,"devmap_record_evidence",json!({"session_id":"fresh","agent_id":"actor","event_id":"fresh-event","kind":"test","target":format!("commit:{}",moved.head),"outcome":"passed"}))).unwrap();
    assert_ne!(result["result"]["isError"], true, "{result}");
    let records = devmap::journal::JournalStore::open(&moved, "fresh")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.context().head(), Some(moved.head.as_str()));
    assert_eq!(
        result["result"]["structuredContent"]["sha256"],
        records[0].sha256
    );
}

#[test]
fn mutation_proxy_is_bound_to_first_source_and_rejects_a_second_repository_before_write() {
    use devmap::{
        mutation::{CommonCaptureIdentity, MutationCommand},
        mutation_proxy::MutationProxy,
        proxy::ProxyMode,
    };
    let first = support::committed_repo();
    let second = support::committed_repo();
    let frozen = tempfile::tempdir().unwrap();
    let a = SourceGitInspector::open(first.path())
        .unwrap()
        .workspace()
        .unwrap();
    let b = SourceGitInspector::open(second.path())
        .unwrap()
        .workspace()
        .unwrap();
    for (index, w) in [&a, &b].into_iter().enumerate() {
        devmap::store::migration::ensure(w, &frozen.path().join(index.to_string())).unwrap();
    }
    let generation = |w: &devmap::git::SourceWorkspace| {
        devmap::store::RepositoryStore::open_existing(w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap()
    };
    let command = |w: &devmap::git::SourceWorkspace| MutationCommand::RecordEvidence {
        common: CommonCaptureIdentity::prepare(
            w,
            "bound-session".into(),
            "actor".into(),
            None,
            None,
            Some("bound-event".into()),
            None,
            time::OffsetDateTime::now_utc(),
        )
        .unwrap(),
        input: devmap::capture::EvidenceInput {
            kind: "test".into(),
            target: format!("commit:{}", w.head),
            command: None,
            outcome: "passed".into(),
        },
    };
    let mut proxy = MutationProxy::new(ProxyMode::Direct);
    proxy.execute(&a, &command(&a)).unwrap();
    let before = (generation(&a), generation(&b));
    let error = proxy.execute(&b, &command(&b)).unwrap_err();
    assert!(matches!(
        error,
        devmap::error::DevMapError::InvalidDomain("mutation proxy source changed")
    ));
    assert_eq!((generation(&a), generation(&b)), before);
    let a_records = devmap::journal::JournalStore::open(&a, "bound-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(a_records.len(), 1);
    let db = rusqlite::Connection::open_with_flags(
        b.git_common_dir.join("devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM journal_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

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

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct PublicFixture {
    repo: tempfile::TempDir,
    _files: tempfile::TempDir,
    exe: PathBuf,
    incompatible: PathBuf,
}
impl PublicFixture {
    fn new() -> Self {
        let repo = support::committed_repo();
        let files = tempfile::tempdir().unwrap();
        let exe = files.path().join(if cfg!(windows) {
            "devmap.exe"
        } else {
            "devmap"
        });
        fs::copy(env!("CARGO_BIN_EXE_devmap"), &exe).unwrap();
        let incompatible = files.path().join(if cfg!(windows) {
            "other-build.exe"
        } else {
            "other-build"
        });
        fs::copy(&exe, &incompatible).unwrap();
        // Harmless executable overlay, making a different authenticated build.
        fs::OpenOptions::new()
            .append(true)
            .open(&incompatible)
            .unwrap()
            .write_all(b"different fixture build")
            .unwrap();
        let workspace = SourceGitInspector::open(repo.path())
            .unwrap()
            .workspace()
            .unwrap();
        devmap::store::migration::ensure(&workspace, &files.path().join("frozen")).unwrap();
        Self {
            repo,
            _files: files,
            exe,
            incompatible,
        }
    }
    fn workspace(&self) -> devmap::git::SourceWorkspace {
        SourceGitInspector::open(self.repo.path())
            .unwrap()
            .workspace()
            .unwrap()
    }
    fn owner(&self, exe: &Path, nonce: &str) -> OwnedChild {
        let child = OwnedChild(
            Command::new(exe)
                .args(["runtime", "--owner", "--source"])
                .arg(self.repo.path())
                .args(["--instance", nonce, "--idle-seconds", "60"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        authenticate_owner(self, exe, nonce, child.0.id());
        child
    }
    fn state(&self) -> Vec<String> {
        let mut c = rusqlite::Connection::open_with_flags(
            self.repo.path().join(".git/devmap/devmap.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let mut rows = vec![
            tx.query_row("SELECT CAST(generation AS TEXT) FROM store_meta", [], |r| {
                r.get(0)
            })
            .unwrap(),
        ];
        for sql in [
            "SELECT record_json FROM journal_records ORDER BY session_id,sequence",
            "SELECT record_json FROM presence_records ORDER BY session_id",
            "SELECT json_array(session_id,covered_sequence,covered_sha256) FROM presence_projection ORDER BY session_id",
            "SELECT json_array(request_id,plan_json) FROM route_records ORDER BY route_id,revision",
        ] {
            rows.extend(
                tx.prepare(sql)
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        tx.commit().unwrap();
        rows
    }
    fn hook(&self, body: &Value, event: &str) -> std::process::Output {
        let mut child = OwnedChild(
            Command::new(&self.exe)
                .args(["hook", "handle", "--source"])
                .arg(self.repo.path())
                .args(["--host", "claude", "--event", event])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        serde_json::to_writer(child.0.stdin.take().unwrap(), body).unwrap();
        let deadline = Instant::now() + Duration::from_secs(45);
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
            assert!(Instant::now() < deadline, "hook child deadline");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
fn authenticate_owner(f: &PublicFixture, exe: &Path, nonce: &str, pid: u32) {
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
    fn new(f: &PublicFixture) -> Self {
        let mut child = OwnedChild(
            Command::new(&f.exe)
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
fn semantic_calls(head: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            "devmap_record_requirement",
            json!({"session_id":"public-requirement","agent_id":"actor","event_id":"public-requirement","source_kind":"user","quoted_text":"retain data"}),
        ),
        (
            "devmap_record_decision",
            json!({"session_id":"public-decision","agent_id":"actor","event_id":"public-decision","decision":"keep","basis":["requirement"],"alternatives":["discard"],"rationale":"history","scope":"local","authority":"user","revisit_trigger":"new input"}),
        ),
        (
            "devmap_record_evidence",
            json!({"session_id":"public-evidence","agent_id":"actor","event_id":"public-evidence","kind":"test","target":format!("commit:{head}"),"outcome":"passed"}),
        ),
    ]
}

#[test]
fn public_mcp_and_hook_require_shared_owner_and_preserve_semantic_sha_route_cas_and_native_retries()
{
    let f = PublicFixture::new();
    let mut mcp = PublicMcp::new(&f);
    let wrong = f.owner(&f.incompatible, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let before = f.state();
    let w = f.workspace();
    let worktree = devmap::worktrees::WorktreeScanner::scan(&w).unwrap()[0]
        .worktree_id
        .clone();
    let route = json!({"request_id":"first-route","route_id":null,"expected_revision":0,"worktree_id":worktree,"goal":"first","target_ref":null,"source":"user"});
    let mut id = 2;
    for (tool, args) in semantic_calls(&w.head)
        .into_iter()
        .chain(std::iter::once(("devmap_set_route_plan", route.clone())))
    {
        let result = mcp.request(&call(id, tool, args));
        id += 1;
        assert_eq!(
            result["result"]["isError"], true,
            "mutation bypassed incompatible owner: {result}"
        );
        assert!(
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("handshake"),
            "{result}"
        );
        assert_eq!(f.state(), before);
    }
    let native = json!({"session_id":"public-native","hook_event_name":"PostToolUse","event_id":"native-stable","occurred_at":"2026-01-01T00:00:00Z","tool_name":"Write"});
    let blocked = f.hook(&native, "PostToolUse");
    assert!(
        !blocked.status.success(),
        "hook bypassed incompatible owner"
    );
    assert_eq!(f.state(), before);
    drop(wrong);
    let owner = f.owner(&f.exe, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    for (tool, args) in semantic_calls(&w.head) {
        let result = mcp.request(&call(id, tool, args.clone()));
        id += 1;
        assert_ne!(result["result"]["isError"], true, "{result}");
        let records = devmap::journal::JournalStore::open(
            &f.workspace(),
            args["session_id"].as_str().unwrap(),
        )
        .unwrap()
        .replay()
        .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            result["result"]["structuredContent"],
            json!({"sha256":records[0].sha256})
        );
    }
    let first = mcp.request(&call(id, "devmap_set_route_plan", route.clone()));
    id += 1;
    assert_ne!(first["result"]["isError"], true, "{first}");
    let first_plan = &first["result"]["structuredContent"];
    let mut update = route.clone();
    update["route_id"] = first_plan["route_id"].clone();
    update["expected_revision"] = json!(1);
    update["request_id"] = json!("next-route");
    update["goal"] = json!("next");
    let second = mcp.request(&call(id, "devmap_set_route_plan", update.clone()));
    id += 1;
    assert_ne!(second["result"]["isError"], true, "{second}");
    update["request_id"] = json!("stale-route");
    let conflict = mcp.request(&call(id, "devmap_set_route_plan", update));
    id += 1;
    assert_eq!(
        conflict["result"]["structuredContent"],
        json!({"error_code":"revision_conflict","current_revision":2,"current_plan":second["result"]["structuredContent"]})
    );
    assert_eq!(
        conflict["result"]["content"][0]["text"],
        "route plan revision conflict: current revision is 2"
    );
    let after = f.state();
    assert_eq!(
        mcp.request(&call(id, "devmap_set_route_plan", route))["result"]["structuredContent"],
        *first_plan
    );
    id += 1;
    assert_eq!(f.state(), after);
    for _ in 0..2 {
        let result = f.hook(&native, "PostToolUse");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"{}\n");
    }
    let native_records = devmap::journal::JournalStore::open(&f.workspace(), "public-native")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(native_records.len(), 2);
    let native_state = f.state();
    assert!(f.hook(&native, "PostToolUse").status.success());
    assert_eq!(f.state(), native_state);
    let anonymous = json!({"session_id":"public-anonymous","hook_event_name":"SessionStart"});
    assert!(f.hook(&anonymous, "SessionStart").status.success());
    assert!(f.hook(&anonymous, "SessionStart").status.success());
    let records = devmap::journal::JournalStore::open(&f.workspace(), "public-anonymous")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0].event.event_id(), records[1].event.event_id());
    drop(owner);
    let _replacement = f.owner(&f.exe, "cccccccccccccccccccccccccccccccc");
    support::git(
        f.repo.path(),
        ["commit", "--allow-empty", "-m", "fresh public invocation"],
    );
    let moved = f.workspace();
    let response=mcp.request(&call(id,"devmap_record_evidence",json!({"session_id":"after-owner-restart","agent_id":"actor","kind":"test","target":format!("commit:{}",moved.head),"outcome":"passed"})));
    assert_ne!(response["result"]["isError"], true, "{response}");
    let records = devmap::journal::JournalStore::open(&moved, "after-owner-restart")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.context().head(), Some(moved.head.as_str()));
    assert_eq!(
        response["result"]["structuredContent"],
        json!({"sha256":records[0].sha256})
    );
}
