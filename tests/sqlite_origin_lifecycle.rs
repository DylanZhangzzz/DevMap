mod support;

use devmap::{
    application::{ClientView, RepositoryApplication},
    canonical::sha256_hex,
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::{JournalRecord, JournalStore},
    presence::{PresenceSignal, PresenceStore},
    store::migration,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1788861600).unwrap()
}
fn workspace(path: &Path) -> SourceWorkspace {
    SourceGitInspector::open(path).unwrap().workspace().unwrap()
}
fn event(w: &SourceWorkspace, session: &str, id: &str, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        id,
        EventType::SessionStarted,
        sequence,
        "2026-09-08T10:00:00Z",
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
    .unwrap()
}
fn capture(w: &SourceWorkspace, session: &str, id: &str) -> JournalRecord {
    let record = JournalStore::open(w, session)
        .unwrap()
        .append_batch_with(|sequence| Ok(vec![event(w, session, id, sequence)]))
        .unwrap()
        .remove(0);
    PresenceStore::open(w)
        .unwrap()
        .observe(
            PresenceSignal::AcceptedRecords(std::slice::from_ref(&record)),
            now(),
        )
        .unwrap();
    record
}

fn atomic_capture(w: &SourceWorkspace, session: &str, id: &str) -> JournalRecord {
    JournalStore::open(w, session)
        .unwrap()
        .with_presence_projection()
        .append_batch_with(|sequence| Ok(vec![event(w, session, id, sequence)]))
        .unwrap()
        .remove(0)
}

struct Fixture {
    _repo: tempfile::TempDir,
    _external: tempfile::TempDir,
    main: SourceWorkspace,
    removed: SourceWorkspace,
    survivor: SourceWorkspace,
    backup: PathBuf,
    removed_id: String,
    route_id: String,
}
impl Fixture {
    fn new() -> Self {
        // tempfile inherits the root's trusted TMP location. All Git mutations
        // below are restricted to these owned, disposable fixture paths.
        let repo = support::committed_repo();
        let external = tempfile::tempdir().unwrap();
        let removed = external.path().join("removed");
        let survivor = external.path().join("survivor");
        for (path, branch) in [(&removed, "removed"), (&survivor, "survivor")] {
            support::git(
                repo.path(),
                ["worktree", "add", "-b", branch, path.to_str().unwrap()],
            );
        }
        let main = workspace(repo.path());
        let removed = workspace(&removed);
        let survivor = workspace(&survivor);
        capture(&main, "main-history", "main-event");
        capture(&removed, "removed-history", "removed-event");
        capture(&survivor, "survivor-history", "survivor-event");
        let removed_id = devmap::worktrees::WorktreeScanner::scan(&main)
            .unwrap()
            .into_iter()
            .find(|row| row.root == removed.root)
            .unwrap()
            .worktree_id;
        let route_id = devmap::route_plan::RoutePlanStore::open(&main)
            .unwrap()
            .set(devmap::route_plan::PlanInput {
                delivery: Default::default(),
                request_id: "old-route".into(),
                route_id: None,
                expected_revision: 0,
                worktree_id: removed_id.clone(),
                goal: "original worktree only".into(),
                target_ref: None,
                milestones: vec![],
                source: "user".into(),
                abandoned: false,
            })
            .unwrap()
            .route_id;
        let mut dock = devmap::dock::DockService::open(&removed.root).unwrap();
        dock.replace_observed_tasks(
            vec![devmap::dock::ObservedTask {
                working_directory: None,
                subagents: None,
                lifecycle: devmap::dock::TaskLifecycle::Present,
                session_id: "01a00000-0000-7000-8000-000000000001".into(),
                display_title: "old task".into(),
                host: "local".into(),
                host_status: "active".into(),
                workspace_path: removed.root.to_string_lossy().into(),
                status: devmap::presence::PresenceStatus::Working,
                updated_at: "2026-09-08T10:00:00Z".into(),
            }],
            now(),
        )
        .unwrap();
        drop(dock);
        let backup = external.path().join("frozen");
        let manifest = migration::freeze(&main, &backup, now()).unwrap();
        assert_eq!(manifest.origins.len(), 3);
        migration::import_shadow(&main, &backup).unwrap();
        migration::activate(&main, &backup).unwrap();
        Self {
            _repo: repo,
            _external: external,
            main,
            removed,
            survivor,
            backup,
            removed_id,
            route_id,
        }
    }
    fn remove(&self) {
        assert!(self.removed.root.starts_with(self._external.path()));
        support::git(
            &self.main.root,
            ["worktree", "remove", self.removed.root.to_str().unwrap()],
        );
        assert!(!self.removed.root.exists());
        assert!(!self.removed.git_dir.exists());
    }
    fn db(&self) -> Connection {
        Connection::open_with_flags(
            self.main.git_common_dir.join("devmap/devmap.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }
}

fn sql_snapshot(f: &Fixture) -> Vec<Vec<String>> {
    let mut c = f.db();
    let tx = c.transaction().unwrap();
    let sql = [
        "SELECT json_array(schema_version,repository_id,common_dir,generation,backend_state) FROM store_meta ORDER BY singleton",
        "SELECT json_array(worktree_id,incarnation,git_dir,workspace_path,retired_at) FROM worktree_registry ORDER BY worktree_id,incarnation",
        "SELECT json_array(session_id,worktree_id,incarnation,origin_path) FROM journal_sessions ORDER BY session_id",
        "SELECT json_array(session_id,sequence,event_id,record_json,byte_length) FROM journal_records ORDER BY session_id,sequence",
        "SELECT json_array(session_id,record_count,last_sha256,byte_length) FROM journal_heads ORDER BY session_id",
        "SELECT json_array(session_id,record_json) FROM presence_records ORDER BY session_id",
        "SELECT json_array(session_id,covered_sequence,covered_sha256,baseline_source) FROM presence_projection ORDER BY session_id",
        "SELECT json_array(source_path,source_hash,record_count,outcome,record_json) FROM migration_sources ORDER BY source_path",
        "SELECT json_array(route_id,revision,request_id,input_json,plan_json) FROM route_records ORDER BY route_id,revision",
        "SELECT json_array(observation_id,host,task_id,observed_at,record_json) FROM binding_records ORDER BY observation_id",
        "SELECT json_array(source_scope,observed_at,record_json) FROM binding_watermarks ORDER BY source_scope",
    ];
    let rows = sql
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
    tx.commit().unwrap();
    rows
}

#[test]
fn migrated_sources_without_activation_and_fence_cannot_become_native_journal() {
    let f = Fixture::new();
    let backup = backup_hashes(&f.backup);
    let opened = JournalStore::open(&f.main, "main-history")
        .unwrap()
        .with_presence_projection();
    let c = Connection::open(f.main.git_common_dir.join("devmap/devmap.db")).unwrap();
    let removed = c
        .execute(
            "DELETE FROM migration_sources WHERE source_path='@activation'",
            [],
        )
        .unwrap();
    assert_eq!(removed, 1, "fixture must have completed real activation");
    let remaining: i64 = c
        .query_row(
            "SELECT count(*) FROM migration_sources WHERE source_path NOT LIKE '@%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        remaining > 0,
        "real imported per-source evidence must remain"
    );
    let fence = f.main.git_common_dir.join("devmap/activation-intent.json");
    assert!(fence.is_file());
    fs::remove_file(&fence).unwrap();
    let before = sql_snapshot(&f);
    let built = std::cell::Cell::new(false);
    let result = opened.append_batch_with(|sequence| {
        built.set(true);
        Ok(vec![event(
            &f.main,
            "main-history",
            "must-refuse",
            sequence,
        )])
    });
    assert!(
        result.is_err(),
        "partial migration evidence must not select native admission"
    );
    assert!(
        !built.get(),
        "provenance must fail before event construction"
    );
    assert!(opened.replay().is_err());
    assert!(JournalStore::open(&f.main, "new-session").is_err());
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
    assert!(!fence.exists());
}
fn backup_hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for item in fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            if metadata.is_dir() {
                walk(root, &path, out)
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    sha256_hex(&fs::read(path).unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn qualified_read(w: &SourceWorkspace) -> devmap::dock::DockReadModel {
    let mut app = RepositoryApplication::open(w).unwrap();
    let mut view = ClientView::new(w.clone());
    app.query(&mut view,now()).expect("authoritative SQL history remains readable with an origin warning after linked origin removal").model
}
fn assert_old_presence_is_not_live(model: &devmap::dock::DockReadModel) {
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "presence_worktree_missing"
                && warning.subject_id.as_deref() == Some("removed-history")),
        "old incarnation needs the existing missing-worktree warning, not silent reassociation"
    );
    assert!(
        !model
            .current
            .iter()
            .chain(&model.active)
            .chain(&model.stale_or_uninstrumented)
            .any(|entry| entry.session_id.as_deref() == Some("removed-history")),
        "historical presence must not become a current session"
    );
}

#[test]
fn removed_linked_origin_keeps_qualified_sql_history_and_frozen_evidence() {
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    let mut app = RepositoryApplication::open(&f.survivor)
        .unwrap()
        .with_git_max_age(std::time::Duration::from_secs(60))
        .unwrap();
    let mut view = ClientView::new(f.survivor.clone());
    let warm = app.query(&mut view, now()).unwrap().model;
    assert!(
        warm.lanes
            .iter()
            .any(|lane| lane.worktree_id == f.removed_id)
    );
    f.remove();
    let model = app.query(&mut view, now()).unwrap().model;
    assert!(
        !model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == f.removed_id)
    );
    assert_old_presence_is_not_live(&model);
    assert!(model.lanes.iter().any(|lane| lane.is_current));
    assert_eq!(
        sql_snapshot(&f),
        before,
        "read/removal observation must not retire or rewrite any SQL history/provenance"
    );
    assert_eq!(backup_hashes(&f.backup), backup);
}

#[test]
fn same_path_replacement_cannot_inherit_old_session_or_live_presence() {
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    let old_handle = JournalStore::open(&f.removed, "removed-history").unwrap();
    let unopened_session_handle = JournalStore::open(&f.removed, "never-appended").unwrap();
    let original_event = old_handle.replay().unwrap()[0].event.clone();
    let standalone_presence = PresenceStore::open(&f.survivor).unwrap();
    let old_incarnation: String = f
        .db()
        .query_row(
            "SELECT incarnation FROM journal_sessions WHERE session_id='removed-history'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.remove();
    support::git(
        &f.main.root,
        [
            "worktree",
            "add",
            f.removed.root.to_str().unwrap(),
            "removed",
        ],
    );
    let replacement = workspace(&f.removed.root);
    assert_eq!(
        replacement.git_dir, f.removed.git_dir,
        "exercise reused path-derived worktree identity"
    );
    assert!(
        old_handle.append(original_event.clone()).is_err(),
        "old handle cannot retrieve a retry receipt from a replacement incarnation"
    );
    assert!(
        old_handle.replay().is_err(),
        "old handle cannot replay through a replacement path"
    );
    assert!(
        unopened_session_handle
            .append(event(&replacement, "never-appended", "stale-handle", 1))
            .is_err(),
        "a handle opened before replacement cannot register a new session afterward"
    );
    assert!(
        JournalStore::open(&replacement, "removed-history")
            .and_then(|store| store.append(original_event))
            .is_err()
    );
    assert!(
        standalone_presence
            .observe(
                PresenceSignal::ExplicitWaiting {
                    session_id: "survivor-history",
                    activity_id: None
                },
                now()
            )
            .is_err()
    );
    assert!(PresenceStore::open(&replacement).is_err());
    assert!(
        devmap::route_plan::RoutePlanStore::open(&replacement)
            .unwrap()
            .set(devmap::route_plan::PlanInput {
                delivery: Default::default(),
                request_id: "forbidden-lifecycle-route".into(),
                route_id: None,
                expected_revision: 0,
                worktree_id: f.removed_id.clone(),
                goal: "not admitted".into(),
                target_ref: None,
                milestones: vec![],
                source: "user".into(),
                abandoned: false,
            })
            .is_err()
    );
    assert_eq!(sql_snapshot(&f), before);
    let model = qualified_read(&f.survivor);
    assert_old_presence_is_not_live(&model);
    assert!(
        !model
            .route_plans
            .iter()
            .any(|plan| plan.route_id == f.route_id)
    );
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "planned_workspace_unavailable"
                && warning.subject_id.as_deref() == Some(f.route_id.as_str()))
    );
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "task_binding_history_unavailable"
                && warning.subject_id.as_deref() == Some(f.removed_id.as_str()))
    );
    // The typed route collection above and complete binding overlay below both
    // protect against path-derived reassociation of the replaced worktree.
    let replaced = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == f.removed_id)
        .unwrap();
    assert!(!replaced.bindings_complete);
    assert!(replaced.bindings.is_empty());
    assert!(
        !replaced
            .origin
            .plan_starts
            .iter()
            .any(|start| start.route_id.as_deref() == Some(f.route_id.as_str()))
    );
    let old = JournalStore::open(&replacement, "removed-history");
    assert!(
        old.and_then(|store| store.append(event(&replacement, "removed-history", "forbidden", 2)))
            .is_err()
    );
    assert_eq!(
        sql_snapshot(&f),
        before,
        "old session rejection cannot alter history or infer retired_at"
    );

    // Separate write-restoration gate: a new session may bind only to the new
    // incarnation. This does not authorize skipping present legacy byte checks.
    let accepted = atomic_capture(&replacement, "replacement-history", "replacement-event");
    let after_capture = sql_snapshot(&f);
    let duplicate = atomic_capture(&replacement, "replacement-history", "replacement-event");
    assert_eq!(duplicate, accepted);
    assert_eq!(
        sql_snapshot(&f),
        after_capture,
        "exact capture retry cannot renew projection or advance generation"
    );
    let route_associated = EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        "route-association-not-admitted",
        EventType::SessionStarted,
        2,
        "2026-09-08T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("actor", None).unwrap(),
        SessionContext::new(
            "replacement-history",
            Some(f.route_id.clone()),
            replacement.root.to_string_lossy(),
            Some(replacement.root.to_string_lossy().into_owned()),
            replacement.branch.clone(),
            Some(replacement.head.clone()),
        )
        .unwrap(),
        serde_json::json!({"activity":"session_started"}),
    )
    .unwrap();
    let error = JournalStore::open(&replacement, "replacement-history")
        .unwrap()
        .with_presence_projection()
        .append(route_associated)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy source changed after activation"),
        "{error}"
    );
    assert_eq!(
        sql_snapshot(&f),
        after_capture,
        "route-associated capture is outside the journal-only exception"
    );
    let new_incarnation: String = f
        .db()
        .query_row(
            "SELECT incarnation FROM journal_sessions WHERE session_id='replacement-history'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(new_incarnation, old_incarnation);
    let after = sql_snapshot(&f);
    for index in 1..before.len() {
        for row in &before[index] {
            assert!(
                after[index].contains(row),
                "historical SQL row changed in table {index}"
            );
        }
    }
    assert_eq!(
        after[7], before[7],
        "migration provenance must not be rewritten for replacement"
    );
    assert_eq!(backup_hashes(&f.backup), backup);
    assert_old_presence_is_not_live(&qualified_read(&f.survivor));
}

#[test]
fn last_linked_origin_removal_allows_missing_administration_parent() {
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    f.remove();
    support::git(
        &f.main.root,
        ["worktree", "remove", f.survivor.root.to_str().unwrap()],
    );
    let administration = f.main.git_common_dir.join("worktrees");
    // Some Git versions retain an empty parent. Removing only this verified
    // fixture-owned empty directory covers versions that clean it themselves.
    assert!(
        fs::canonicalize(&f.main.git_common_dir)
            .unwrap()
            .starts_with(fs::canonicalize(f._repo.path()).unwrap())
    );
    if administration.exists() {
        assert_eq!(fs::read_dir(&administration).unwrap().count(), 0);
        fs::remove_dir(&administration).unwrap();
    }
    assert!(!administration.exists());
    let model = qualified_read(&f.main);
    assert_old_presence_is_not_live(&model);
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
}

#[test]
fn missing_origin_does_not_excuse_present_origin_byte_tamper() {
    use std::io::Write;
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    f.remove();
    let path = f
        .main
        .git_dir
        .join("devmap/sessions/main-history/events.ndjson");
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(b" ")
        .unwrap();
    let mut app = RepositoryApplication::open(&f.survivor).unwrap();
    let mut view = ClientView::new(f.survivor.clone());
    let error = app.query(&mut view, now()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy source inventory/hash drift"),
        "{error}"
    );
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
}

#[test]
fn linked_origin_foreign_git_pointer_is_corruption_not_replacement() {
    let f = Fixture::new();
    let foreign = support::committed_repo();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    fs::write(
        f.removed.root.join(".git"),
        format!("gitdir: {}\n", foreign.path().join(".git").display()),
    )
    .unwrap();
    let mut app = RepositoryApplication::open(&f.survivor).unwrap();
    let mut view = ClientView::new(f.survivor.clone());
    assert!(
        app.query(&mut view, now()).is_err(),
        "foreign repository pointer must not qualify as an unavailable/replaced origin"
    );
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
}

#[test]
fn linked_origin_wrong_backlink_is_corruption_not_replacement() {
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    fs::write(
        f.removed.git_dir.join("gitdir"),
        format!("{}\n", f.survivor.root.join(".git").display()),
    )
    .unwrap();
    let mut app = RepositoryApplication::open(&f.survivor).unwrap();
    let mut view = ClientView::new(f.survivor.clone());
    assert!(
        app.query(&mut view, now()).is_err(),
        "mismatched worktree/admin backlink must not qualify as replacement"
    );
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
}

#[test]
fn linked_origin_wrong_backlink_basename_is_rejected() {
    let f = Fixture::new();
    let before = sql_snapshot(&f);
    let backup = backup_hashes(&f.backup);
    let wrong_marker = f.removed.root.join(".git-wrong");
    fs::copy(f.removed.root.join(".git"), &wrong_marker).unwrap();
    fs::write(
        f.removed.git_dir.join("gitdir"),
        format!("{}\n", wrong_marker.display()),
    )
    .unwrap();
    let mut app = RepositoryApplication::open(&f.survivor).unwrap();
    let mut view = ClientView::new(f.survivor.clone());
    assert!(
        app.query(&mut view, now()).is_err(),
        "backlink basename must match the actual root .git marker"
    );
    assert_eq!(sql_snapshot(&f), before);
    assert_eq!(backup_hashes(&f.backup), backup);
}
