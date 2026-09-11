use super::*;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn isolated(name: &str) -> bool {
    if std::env::var("DEVMAP_ORIGIN_CACHE_CASE").as_deref() == Ok(name) {
        return true;
    }
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir(&home).unwrap();
    let stdout = directory.path().join("stdout");
    let stderr = directory.path().join("stderr");
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        &format!("store::snapshot::origin_cache_tests::{name}"),
        "--nocapture",
        "--test-threads=1",
    ]);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            cmd.env_remove(key);
        }
    }
    cmd.env("DEVMAP_ORIGIN_CACHE_CASE", name)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &home)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = Owned(cmd.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "owned origin test deadline");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{name}: {status}\n{}\n{}",
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
    false
}
fn git(root: &Path, args: &[&str]) {
    let output =
        crate::git_process::output(Command::new("git").arg("-C").arg(root).args(args)).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture(linked: bool) -> (tempfile::TempDir, SourceWorkspace, std::path::PathBuf) {
    use crate::events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    );
    let linked_root = directory.path().join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-qb",
            "linked",
            linked_root.to_str().unwrap(),
        ],
    );
    let workspace = crate::git::SourceGitInspector::open(if linked { &linked_root } else { &root })
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let event = EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        "event",
        EventType::CaptureGap,
        1,
        "2026-09-08T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("actor", None).unwrap(),
        SessionContext::new("session", None, "fixture", None, None, None).unwrap(),
        serde_json::json!({"reason":"fixture"}),
    )
    .unwrap();
    let journal = journal::JournalStore::open(&workspace, "session").unwrap();
    let record = journal.append(event).unwrap();
    PresenceStore::open(&workspace)
        .unwrap()
        .observe(
            presence::PresenceSignal::AcceptedRecords(&[record]),
            time::OffsetDateTime::now_utc(),
        )
        .unwrap();
    let legacy = workspace
        .git_dir
        .join("devmap/sessions/session/events.ndjson");
    super::super::migration::ensure(&workspace, &directory.path().join("frozen")).unwrap();
    assert!(
        super::super::is_active(
            RepositoryStore::open_existing(&workspace)
                .unwrap()
                .unwrap()
                .connection()
        )
        .unwrap()
    );
    (directory, workspace, legacy)
}
fn hot(linked: bool) {
    let (_owned, workspace, _) = fixture(linked);
    let mut reader = InputReader::new();
    let (generation, before) = reader.read(&workspace).unwrap();
    assert!(generation.is_some());
    assert_eq!(
        before.journals["session"].integrity,
        JournalIntegrity::Verified
    );
    let count = crate::git_process::test_spawn_count();
    let (next_generation, after) = reader.read(&workspace).unwrap();
    let starts = crate::git_process::test_spawn_count() - count;
    assert_eq!(generation, next_generation);
    assert_eq!(
        after.journals["session"].integrity,
        JournalIntegrity::Verified
    );
    assert_eq!(starts, 0, "active SQL repeat must reuse proven origins");
}
#[test]
fn active_sql_main_repeat_reuses_origin_enumeration() {
    if isolated("active_sql_main_repeat_reuses_origin_enumeration") {
        hot(false);
    }
}
#[test]
fn active_sql_linked_repeat_reuses_origin_enumeration() {
    if isolated("active_sql_linked_repeat_reuses_origin_enumeration") {
        hot(true);
    }
}
#[test]
fn active_sql_repeat_still_checks_complete_legacy_bytes() {
    if !isolated("active_sql_repeat_still_checks_complete_legacy_bytes") {
        return;
    }
    let (_owned, workspace, legacy) = fixture(false);
    let mut reader = InputReader::new();
    assert!(reader.read(&workspace).unwrap().0.is_some());
    fs::OpenOptions::new()
        .append(true)
        .open(&legacy)
        .unwrap()
        .write_all(b" ")
        .unwrap();
    assert!(
        reader.read(&workspace).is_err(),
        "enumeration reuse cannot skip legacy hash drift"
    );
}

#[test]
fn new_worktree_invalidates_cached_enumeration() {
    if !isolated("new_worktree_invalidates_cached_enumeration") {
        return;
    }
    let (owned, workspace, _) = fixture(false);
    let mut reader = InputReader::new();
    let generation = reader.read(&workspace).unwrap().0;
    let before = reader.origin_fingerprint().unwrap().to_owned();
    let new_root = owned.path().join("added");
    git(
        &workspace.root,
        &[
            "worktree",
            "add",
            "-qb",
            "added",
            new_root.to_str().unwrap(),
        ],
    );
    let count = crate::git_process::test_spawn_count();
    assert_eq!(reader.read(&workspace).unwrap().0, generation);
    assert!(crate::git_process::test_spawn_count() > count);
    assert_ne!(reader.origin_fingerprint(), Some(before.as_str()));
    let mut fresh = InputReader::new();
    assert_eq!(fresh.read(&workspace).unwrap().0, generation);
    assert_eq!(reader.origin_fingerprint(), fresh.origin_fingerprint());
    let count = crate::git_process::test_spawn_count();
    assert_eq!(reader.read(&workspace).unwrap().0, generation);
    assert_eq!(crate::git_process::test_spawn_count(), count);
}

#[test]
fn invalid_locked_bytes_preserve_fresh_scanner_error() {
    if !isolated("invalid_locked_bytes_preserve_fresh_scanner_error") {
        return;
    }
    let (_owned, workspace, _) = fixture(false);
    let mut reader = InputReader::new();
    reader.read(&workspace).unwrap();
    let locked = workspace.git_common_dir.join("worktrees/linked/locked");
    fs::write(&locked, [0xff, 0xfe]).unwrap();
    let fresh_error = InputReader::new()
        .read(&workspace)
        .err()
        .expect("fresh scanner must reject non UTF8 locked reason");
    let cached_error = reader
        .read(&workspace)
        .err()
        .expect("cached reader must preserve scanner refusal");
    assert_eq!(cached_error.to_string(), fresh_error.to_string());
    // Preserve the changed file in the owned fixture while repairing its absence.
    fs::rename(&locked, locked.with_file_name("retained-invalid-locked")).unwrap();
    let count = crate::git_process::test_spawn_count();
    reader.read(&workspace).unwrap();
    assert!(crate::git_process::test_spawn_count() > count);
    let count = crate::git_process::test_spawn_count();
    reader.read(&workspace).unwrap();
    assert_eq!(crate::git_process::test_spawn_count(), count);
}

#[test]
fn between_guard_change_rejects_result_and_discards_cache() {
    if !isolated("between_guard_change_rejects_result_and_discards_cache") {
        return;
    }
    let (owned, workspace, _) = fixture(false);
    let mut reader = InputReader::new();
    let generation = reader.read(&workspace).unwrap().0;
    assert!(reader.origins.has_entry());
    let store = RepositoryStore::open_existing(&workspace).unwrap().unwrap();
    let tx = store.connection().unchecked_transaction().unwrap();
    let root = owned.path().join("during-read");
    let result = reader.origins.observe_checked(&workspace, &tx, || {
        git(
            &workspace.root,
            &[
                "worktree",
                "add",
                "-qb",
                "during-read",
                root.to_str().unwrap(),
            ],
        );
    });
    let error = result
        .err()
        .expect("must reject observation spanning an origin change");
    assert!(
        error
            .to_string()
            .contains("origin enumeration proof changed")
    );
    assert!(
        !reader.origins.has_entry(),
        "failed observation must drop its proof"
    );
    tx.commit().unwrap();
    let count = crate::git_process::test_spawn_count();
    assert_eq!(reader.read(&workspace).unwrap().0, generation);
    assert!(crate::git_process::test_spawn_count() > count);
    let mut fresh = InputReader::new();
    fresh.read(&workspace).unwrap();
    assert_eq!(reader.origin_fingerprint(), fresh.origin_fingerprint());
    let count = crate::git_process::test_spawn_count();
    reader.read(&workspace).unwrap();
    assert_eq!(crate::git_process::test_spawn_count(), count);
}

fn report_value(report: &super::super::migration::ActiveOriginReport) -> serde_json::Value {
    serde_json::json!({
        "current": report.current(),
        "unavailable": report.unavailable,
        "fingerprint": report.fingerprint,
    })
}

#[test]
fn other_linked_head_change_matches_original_uncached_observer() {
    if !isolated("other_linked_head_change_matches_original_uncached_observer") {
        return;
    }
    let (_owned, workspace, _) = fixture(false);
    let mut reader = InputReader::new();
    reader.read(&workspace).unwrap();
    let head = workspace.git_common_dir.join("worktrees/linked/HEAD");
    let original = fs::read(&head).unwrap();
    fs::write(&head, b"malformed-linked-head\n").unwrap();
    let store = RepositoryStore::open_existing(&workspace).unwrap().unwrap();
    let direct =
        super::super::migration::observe_active_read_origins(&workspace, store.connection());
    let count = crate::git_process::test_spawn_count();
    let cached = reader.origins.observe(&workspace, store.connection());
    assert!(crate::git_process::test_spawn_count() > count);
    match (direct, cached) {
        (Ok(direct), Ok(cached)) => assert_eq!(report_value(&direct), report_value(&cached)),
        (Err(direct), Err(cached)) => assert_eq!(direct.to_string(), cached.to_string()),
        _ => panic!("cached observer disagrees with original observer"),
    }
    fs::write(&head, original).unwrap();
    let direct =
        super::super::migration::observe_active_read_origins(&workspace, store.connection())
            .unwrap();
    let cached = reader
        .origins
        .observe(&workspace, store.connection())
        .unwrap();
    assert_eq!(report_value(&direct), report_value(&cached));
}

fn assert_inputs_equal(left: &DockStorageInputs, right: &DockStorageInputs) {
    assert_eq!(left.presence, right.presence);
    assert_eq!(
        serde_json::to_value(&left.journals).unwrap(),
        serde_json::to_value(&right.journals).unwrap()
    );
    assert_eq!(
        left.routes
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap())
            .map_err(ToString::to_string),
        right
            .routes
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap())
            .map_err(ToString::to_string),
    );
    let bindings =
        |input: &DockStorageInputs| {
            input.bindings.as_ref().map(|value| {
        serde_json::json!({
            "records": value.records.iter().map(|row| serde_json::json!({
                "observation": row.observation, "visible_worktrees": row.visible_worktrees,
            })).collect::<Vec<_>>(),
            "incomplete_worktrees": value.incomplete_worktrees,
        })
    }).map_err(ToString::to_string)
        };
    assert_eq!(bindings(left), bindings(right));
}

#[test]
fn reader_switches_main_linked_main_without_sharing_source_proofs() {
    if !isolated("reader_switches_main_linked_main_without_sharing_source_proofs") {
        return;
    }
    let (owned, main, _) = fixture(false);
    let linked = crate::git::SourceGitInspector::open(owned.path().join("linked"))
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let mut reader = InputReader::new();
    for workspace in [&main, &linked, &main] {
        let count = crate::git_process::test_spawn_count();
        let (generation, inputs) = reader.read(workspace).unwrap();
        assert!(crate::git_process::test_spawn_count() > count);
        let mut fresh = InputReader::new();
        let count = crate::git_process::test_spawn_count();
        let (fresh_generation, fresh_inputs) = fresh.read(workspace).unwrap();
        assert!(
            crate::git_process::test_spawn_count() > count,
            "new reader cannot borrow old proof"
        );
        assert_eq!(generation, fresh_generation);
        assert_inputs_equal(&inputs, &fresh_inputs);
        let count = crate::git_process::test_spawn_count();
        let (hot_generation, hot_inputs) = reader.read(workspace).unwrap();
        assert_eq!(crate::git_process::test_spawn_count(), count);
        assert_eq!(generation, hot_generation);
        assert_inputs_equal(&inputs, &hot_inputs);
    }
}
