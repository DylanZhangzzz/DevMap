use devmap::runtime::protocol::{Hello, HelloReply, VERSION, Welcome};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    _temp: tempfile::TempDir,
    exe: PathBuf,
    repo: PathBuf,
    owner: Option<Child>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = self.owner.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn git(path: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().into()
}
impl Fixture {
    fn new() -> Self {
        Self::with_moving_anchor(false)
    }
    fn with_moving_anchor(moving: bool) -> Self {
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
        git(
            &repo,
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
        let source = if moving {
            let linked = temp.path().join("linked");
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "moving-owner",
                    linked.to_str().unwrap(),
                ],
            );
            seed_move_history(&repo, &linked, &temp.path().join("backup"));
            linked
        } else {
            repo.clone()
        };
        let owner = Command::new(&exe)
            .args(["runtime", "--owner", "--source"])
            .arg(&source)
            .current_dir(temp.path())
            .args([
                "--instance",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--idle-seconds",
                "60",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            _temp: temp,
            exe,
            repo,
            owner: Some(owner),
        }
    }
    fn welcome(&self) -> Welcome {
        let out = Command::new(&self.exe)
            .args(["runtime", "--identity", "--source"])
            .arg(&self.repo)
            .output()
            .unwrap();
        assert!(out.status.success());
        let identity: Value = serde_json::from_slice(&out.stdout).unwrap();
        Welcome {
            protocol: VERSION,
            repository: identity["repository"].as_str().unwrap().into(),
            build: format!("{:x}", Sha256::digest(fs::read(&self.exe).unwrap())),
            owner_instance: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            owner_pid: self.owner.as_ref().unwrap().id(),
            client_instance: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }
}
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;
async fn connect(f: &Fixture, w: &Welcome) -> Stream {
    connect_source(f, w, &f.repo).await
}
async fn connect_source(f: &Fixture, w: &Welcome, source: &Path) -> Stream {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut s = loop {
        #[cfg(windows)]
        let result = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(format!(r"\\.\pipe\devmap-{}", w.repository));
        #[cfg(unix)]
        let result = {
            let user = format!(
                "{:x}",
                Sha256::digest(unsafe { libc::geteuid() }.to_string().as_bytes())
            );
            let path = fs::canonicalize("/tmp")
                .unwrap()
                .join(format!("devmap-runtime-{}", &user[..16]))
                .join(&w.repository[..32])
                .join("ipc");
            tokio::net::UnixStream::connect(path).await
        };
        match result {
            Ok(s) => break s,
            Err(e) => {
                assert!(std::time::Instant::now() < deadline, "owner startup: {e}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    send(
        &mut s,
        &Hello {
            protocol: VERSION,
            repository: w.repository.clone(),
            build: format!("{:x}", Sha256::digest(fs::read(&f.exe).unwrap())),
            source: fs::canonicalize(source).unwrap(),
            git_dir: fs::canonicalize(git(source, &["rev-parse", "--absolute-git-dir"])).unwrap(),
            client_instance: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
    )
    .await;
    let reply: HelloReply = serde_json::from_value(receive(&mut s).await).unwrap();
    match reply {
        HelloReply::Accepted { welcome } => {
            assert_eq!(welcome.owner_instance, w.owner_instance);
            assert_eq!(welcome.owner_pid, w.owner_pid);
        }
        other => panic!("{other:?}"),
    }
    s
}
async fn send(s: &mut Stream, value: &impl serde::Serialize) {
    let bytes = serde_json::to_vec(value).unwrap();
    assert!(bytes.len() <= 16384);
    s.write_u32(bytes.len() as u32).await.unwrap();
    s.write_all(&bytes).await.unwrap();
}
async fn receive(s: &mut Stream) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        let n = s.read_u32().await.unwrap() as usize;
        assert!(n <= 16384);
        let mut bytes = vec![0; n];
        s.read_exact(&mut bytes).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    })
    .await
    .unwrap()
}
fn empty_query() -> Value {
    json!({"tasks":[], "inventory_observed_at":null, "complete":false, "previous_heads":[]})
}
#[test]
fn real_owner_query_uses_multipart_and_does_not_create_database() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        let bytes = serde_json::to_vec(&json!({"operation":"Query", "query":empty_query()})).unwrap();
        send(&mut s, &json!({"operation":"Begin", "protocol":VERSION, "repository":w.repository, "client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "request_id":1, "owner_instance":w.owner_instance, "total":bytes.len(), "digest":format!("{:x}",Sha256::digest(&bytes))})).await;
        let ready = receive(&mut s).await; assert_eq!(ready["status"], "Ready");
    });
    assert!(!f.repo.join(".git/devmap/devmap.db").exists());
}

async fn begin(s: &mut Stream, w: &Welcome, request_id: u64, bytes: &[u8]) -> Value {
    let tag = json!({"request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":format!("{:x}",Sha256::digest(bytes))});
    send(s, &json!({"operation":"Begin", "protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":tag["digest"]})).await;
    assert_eq!(receive(s).await["status"], "Ready");
    tag
}
async fn call(s: &mut Stream, w: &Welcome, id: u64, request: Value) -> Value {
    let bytes = serde_json::to_vec(&request).unwrap();
    let tag = begin(s, w, id, &bytes).await;
    for (index, bytes) in bytes.chunks(2048).enumerate() {
        send(
            s,
            &json!({"transfer":tag,"offset":index*2048,"bytes":bytes}),
        )
        .await;
    }
    let reply = receive(s).await;
    assert_eq!(reply["status"], "Result", "{reply}");
    let result_tag = &reply["transfer"];
    assert_eq!(result_tag["request_id"], id);
    assert_eq!(result_tag["owner_instance"], w.owner_instance);
    let total = result_tag["total"].as_u64().unwrap() as usize;
    assert!(total <= devmap::runtime::protocol::MAX_RESULT);
    let mut result = vec![];
    while result.len() < total {
        let chunk = receive(s).await;
        assert_eq!(&chunk["transfer"], result_tag);
        assert_eq!(chunk["offset"], result.len());
        result.extend(serde_json::from_value::<Vec<u8>>(chunk["bytes"].clone()).unwrap());
    }
    assert_eq!(result.len(), total);
    assert_eq!(
        result_tag["digest"],
        format!("{:x}", Sha256::digest(&result))
    );
    serde_json::from_slice(&result).unwrap()
}
#[test]
fn complete_query_and_large_inventory_round_trip_keep_original_timestamp() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        let result = call(&mut s, &w, 1, json!({"operation":"Query","query":empty_query()})).await;
        assert_eq!(result["result"], "Snapshot"); assert_eq!(result["snapshot"]["model"]["schema_version"], "devmap/dock/4", "{result}");
        assert!(!f.repo.join(".git/devmap/devmap.db").exists());
        let stamp = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap();
        let tasks: Vec<_> = (0..100).map(|i| json!({"working_directory":null,"subagents":null,"lifecycle":"present","session_id":format!("task-{i}"),"display_title":"multi-part inventory title".repeat(5),"host":"codex","host_status":"running","workspace_path":f.repo.to_string_lossy(),"status":"working","updated_at":stamp})).collect();
        let request = json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":tasks,"complete":true,"observed_at":stamp});
        assert!(serde_json::to_vec(&request).unwrap().len() > 16384);
        let result = call(&mut s, &w, 2, request).await;
        assert_eq!(result["result"], "Inventory", "{result}");
        assert_eq!(result["query"]["inventory_observed_at"], stamp);
        assert_eq!(result["query"]["tasks"].as_array().unwrap().len(),100);
        let projection = call(&mut s, &w, 3, json!({"operation":"Query","query":result["query"]})).await;
        assert_eq!(projection["result"],"Snapshot");
    });
}
#[test]
fn partial_upload_reserves_capacity_and_disconnect_does_not_harm_peer() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut a = connect(&f,&w).await; let mut b = connect(&f,&w).await; let mut c = connect(&f,&w).await;
        let bytes = serde_json::to_vec(&json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":[],"complete":true,"observed_at":"2026-09-08T00:00:00Z"})).unwrap();
        begin(&mut a,&w,1,&bytes).await; begin(&mut b,&w,1,&bytes).await;
        send(&mut c,&json!({"operation":"Begin","protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_id":1,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":format!("{:x}",Sha256::digest(&bytes))})).await;
        assert_eq!(receive(&mut c).await["status"],"Busy");
        drop(a); drop(b);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = call(&mut c,&w,2,json!({"operation":"Query","query":empty_query()})).await;
        assert_eq!(result["result"],"Snapshot");
    });
    assert!(
        !f.repo.join(".git/devmap").exists(),
        "partial commands must not write legacy or SQL"
    );
}
#[test]
fn malformed_transfer_offsets_totals_and_digests_never_dispatch() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        for mode in ["offset","total","digest","content"] {
            let mut s = connect(&f,&w).await;
            let bytes = serde_json::to_vec(&json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":[],"complete":true,"observed_at":"2026-09-08T00:00:00Z"})).unwrap();
            let mut tag = begin(&mut s,&w,1,&bytes).await;
            if mode=="total" { tag["total"]=json!(bytes.len()+1); }
            if mode=="digest" { tag["digest"]=json!("0".repeat(64)); }
            let mut content = bytes.clone(); if mode=="content" { content[0]=b' '; }
            send(&mut s,&json!({"transfer":tag,"offset":if mode=="offset" {1}else{0},"bytes":content})).await;
            let rejected = tokio::time::timeout(Duration::from_secs(5),s.read_u32()).await.unwrap();
            assert!(rejected.is_err(),"malformed {mode} accepted");
        }
    });
    assert!(!f.repo.join(".git/devmap").exists());
}

#[test]
fn oversized_begin_is_rejected_before_upload_or_domain_write() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        send(&mut s, &json!({"operation":"Begin","protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_id":1,"owner_instance":w.owner_instance,"total":devmap::runtime::protocol::MAX_REQUEST+1,"digest":"0".repeat(64)})).await;
        assert!(tokio::time::timeout(Duration::from_secs(5),s.read_u32()).await.unwrap().is_err());
    });
    assert!(!f.repo.join(".git/devmap").exists());
}

use devmap::{
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStore},
    store::migration,
};
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
fn workspace(path: &Path) -> SourceWorkspace {
    SourceGitInspector::open(path).unwrap().workspace().unwrap()
}
fn capture(w: &SourceWorkspace, session: &str) {
    let now = OffsetDateTime::now_utc();
    let record = JournalStore::open(w, session)
        .unwrap()
        .append(
            EventEnvelope::new(
                EVENT_SCHEMA_VERSION,
                format!("{session}-event"),
                EventType::SessionStarted,
                1,
                now.format(&Rfc3339).unwrap(),
                HostIdentity::new("test", "1").unwrap(),
                ActorIdentity::new("actor", None).unwrap(),
                SessionContext::new(
                    session,
                    None,
                    w.root.to_string_lossy(),
                    Some(w.root.to_string_lossy().into_owned()),
                    w.branch.clone(),
                    Some(w.head.clone()),
                )
                .unwrap(),
                serde_json::json!({"activity":"session_started"}),
            )
            .unwrap(),
        )
        .unwrap();
    PresenceStore::open(w)
        .unwrap()
        .observe(PresenceSignal::AcceptedRecords(&[record]), now)
        .unwrap();
}
type SqlRows = BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>;
fn sql_rows(w: &SourceWorkspace) -> SqlRows {
    let c = rusqlite::Connection::open_with_flags(
        w.git_common_dir.join("devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    c.execute_batch("BEGIN").unwrap();
    let mut output = BTreeMap::new();
    for table in [
        "store_meta",
        "worktree_registry",
        "journal_sessions",
        "journal_records",
        "journal_heads",
        "presence_records",
        "presence_projection",
        "route_records",
        "binding_records",
        "binding_watermarks",
        "migration_sources",
        "route_origin_links",
        "binding_origin_links",
        "binding_origin_cursors",
    ] {
        let mut statement = c
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        output.insert(table.into(), rows);
    }
    output
}

fn backup_tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if metadata.is_dir() {
                output.insert(relative, None);
                visit(root, &path, output);
            } else {
                output.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn seed_move_history(main: &Path, linked: &Path, backup: &Path) {
    let main = workspace(main);
    let linked = workspace(linked);
    capture(&linked, "moving-owner-session");
    migration::freeze(&main, backup, OffsetDateTime::now_utc()).unwrap();
    migration::import_shadow(&main, backup).unwrap();
    migration::activate(&main, backup).unwrap();
}

fn owned_process_move(foreign: bool) {
    let mut f = Fixture::with_moving_anchor(true);
    let w = f.welcome();
    let old_path = f._temp.path().join("linked");
    let moved_path = f._temp.path().join("moved");
    let old = workspace(&old_path);
    let main = workspace(&f.repo);
    let old_id = devmap::worktrees::WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|r| r.root == old.root)
        .unwrap()
        .worktree_id;
    let backup = f._temp.path().join("backup");
    let legacy_path = old
        .git_dir
        .join("devmap/sessions/moving-owner-session/events.ndjson");
    let legacy = fs::read(&legacy_path).unwrap();
    let sql = sql_rows(&main);
    let frozen = backup_tree(&backup);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        // The first actual Query (not just owner --source) establishes A as app anchor.
        let mut stale = connect_source(&f, &w, &old_path).await;
        let warm = call(
            &mut stale,
            &w,
            1,
            json!({"operation":"Query","query":empty_query()}),
        )
        .await;
        assert_eq!(warm["result"], "Snapshot", "{warm}");
        assert_eq!(warm["snapshot"]["model"]["current_worktree_id"], old_id);
        assert_eq!(sql_rows(&main), sql);
        git(
            &f.repo,
            &[
                "worktree",
                "move",
                old_path.to_str().unwrap(),
                moved_path.to_str().unwrap(),
            ],
        );
        if foreign {
            fs::create_dir(&old_path).unwrap();
            git(&old_path, &["init", "--quiet"]);
            git(
                &old_path,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "foreign",
                ],
            );
        } else {
            git(
                &f.repo,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "owner-occupant",
                    old_path.to_str().unwrap(),
                ],
            );
        }
        let moved = workspace(&moved_path);
        assert_eq!(moved.git_dir, old.git_dir);
        assert_eq!(moved.head, old.head);
        assert_eq!(moved.branch, old.branch);
        let occupant = workspace(&old_path);
        assert_ne!(occupant.git_dir, old.git_dir);
        let foreign_bytes = foreign.then(|| backup_tree(&occupant.git_dir));
        // Direct connection to the original endpoint cannot auto-start a new owner.
        let mut fresh = connect_source(&f, &w, &moved_path).await;
        let result = call(
            &mut fresh,
            &w,
            1,
            json!({"operation":"Query","query":empty_query()}),
        )
        .await;
        assert_eq!(sql_rows(&main), sql);
        assert_eq!(backup_tree(&backup), frozen);
        assert_eq!(fs::read(&legacy_path).unwrap(), legacy);
        assert_eq!(result["result"], "Snapshot", "{result}");
        assert_eq!(
            result["snapshot"]["store_generation"],
            warm["snapshot"]["store_generation"]
        );
        assert!(
            result["snapshot"]["git_cycle"].as_u64().unwrap()
                > warm["snapshot"]["git_cycle"].as_u64().unwrap()
        );
        let model = &result["snapshot"]["model"];
        assert_eq!(model["current_worktree_id"], old_id);
        let lane = model["lanes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["worktree_id"] == old_id)
            .unwrap();
        assert_eq!(
            lane["workspace_path"],
            moved.root.to_string_lossy().as_ref()
        );
        assert!(
            lane["chats"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["session_id"] == "moving-owner-session")
        );
        for lane in model["lanes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["worktree_id"] != old_id)
        {
            assert!(
                !lane["chats"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["session_id"] == "moving-owner-session")
            );
        }
        // This stream retains A's original authenticated Hello; do not construct A2 identity.
        let rejected = call(
            &mut stale,
            &w,
            2,
            json!({"operation":"Query","query":empty_query()}),
        )
        .await;
        assert_eq!(rejected["result"], "Error", "{rejected}");
        assert!(
            rejected
                .to_string()
                .contains("authenticated source changed"),
            "{rejected}"
        );
        let mut main_stream = connect_source(&f, &w, &f.repo).await;
        let main_result = call(
            &mut main_stream,
            &w,
            1,
            json!({"operation":"Query","query":empty_query()}),
        )
        .await;
        assert_eq!(main_result["result"], "Snapshot", "{main_result}");
        assert_eq!(sql_rows(&main), sql);
        assert_eq!(backup_tree(&backup), frozen);
        assert_eq!(fs::read(&legacy_path).unwrap(), legacy);
        assert_eq!(workspace(&moved_path).head, moved.head);
        assert_eq!(workspace(&moved_path).branch, moved.branch);
        if let Some(bytes) = foreign_bytes {
            assert_eq!(backup_tree(&occupant.git_dir), bytes);
        }
    });
    drop(rt);
    assert!(f.owner.as_mut().unwrap().try_wait().unwrap().is_none());
    assert_eq!(f.owner.as_ref().unwrap().id(), w.owner_pid);
    // Fixture Drop kills and waits only the exact retained Child.
}
#[test]
fn same_owned_owner_serves_moved_source_after_same_repo_reoccupation() {
    owned_process_move(false);
}
#[test]
fn same_owned_owner_serves_moved_source_after_foreign_reoccupation() {
    owned_process_move(true);
}
