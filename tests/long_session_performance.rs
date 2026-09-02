mod support;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use devmap::events::{
    ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
};
use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use serde_json::json;
use support::committed_repo;

const SESSION: &str = "release-long-session";
const PRELOAD_RECORDS: u64 = 1_000;
const SAMPLE_RECORDS: u64 = 64;

// These are broad operational budgets, not machine-specific microbenchmarks. A native hook has
// a five-second ceiling here, while the indexed steady-state journal window allows 750 ms at p95
// and 20 seconds in aggregate. Preloading 1,000 durable records may take up to 60 seconds.
const PRELOAD_LIMIT: Duration = Duration::from_secs(60);
const SAMPLE_TOTAL_LIMIT: Duration = Duration::from_secs(20);
const SAMPLE_P95_LIMIT: Duration = Duration::from_millis(750);
const NATIVE_HOOK_LIMIT: Duration = Duration::from_secs(5);

#[test]
fn release_long_session_stays_within_conservative_native_hook_budgets() {
    if cfg!(debug_assertions) {
        eprintln!("release-only performance gate skipped in the debug test profile");
        return;
    }

    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, SESSION).unwrap();

    let preload_started = Instant::now();
    for sequence in 1..=PRELOAD_RECORDS {
        store.append(event(&workspace, sequence)).unwrap();
    }
    let preload_elapsed = preload_started.elapsed();
    assert!(
        preload_elapsed <= PRELOAD_LIMIT,
        "1,000-record durable preload took {preload_elapsed:?}"
    );

    let sample_started = Instant::now();
    let mut latencies = Vec::with_capacity(SAMPLE_RECORDS as usize);
    for sequence in (PRELOAD_RECORDS + 1)..=(PRELOAD_RECORDS + SAMPLE_RECORDS) {
        let started = Instant::now();
        store.append(event(&workspace, sequence)).unwrap();
        latencies.push(started.elapsed());
    }
    let sample_elapsed = sample_started.elapsed();
    latencies.sort_unstable();
    let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
    let p95 = latencies[p95_index];
    assert!(
        sample_elapsed <= SAMPLE_TOTAL_LIMIT,
        "steady-state append window took {sample_elapsed:?}"
    );
    assert!(
        p95 <= SAMPLE_P95_LIMIT,
        "steady-state append p95 was {p95:?}"
    );

    let native_started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args([
            "hook",
            "handle",
            "--source",
            repository.path().to_str().unwrap(),
            "--host",
            "codex",
            "--event",
            "UserPromptSubmit",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = json!({
        "session_id": SESSION,
        "hook_event_name": "UserPromptSubmit",
        "turn_id": "release-native-probe",
        "prompt": "Content is hashed and not persisted."
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let native_elapsed = native_started.elapsed();
    assert!(
        output.status.success(),
        "native hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        native_elapsed <= NATIVE_HOOK_LIMIT,
        "native hook took {native_elapsed:?} after a long session"
    );
    assert_eq!(
        store.replay().unwrap().len(),
        (PRELOAD_RECORDS + SAMPLE_RECORDS + 1) as usize
    );

    eprintln!(
        "PERF records={} preload_ms={} sample_count={} sample_total_ms={} p95_ms={} native_hook_ms={} thresholds_ms=preload:{},sample_total:{},p95:{},native:{}",
        PRELOAD_RECORDS + SAMPLE_RECORDS + 1,
        preload_elapsed.as_millis(),
        SAMPLE_RECORDS,
        sample_elapsed.as_millis(),
        p95.as_millis(),
        native_elapsed.as_millis(),
        PRELOAD_LIMIT.as_millis(),
        SAMPLE_TOTAL_LIMIT.as_millis(),
        SAMPLE_P95_LIMIT.as_millis(),
        NATIVE_HOOK_LIMIT.as_millis(),
    );
}

fn event(workspace: &devmap::git::SourceWorkspace, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        format!("perf-{sequence}"),
        EventType::ToolCompleted,
        sequence,
        "2026-08-27T16:00:00Z",
        HostIdentity::new("performance", "1.0.0").unwrap(),
        ActorIdentity::new("agent-performance", None).unwrap(),
        SessionContext::new(
            SESSION,
            None,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
        json!({"capture_grade": "D", "activity": "tool_completed"}),
    )
    .unwrap()
}
