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
    instance: String,
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
    fn workspace(&self) -> devmap::git::SourceWorkspace {
        devmap::git::SourceGitInspector::open(&self.repo)
            .unwrap()
            .workspace()
            .unwrap()
    }
    fn restart_after_head_move(&mut self) {
        let mut owner = self.owner.take().unwrap();
        owner.kill().unwrap();
        owner.wait().unwrap();
        let before = git(&self.repo, &["rev-parse", "HEAD"]);
        git(&self.repo, &["commit", "--allow-empty", "-qm", "advance"]);
        assert_ne!(before, git(&self.repo, &["rev-parse", "HEAD"]));
        self.instance = format!(
            "{:032x}",
            u128::from_str_radix(&self.instance, 16).unwrap() + 1
        );
        self.owner = Some(
            Command::new(&self.exe)
                .args(["runtime", "--owner", "--source"])
                .arg(&self.repo)
                .args(["--instance", &self.instance, "--idle-seconds", "60"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
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
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
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
        let workspace = devmap::git::SourceGitInspector::open(&repo)
            .unwrap()
            .workspace()
            .unwrap();
        devmap::store::migration::ensure(&workspace, &temp.path().join("frozen")).unwrap();
        let owner = Command::new(&exe)
            .args(["runtime", "--owner", "--source"])
            .arg(&repo)
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
            instance: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
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
            owner_instance: self.instance.clone(),
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
            source: fs::canonicalize(&f.repo).unwrap(),
            git_dir: fs::canonicalize(git(&f.repo, &["rev-parse", "--absolute-git-dir"])).unwrap(),
            client_instance: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
    )
    .await;
    let reply: HelloReply = serde_json::from_value(receive(&mut s).await).unwrap();
    match reply {
        HelloReply::Accepted { welcome } => {
            assert_eq!(welcome.owner_instance, w.owner_instance);
            assert_eq!(welcome.owner_pid, w.owner_pid);
            assert_eq!(welcome.protocol, w.protocol);
            assert_eq!(welcome.repository, w.repository);
            assert_eq!(welcome.build, w.build);
            assert_eq!(welcome.client_instance, w.client_instance);
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

async fn begin(s: &mut Stream, w: &Welcome, request_id: u64, bytes: &[u8]) -> Value {
    let tag = json!({"request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":format!("{:x}",Sha256::digest(bytes))});
    send(s, &json!({"operation":"Begin", "protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":tag["digest"]})).await;
    assert_eq!(receive(s).await["status"], "Ready");
    tag
}
async fn call(s: &mut Stream, w: &Welcome, id: u64, request: Value) -> Value {
    let bytes = serde_json::to_vec(&request).unwrap();
    call_bytes(s, w, id, &bytes).await
}
async fn upload(s: &mut Stream, w: &Welcome, id: u64, bytes: &[u8]) {
    let tag = begin(s, w, id, bytes).await;
    for (index, bytes) in bytes.chunks(2048).enumerate() {
        send(
            s,
            &json!({"transfer":tag,"offset":index*2048,"bytes":bytes}),
        )
        .await;
    }
}
async fn call_bytes(s: &mut Stream, w: &Welcome, id: u64, bytes: &[u8]) -> Value {
    upload(s, w, id, bytes).await;
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
fn real_owner_dispatches_typed_mutation() {
    let f = Fixture::new();
    let w = f.welcome();
    let workspace = devmap::git::SourceGitInspector::open(&f.repo)
        .unwrap()
        .workspace()
        .unwrap();
    let worktree_id = devmap::worktrees::WorktreeScanner::scan(&workspace).unwrap()[0]
        .worktree_id
        .clone();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        let query=json!({"operation":"Query","query":{"tasks":[],"inventory_observed_at":null,"complete":false,"previous_heads":[]}});
        let warm=call(&mut s,&w,1,query.clone()).await;
        assert_eq!(warm["result"],"Snapshot","{warm}");
        assert!(!warm["snapshot"]["model"].to_string().contains("unique-mutation-cache-goal"));
        let response = call(&mut s, &w, 2, json!({"operation":"Mutate","command":{"kind":"set_route","input":{"request_id":"request","route_id":null,"expected_revision":0,"worktree_id":worktree_id,"goal":"unique-mutation-cache-goal","target_ref":null,"source":"user"}}})).await;
        assert_eq!(response["result"], "Mutation", "{response}");
        assert_eq!(response["mutation"]["kind"], "route");
        assert_eq!(response["mutation"]["plan"]["revision"], 1);
        let refreshed=call(&mut s,&w,3,query).await;
        assert_eq!(refreshed["result"],"Snapshot","{refreshed}");
        assert!(refreshed["snapshot"]["model"].to_string().contains("unique-mutation-cache-goal"));
        assert_eq!(refreshed["snapshot"]["store_generation"].as_u64().unwrap(),warm["snapshot"]["store_generation"].as_u64().unwrap()+1);
    });
}

use devmap::runtime::protocol::{ApplicationRequest, ApplicationResult, DomainError};

#[test]
fn closed_mutation_envelope_and_structured_conflict_roundtrip() {
    let command = json!({"kind":"set_route","input":{"request_id":"request","route_id":null,"expected_revision":0,"worktree_id":"worktree","goal":"goal","target_ref":null,"source":"user"}});
    let request = json!({"operation":"Mutate","command":command});
    let decoded: ApplicationRequest =
        serde_json::from_value(request.clone()).expect("reviewed mutation operation must exist");
    assert_eq!(
        serde_json::to_value(decoded).unwrap()["command"]["kind"],
        "set_route"
    );
    let mut unknown = request;
    unknown["source_path"] = json!("untrusted");
    assert!(serde_json::from_value::<ApplicationRequest>(unknown).is_err());
    let conflict = json!({"code":"revision_conflict","message":"original conflict message","current_revision":7,"current_plan":null});
    let decoded: DomainError =
        serde_json::from_value(conflict.clone()).expect("structured conflict must survive wire");
    assert_eq!(serde_json::to_value(decoded).unwrap(), conflict);
    let mut missing_plan = conflict;
    missing_plan.as_object_mut().unwrap().remove("current_plan");
    assert!(serde_json::from_value::<DomainError>(missing_plan).is_err());
    assert!(
        serde_json::from_value::<DomainError>(json!({"code":"arbitrary","message":"no"})).is_err()
    );
    let result = json!({"result":"Mutation","result_unused":false});
    assert!(serde_json::from_value::<ApplicationResult>(result).is_err());
}

use devmap::{
    capture::{AgentDecisionInput, EvidenceInput, RequirementTraceInput},
    mutation::{CommonCaptureIdentity, MutationCommand, MutationResult},
    route_plan::PlanInput,
};

#[derive(Debug, PartialEq, Eq)]
struct SqlState {
    generation: u64,
    records: Vec<devmap::journal::JournalRecord>,
    // Every authoritative table is observed in one pinned read transaction.
    tables: Vec<Vec<String>>,
}
fn read_state(f: &Fixture) -> SqlState {
    let mut db = rusqlite::Connection::open_with_flags(
        f.repo.join(".git/devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    db.busy_timeout(Duration::from_secs(2)).unwrap();
    let tx = db.transaction().unwrap();
    let generation = tx
        .query_row("SELECT generation FROM store_meta", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
        .try_into()
        .unwrap();
    let queries = [
        "SELECT json_array(session_id,sequence,event_id,record_json,byte_length) FROM journal_records ORDER BY session_id,sequence",
        "SELECT json_array(session_id,record_count,last_sha256,byte_length) FROM journal_heads ORDER BY session_id",
        "SELECT json_array(session_id,record_json) FROM presence_records ORDER BY session_id",
        "SELECT json_array(session_id,covered_sequence,covered_sha256,baseline_source) FROM presence_projection ORDER BY session_id",
        "SELECT json_array(route_id,revision,request_id,input_json,plan_json) FROM route_records ORDER BY route_id,revision",
        "SELECT json_array(session_id,worktree_id,incarnation,origin_path) FROM journal_sessions ORDER BY session_id",
        "SELECT json_array(worktree_id,incarnation,git_dir,workspace_path,retired_at) FROM worktree_registry ORDER BY worktree_id,incarnation",
        "SELECT json_array(route_id,revision,worktree_id,incarnation,qualification) FROM route_origin_links ORDER BY route_id,revision",
        "SELECT json_array(observation_id,destination_worktree_id,destination_incarnation,destination_qualification,source_worktree_id,source_incarnation,source_qualification) FROM binding_origin_links ORDER BY observation_id",
        "SELECT json_array(source_scope,observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id) FROM binding_origin_cursors ORDER BY source_scope",
        "SELECT json_array(observation_id,host,task_id,observed_at,record_json) FROM binding_records ORDER BY observation_id",
        "SELECT json_array(source_scope,observed_at,record_json) FROM binding_watermarks ORDER BY source_scope",
        "SELECT json_array(source_path,source_hash,record_count,outcome,record_json) FROM migration_sources ORDER BY source_path",
        "SELECT json_array(singleton,schema_version,repository_id,common_dir,generation,backend_state) FROM store_meta ORDER BY singleton",
    ];
    let tables: Vec<Vec<String>> = queries
        .iter()
        .map(|sql| {
            tx.prepare(sql)
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
        .collect();
    let records = tx
        .prepare("SELECT record_json FROM journal_records ORDER BY session_id,sequence")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect();
    tx.commit().unwrap();
    SqlState {
        generation,
        records,
        tables,
    }
}
fn verify_integrity(f: &Fixture) {
    let db = rusqlite::Connection::open_with_flags(
        f.repo.join(".git/devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(
        db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    let sessions: Vec<String> = db
        .prepare("SELECT session_id FROM journal_sessions")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for session in sessions {
        devmap::journal::JournalStore::open(&f.workspace(), &session)
            .unwrap()
            .replay()
            .unwrap();
    }
}
fn command_bytes(command: &MutationCommand) -> Vec<u8> {
    serde_json::to_vec(&json!({"operation":"Mutate","command":command})).unwrap()
}
fn common(f: &Fixture, id: &str) -> CommonCaptureIdentity {
    CommonCaptureIdentity::prepare(
        &f.workspace(),
        format!("session-{id}"),
        "actor".into(),
        None,
        None,
        Some(id.into()),
        None,
        time::OffsetDateTime::parse(
            "2026-01-01T00:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap(),
    )
    .unwrap()
}
fn captures(f: &Fixture) -> Vec<(MutationCommand, usize, bool)> {
    let w = f.workspace();
    let now = time::OffsetDateTime::parse(
        "2026-01-01T00:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let hook = |event: &str, body| MutationCommand::CaptureHook {
        prepared: devmap::hook::PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            event,
            body,
            &w,
            now,
        )
        .unwrap(),
    };
    vec![
        (
            MutationCommand::RecordRequirement {
                common: common(f, "requirement"),
                input: RequirementTraceInput {
                    source_kind: "user".into(),
                    source_locator: None,
                    quoted_text: "retain history".into(),
                },
                raw_transcript_opt_in: false,
            },
            1,
            false,
        ),
        (
            MutationCommand::RecordDecision {
                common: common(f, "decision"),
                input: AgentDecisionInput {
                    decision: "retain".into(),
                    basis: vec!["requirement".into()],
                    alternatives: vec!["discard".into()],
                    rationale: "history".into(),
                    scope: "local".into(),
                    authority: "user".into(),
                    revisit_trigger: "new instruction".into(),
                },
            },
            1,
            false,
        ),
        (
            MutationCommand::RecordEvidence {
                common: common(f, "evidence"),
                input: EvidenceInput {
                    kind: "test".into(),
                    target: format!("commit:{}", w.head),
                    command: None,
                    outcome: "passed".into(),
                },
            },
            1,
            false,
        ),
        (
            hook(
                "PostToolUse",
                json!({"session_id":"native","hook_event_name":"PostToolUse","event_id":"native-write","tool_name":"Write"}),
            ),
            2,
            true,
        ),
        (
            hook(
                "SessionStart",
                json!({"session_id":"anonymous","hook_event_name":"SessionStart"}),
            ),
            1,
            false,
        ),
        (
            hook("PostToolUse", json!({"event_id":"native-gap"})),
            1,
            true,
        ),
    ]
}
fn event_identity(command: &MutationCommand) -> (&str, Option<&str>) {
    match command {
        MutationCommand::RecordRequirement { common, .. }
        | MutationCommand::RecordDecision { common, .. }
        | MutationCommand::RecordEvidence { common, .. } => {
            (&common.event_id, common.head.as_deref())
        }
        MutationCommand::CaptureHook { prepared } => {
            (&prepared.identity.event_id, Some(&prepared.identity.head))
        }
        _ => unreachable!(),
    }
}
fn expected_hashes(state: &SqlState, command: &MutationCommand, count: usize) -> Vec<String> {
    let (id, head) = event_identity(command);
    (0..count)
        .map(|index| {
            let expected = if index == 0 {
                id.to_owned()
            } else {
                format!("{id}-{index}")
            };
            let record = state
                .records
                .iter()
                .find(|r| r.event.event_id() == expected)
                .expect("intended committed event");
            assert_eq!(record.event.context().head(), head);
            record.sha256.clone()
        })
        .collect()
}
fn assert_atomic_capture_receipt(state: &SqlState, command: &MutationCommand, count: usize) {
    let hashes = expected_hashes(state, command, count);
    let (id, head) = event_identity(command);
    let record = state
        .records
        .iter()
        .find(|r| r.event.event_id() == id)
        .unwrap();
    let session = record.event.context().session_id();
    if count == 2 {
        assert_eq!(
            *record.event.event_type(),
            devmap::events::EventType::ToolCompleted
        );
        let gap = state
            .records
            .iter()
            .find(|r| r.event.event_id() == format!("{id}-1"))
            .unwrap();
        assert_eq!(
            *gap.event.event_type(),
            devmap::events::EventType::CaptureGap
        );
    }
    let row = |table: usize| {
        state.tables[table]
            .iter()
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .find(|row| row[0] == session)
            .expect("atomic capture companion row missing")
    };
    let journal_head = row(1);
    assert_eq!(journal_head[1], count);
    assert_eq!(journal_head[2], hashes.last().unwrap().as_str());
    let projection = row(3);
    assert_eq!(projection[1], count);
    assert_eq!(projection[2], hashes.last().unwrap().as_str());
    assert_eq!(projection[3], "capture");
    let presence: Value = serde_json::from_str(row(2)[1].as_str().unwrap()).unwrap();
    assert_eq!(presence["head"], head.unwrap());
    let received = match command {
        MutationCommand::RecordRequirement { common, .. }
        | MutationCommand::RecordDecision { common, .. }
        | MutationCommand::RecordEvidence { common, .. } => &common.received_at,
        MutationCommand::CaptureHook { prepared } => &prepared.identity.received_at,
        _ => unreachable!(),
    };
    let format = &time::format_description::well_known::Rfc3339;
    let expected_lease = (time::OffsetDateTime::parse(received, format).unwrap()
        + time::Duration::seconds(devmap::presence::DEFAULT_LEASE_SECONDS))
    .format(format)
    .unwrap();
    assert_eq!(
        presence["lease_expires_at"], expected_lease,
        "receipt-time lease was not frozen"
    );
}
async fn committed_capture(
    f: &Fixture,
    before: &SqlState,
    command: &MutationCommand,
    count: usize,
) -> SqlState {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let (id, _) = event_identity(command);
    loop {
        let state = read_state(f); // Opens a fresh read transaction on every observation.
        if state.records.iter().any(|r| r.event.event_id() == id) {
            assert_eq!(state.records.len(), before.records.len() + count);
            assert_eq!(state.generation, before.generation + 1);
            expected_hashes(&state, command, count);
            assert_atomic_capture_receipt(&state, command, count);
            return state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "intended commit never became visible"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn mutated_payload(command: &MutationCommand) -> Option<MutationCommand> {
    let mut changed = command.clone();
    match &mut changed {
        MutationCommand::RecordRequirement { input, .. } => {
            input.quoted_text = "different requirement".into()
        }
        MutationCommand::RecordDecision { input, .. } => {
            input.decision = "different decision".into()
        }
        MutationCommand::RecordEvidence { input, .. } => input.outcome = "failed".into(),
        MutationCommand::CaptureHook { prepared }
            if prepared.event == "PostToolUse" && prepared.body.contains_key("session_id") =>
        {
            prepared.body.insert("tool_name".into(), json!("Edit"));
        }
        _ => return None,
    }
    Some(changed)
}

#[test]
fn committed_capture_response_abandoned_before_consumption_replays_exactly_after_owner_and_head_change()
 {
    let mut f = Fixture::new();
    let commands = captures(&f);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for (command, count, is_gap) in commands {
                let bytes = command_bytes(&command);
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let before = read_state(&f);
                let w = f.welcome();
                let mut first = connect(&f, &w).await;
                upload(&mut first, &w, 1, &bytes).await;
                // Deliberately NO result read. SQL independently establishes commit.
                let committed = committed_capture(&f, &before, &command, count).await;
                let gaps = |s: &SqlState| {
                    s.records
                        .iter()
                        .filter(|r| *r.event.event_type() == devmap::events::EventType::CaptureGap)
                        .count()
                };
                assert_eq!(gaps(&committed), gaps(&before) + usize::from(is_gap));
                let hashes = expected_hashes(&committed, &command, count);
                drop(first);
                f.restart_after_head_move();
                let replacement = f.welcome();
                assert_ne!(replacement.owner_instance, w.owner_instance);
                let mut retry = connect(&f, &replacement).await;
                assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
                let response = call_bytes(&mut retry, &replacement, 1, &bytes).await;
                let expected = if matches!(command, MutationCommand::CaptureHook { .. }) {
                    MutationResult::HookAccepted { sha256: hashes }
                } else {
                    MutationResult::CaptureAccepted {
                        sha256: hashes[0].clone(),
                    }
                };
                assert_eq!(response, json!({"result":"Mutation","mutation":expected}));
                assert_eq!(read_state(&f), committed, "retry changed committed state");
                if let Some(changed) = mutated_payload(&command) {
                    let result =
                        call_bytes(&mut retry, &replacement, 2, &command_bytes(&changed)).await;
                    assert_eq!(result["result"], "Error");
                    assert_eq!(result["error"]["code"], "domain");
                    assert_eq!(read_state(&f), committed);
                }
                verify_integrity(&f);
                assert_eq!(read_state(&f), committed);
            }
        });
}

#[test]
fn separately_prepared_anonymous_invocations_each_commit_once() {
    let f = Fixture::new();
    let w = f.welcome();
    let prepare = || {
        devmap::hook::PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            "SessionStart",
            json!({"session_id":"anonymous-distinct","hook_event_name":"SessionStart"}),
            &f.workspace(),
            time::OffsetDateTime::now_utc(),
        )
        .unwrap()
    };
    let first = prepare();
    let second = prepare();
    assert_ne!(first.identity.event_id, second.identity.event_id);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut stream = connect(&f, &w).await;
            let mut id = 0;
            let baseline = read_state(&f);
            for (index, prepared) in [first, second].into_iter().enumerate() {
                let command = MutationCommand::CaptureHook { prepared };
                let bytes = command_bytes(&command);
                id += 1;
                let result = call_bytes(&mut stream, &w, id, &bytes).await;
                assert_eq!(result["result"], "Mutation", "{result}");
                let committed = read_state(&f);
                assert_eq!(committed.records.len(), baseline.records.len() + index + 1);
                assert_eq!(committed.generation, baseline.generation + index as u64 + 1);
                id += 1;
                assert_eq!(call_bytes(&mut stream, &w, id, &bytes).await, result);
                assert_eq!(read_state(&f), committed);
            }
            verify_integrity(&f);
        });
}

fn route_input(f: &Fixture, request: &str) -> PlanInput {
    PlanInput {
        delivery: Default::default(),
        request_id: request.into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: devmap::worktrees::WorktreeScanner::scan(&f.workspace()).unwrap()[0]
            .worktree_id
            .clone(),
        goal: request.into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    }
}
async fn route_call(s: &mut Stream, w: &Welcome, id: u64, input: PlanInput) -> Value {
    call_bytes(
        s,
        w,
        id,
        &command_bytes(&MutationCommand::SetRoute { input }),
    )
    .await
}
#[test]
fn abandoned_route_response_keeps_structured_cas_and_old_request_result_after_later_update() {
    let mut f = Fixture::new();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let w = f.welcome();
            let mut first = connect(&f, &w).await;
            let initial = route_call(&mut first, &w, 1, route_input(&f, "initial")).await;
            assert_eq!(initial["result"], "Mutation", "{initial}");
            let plan = &initial["mutation"]["plan"];
            let mut winner = route_input(&f, "winner");
            winner.route_id = Some(plan["route_id"].as_str().unwrap().into());
            winner.expected_revision = 1;
            let bytes = command_bytes(&MutationCommand::SetRoute {
                input: winner.clone(),
            });
            let digest = format!("{:x}", Sha256::digest(&bytes));
            let before = read_state(&f);
            upload(&mut first, &w, 2, &bytes).await;
            // No result consumption; independently read the original stored plan.
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let committed = loop {
                let state = read_state(&f);
                if state.tables[4]
                    .iter()
                    .any(|row| serde_json::from_str::<Value>(row).unwrap()[2] == "winner")
                {
                    break state;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "route commit was not observed"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            assert_eq!(committed.generation, before.generation + 1);
            assert_eq!(committed.tables[4].len(), before.tables[4].len() + 1);
            let stored: Value = committed.tables[4]
                .iter()
                .map(|row| serde_json::from_str::<Value>(row).unwrap())
                .find(|row| row[2] == "winner")
                .unwrap();
            let stored_plan: Value = serde_json::from_str(stored[4].as_str().unwrap()).unwrap();
            let expected =
                json!({"result":"Mutation","mutation":{"kind":"route","plan":stored_plan}});
            drop(first);
            f.restart_after_head_move();
            let replacement = f.welcome();
            assert_ne!(w.owner_instance, replacement.owner_instance);
            let mut stream = connect(&f, &replacement).await;
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
            assert_eq!(
                call_bytes(&mut stream, &replacement, 1, &bytes).await,
                expected
            );
            assert_eq!(read_state(&f), committed);
            let mut loser = winner.clone();
            loser.request_id = "loser".into();
            loser.goal = "loser".into();
            let response = route_call(&mut stream, &replacement, 2, loser).await;
            let result: ApplicationResult = serde_json::from_value(response).unwrap();
            let ApplicationResult::Error { error } = result else {
                panic!("expected conflict")
            };
            let devmap::runtime::RuntimeCallError::Domain(DomainError::RevisionConflict {
                current_revision,
                current_plan,
                message,
            }) = devmap::runtime::RuntimeCallError::Domain(error)
            else {
                panic!("typed conflict missing")
            };
            assert_eq!(current_revision, 2);
            assert_eq!(serde_json::to_value(current_plan).unwrap(), stored_plan);
            assert_eq!(
                message,
                "route plan revision conflict: current revision is 2"
            );
            assert_eq!(read_state(&f), committed);
            let mut latest = winner.clone();
            latest.expected_revision = 2;
            latest.request_id = "latest".into();
            latest.goal = "latest".into();
            let response = route_call(&mut stream, &replacement, 3, latest).await;
            assert_eq!(response["mutation"]["plan"]["revision"], 3, "{response}");
            let updated = read_state(&f);
            assert_eq!(updated.generation, committed.generation + 1);
            assert_eq!(
                call_bytes(&mut stream, &replacement, 4, &bytes).await,
                expected
            );
            assert_eq!(read_state(&f), updated, "old request overwrote latest plan");
            let mut changed = winner;
            changed.goal = "changed same request id".into();
            let response = route_call(&mut stream, &replacement, 5, changed).await;
            assert_eq!(response["error"]["code"], "domain", "{response}");
            assert_eq!(read_state(&f), updated);
            verify_integrity(&f);
        });
}

#[test]
fn incomplete_digest_mismatch_and_unknown_mutation_uploads_have_zero_effect() {
    let f = Fixture::new();
    let w = f.welcome();
    let before = read_state(&f);
    let command = &captures(&f)[0].0;
    let bytes = command_bytes(command);
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut partial=connect(&f,&w).await;
        let tag=begin(&mut partial,&w,1,&bytes).await;
        send(&mut partial,&json!({"transfer":tag,"offset":0,"bytes":&bytes[..bytes.len()-1]})).await;
        drop(partial);
        let mut malformed=connect(&f,&w).await;
        let tag=begin(&mut malformed,&w,1,&bytes).await;
        let mut altered=bytes.clone();altered[0]=b' ';
        send(&mut malformed,&json!({"transfer":tag,"offset":0,"bytes":altered})).await;
        assert!(tokio::time::timeout(Duration::from_secs(5),malformed.read_u32()).await.unwrap().is_err());
        let mut unknown=connect(&f,&w).await;
        let mut request:Value=serde_json::from_slice(&bytes).unwrap();
        request["command"]["unexpected"]=json!(true);
        let response=call(&mut unknown,&w,1,request).await;
        assert_eq!(response["result"],"Error","{response}");
        assert_eq!(response["error"]["code"],"domain");
        assert_eq!(read_state(&f),before);
        // A serially accepted query is a barrier after prior accepted commands.
        let response=call(&mut unknown,&w,2,json!({"operation":"Query","query":{"tasks":[],"inventory_observed_at":null,"complete":false,"previous_heads":[]}})).await;
        assert_eq!(response["result"],"Snapshot","{response}");
        assert_eq!(read_state(&f),before);
    });
}
