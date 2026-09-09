mod support;

use devmap::{
    application::{ClientView, RepositoryApplication},
    dock::{DockService, ObservedTask, TaskLifecycle},
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStatus, PresenceStore},
    route_plan::{PlanInput, RoutePlanStore},
    store::migration,
    worktrees::WorktreeScanner,
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
fn capture(w: &SourceWorkspace, session: &str) {
    let event = EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        format!("{session}-event"),
        EventType::SessionStarted,
        1,
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
    .unwrap();
    let record = JournalStore::open(w, session)
        .unwrap()
        .append(event)
        .unwrap();
    PresenceStore::open(w)
        .unwrap()
        .observe(PresenceSignal::AcceptedRecords(&[record]), now())
        .unwrap();
}
fn database(w: &SourceWorkspace) -> Connection {
    Connection::open_with_flags(
        w.git_common_dir.join("devmap/devmap.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}
fn sql_snapshot(w: &SourceWorkspace) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let mut c = database(w);
    let tx = c.transaction().unwrap();
    let mut output = BTreeMap::new();
    let tables = [
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
    ];
    let actual = tx
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()
        .unwrap();
    assert_eq!(actual, tables.iter().map(|name| name.to_string()).collect());
    for table in tables {
        let mut statement = tx
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        output.insert(table.to_owned(), rows);
    }
    tx.commit().unwrap();
    output
}
fn backup_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            if metadata.is_dir() {
                walk(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
fn replacement_scenario(with_journal: bool) {
    // All fixtures inherit the root-approved external TMP directory.
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let main = workspace(repo.path());
    capture(&main, "legacy-main");
    let backup = external.path().join("frozen");
    let manifest = migration::freeze(&main, &backup, now()).unwrap();
    assert_eq!(manifest.origins.len(), 1);
    migration::import_shadow(&main, &backup).unwrap();
    migration::activate(&main, &backup).unwrap();
    let frozen = backup_bytes(&backup);

    // This origin did not exist at activation, so its identity must come from
    // native SQL domain writes rather than the frozen manifest.
    let path = external.path().join("native-linked");
    support::git(
        &main.root,
        [
            "worktree",
            "add",
            "-b",
            "native-linked",
            path.to_str().unwrap(),
        ],
    );
    let native = workspace(&path);
    let id = WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|r| r.root == native.root)
        .unwrap()
        .worktree_id;
    if with_journal {
        capture(&native, "native-session");
    }
    let plan = RoutePlanStore::open(&main)
        .unwrap()
        .set(PlanInput {
            delivery: Default::default(),
            request_id: "native-route".into(),
            route_id: None,
            expected_revision: 0,
            worktree_id: id.clone(),
            goal: "native original only".into(),
            target_ref: None,
            milestones: vec![],
            source: "user".into(),
            abandoned: false,
        })
        .unwrap();
    let mut dock = DockService::open(&native.root).unwrap();
    dock.replace_observed_tasks(
        vec![ObservedTask {
            working_directory: None,
            subagents: None,
            lifecycle: TaskLifecycle::Present,
            session_id: "01a00000-0000-7000-8000-000000000009".into(),
            display_title: "native task".into(),
            host: "local".into(),
            host_status: "active".into(),
            workspace_path: native.root.to_string_lossy().into_owned(),
            status: PresenceStatus::Working,
            updated_at: "2026-09-08T10:00:00Z".into(),
        }],
        now(),
    )
    .unwrap();
    drop(dock);
    let registry_count: i64 = database(&main)
        .query_row(
            "SELECT count(*) FROM worktree_registry WHERE worktree_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!(
        "native origin registry rows after route/binding writes: {registry_count}; journal={with_journal}"
    );
    if with_journal {
        assert!(registry_count > 0);
    }
    let mut app = RepositoryApplication::open(&main).unwrap();
    let mut view = ClientView::new(main.clone());
    let first = app.query(&mut view, now()).unwrap().model;
    assert!(
        first
            .route_plans
            .iter()
            .any(|r| r.route_id == plan.route_id)
    );
    let facts = first
        .workspace_facts
        .iter()
        .find(|f| f.worktree_id == id)
        .unwrap();
    assert!(
        !facts.bindings.is_empty(),
        "prove the original binding was retained before replacement"
    );
    let before = sql_snapshot(&main);
    support::git(&main.root, ["worktree", "remove", path.to_str().unwrap()]);
    support::git(
        &main.root,
        ["worktree", "add", path.to_str().unwrap(), "native-linked"],
    );
    assert_eq!(workspace(&path).git_dir, native.git_dir);
    let model = app.query(&mut view, now()).unwrap().model;
    assert_eq!(
        sql_snapshot(&main),
        before,
        "qualification preserves every SQL history/provenance row"
    );
    assert_eq!(backup_bytes(&backup), frozen);
    if with_journal {
        assert!(
            !model
                .current
                .iter()
                .chain(&model.active)
                .chain(&model.stale_or_uninstrumented)
                .any(|entry| entry.session_id.as_deref() == Some("native-session"))
        );
    }
    let facts = model
        .workspace_facts
        .iter()
        .find(|f| f.worktree_id == id)
        .unwrap();
    let attached_route = model
        .route_plans
        .iter()
        .any(|r| r.route_id == plan.route_id)
        || facts
            .origin
            .plan_starts
            .iter()
            .any(|start| start.route_id.as_deref() == Some(plan.route_id.as_str()));
    let attached_binding = !facts.bindings.is_empty();
    assert_eq!(
        (attached_route, attached_binding),
        (false, false),
        "replacement cannot inherit native pre-replacement route/binding; registry rows={registry_count}, journal={with_journal}"
    );
    assert!(
        !facts.bindings_complete,
        "unqualified historical binding attachment must be explicitly incomplete"
    );
}

#[test]
fn post_activation_journal_origin_replacement_cannot_inherit_route_or_binding() {
    replacement_scenario(true);
}
#[test]
fn post_activation_route_binding_only_origin_replacement_cannot_inherit_attachments() {
    replacement_scenario(false);
}

fn activated_native() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    SourceWorkspace,
    PathBuf,
    String,
) {
    let repo = support::committed_repo();
    let files = tempfile::tempdir().unwrap();
    let main = workspace(repo.path());
    let backup = files.path().join("baseline");
    migration::freeze(&main, &backup, now()).unwrap();
    migration::import_shadow(&main, &backup).unwrap();
    migration::activate(&main, &backup).unwrap();
    let path = files.path().join("native-a");
    support::git(
        &main.root,
        ["worktree", "add", "-b", "native-a", path.to_str().unwrap()],
    );
    let native = workspace(&path);
    let expected_git_dir = fs::canonicalize(&native.git_dir).unwrap();
    let scanned = WorktreeScanner::scan(&main).unwrap();
    let id = scanned.iter()
        .find(|row| fs::canonicalize(&row.git_dir).unwrap() == expected_git_dir)
        .unwrap_or_else(|| panic!(
            "native administration directory not found: expected raw={:?}, canonical={:?}; scanned={:?}",
            native.git_dir, expected_git_dir,
            scanned.iter().map(|row| (&row.git_dir, fs::canonicalize(&row.git_dir))).collect::<Vec<_>>()
        ))
        .worktree_id.clone();
    (repo, files, main, path, id)
}
#[test]
fn route_only_native_origin_rejects_presence_without_an_accepted_sql_session() {
    let (_repo, files, main, path, id) = activated_native();
    let native = workspace(&path);
    let plan = RoutePlanStore::open(&main)
        .unwrap()
        .set(route_input(&id, "route-only-presence-gate"))
        .unwrap();
    let registered: i64 = database(&main)
        .query_row(
            "SELECT count(*) FROM route_origin_links AS l JOIN worktree_registry AS r ON r.worktree_id=l.worktree_id AND r.incarnation=l.incarnation WHERE l.route_id=?1 AND l.worktree_id=?2 AND l.qualification='native_verified'",
            [&plan.route_id, &id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(registered, 1, "route write must register the native origin");
    assert_eq!(
        database(&main)
            .query_row("SELECT count(*) FROM journal_sessions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    // Produce a canonical, replayable record through the public legacy writer.
    // Its context names A1, but A1's active SQL store has never accepted it.
    let legacy_repo = support::committed_repo();
    let legacy = workspace(legacy_repo.path());
    let session = "external-native-presence";
    let event = EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        "external-native-presence-event",
        EventType::SessionStarted,
        1,
        "2026-09-08T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("actor", None).unwrap(),
        SessionContext::new(
            session,
            None,
            native.root.to_string_lossy(),
            Some(native.root.to_string_lossy().into_owned()),
            native.branch.clone(),
            Some(native.head.clone()),
        )
        .unwrap(),
        serde_json::json!({"activity":"session_started"}),
    )
    .unwrap();
    let journal = JournalStore::open(&legacy, session).unwrap();
    let record = journal.append(event).unwrap();
    assert_eq!(journal.replay().unwrap(), vec![record.clone()]);
    assert!(!legacy.git_common_dir.join("devmap/devmap.db").exists());

    let before = sql_snapshot(&main);
    assert!(before["journal_sessions"].is_empty());
    assert!(before["journal_records"].is_empty());
    assert!(before["presence_records"].is_empty());
    let frozen = backup_bytes(&files.path().join("baseline"));
    let presence = PresenceStore::open(&native).unwrap();
    let accepted_error = presence
        .observe(PresenceSignal::AcceptedRecords(&[record]), now())
        .unwrap_err();
    assert!(
        matches!(
            accepted_error,
            devmap::error::DevMapError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
        ),
        "unexpected accepted-record rejection: {accepted_error:?}"
    );
    assert_eq!(sql_snapshot(&main), before);
    assert_eq!(backup_bytes(&files.path().join("baseline")), frozen);

    let waiting_error = presence
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: session,
                activity_id: None,
            },
            now(),
        )
        .unwrap_err();
    assert!(
        matches!(
            waiting_error,
            devmap::error::DevMapError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
        ),
        "unexpected explicit-waiting rejection: {waiting_error:?}"
    );
    assert_eq!(sql_snapshot(&main), before);
    assert_eq!(backup_bytes(&files.path().join("baseline")), frozen);
}

fn recreate_native(main: &SourceWorkspace, path: &Path) {
    let old = workspace(path);
    support::git(&main.root, ["worktree", "remove", path.to_str().unwrap()]);
    support::git(
        &main.root,
        ["worktree", "add", path.to_str().unwrap(), "native-a"],
    );
    assert_eq!(workspace(path).git_dir, old.git_dir);
}
fn route_input(id: &str, request: &str) -> PlanInput {
    PlanInput {
        delivery: Default::default(),
        request_id: request.into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: id.into(),
        goal: request.into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    }
}
#[test]
fn replacement_accepts_a_new_qualified_route_but_old_receipt_and_cas_stay_historical() {
    let (_repo, files, main, path, id) = activated_native();
    let frozen = backup_bytes(&files.path().join("baseline"));
    let store = RoutePlanStore::open(&main).unwrap();
    let input = route_input(&id, "old-native-route");
    let old = store.set(input.clone()).unwrap();
    let link = |route: &str| {
        database(&main).query_row(
        "SELECT incarnation,qualification FROM route_origin_links WHERE route_id=?1 AND revision=1", [route],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap()
    };
    let old_link = link(&old.route_id);
    assert_eq!(old_link.1, "native_verified");
    let old_json: String = database(&main)
        .query_row(
            "SELECT plan_json FROM route_records WHERE route_id=?1",
            [&old.route_id],
            |row| row.get(0),
        )
        .unwrap();
    recreate_native(&main, &path);
    let fresh = store.set(route_input(&id, "new-native-route")).unwrap();
    assert_ne!(fresh.route_id, old.route_id);
    let fresh_link = link(&fresh.route_id);
    assert_eq!(fresh_link.1, "native_verified");
    assert_ne!(fresh_link.0, old_link.0);
    assert_eq!(link(&old.route_id), old_link);
    let before = sql_snapshot(&main);
    assert_eq!(
        store.set(input.clone()).unwrap(),
        old,
        "exact old request must retain its receipt"
    );
    let mut stale = input;
    stale.request_id = "stale-native-update".into();
    stale.route_id = Some(old.route_id.clone());
    match store.set(stale).unwrap_err() {
        devmap::error::DevMapError::RoutePlanConflict {
            revision,
            current_plan,
        } => {
            assert_eq!(revision, 1);
            assert_eq!(current_plan.as_deref(), Some(&old));
        }
        error => panic!("lost structured CAS conflict: {error}"),
    }
    assert_eq!(
        sql_snapshot(&main),
        before,
        "receipt and stale CAS must not mutate identity tables"
    );
    assert_eq!(
        database(&main)
            .query_row(
                "SELECT plan_json FROM route_records WHERE route_id=?1",
                [&old.route_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        old_json
    );
    let mut app = RepositoryApplication::open(&main).unwrap();
    let mut view = ClientView::new(main.clone());
    let model = app.query(&mut view, now()).unwrap().model;
    assert!(
        model
            .route_plans
            .iter()
            .any(|plan| plan.route_id == fresh.route_id)
    );
    assert!(
        !model
            .route_plans
            .iter()
            .any(|plan| plan.route_id == old.route_id)
    );
    assert_eq!(backup_bytes(&files.path().join("baseline")), frozen);
}
fn native_task(path: &Path, at: OffsetDateTime) -> ObservedTask {
    ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: TaskLifecycle::Present,
        session_id: "01a00000-0000-7000-8000-000000000010".into(),
        display_title: "origin-aware task".into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: path.to_string_lossy().into_owned(),
        status: PresenceStatus::Working,
        updated_at: at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
    }
}
#[test]
fn same_path_binding_replacement_updates_cursor_then_move_preserves_public_history_chain() {
    let (_repo, files, main, path, id) = activated_native();
    let frozen = backup_bytes(&files.path().join("baseline"));
    let mut app = RepositoryApplication::open(&main).unwrap();
    let mut view = ClientView::new(main.clone());
    let t1 = now();
    let t2 = now() + time::Duration::seconds(10);
    let t3 = now() + time::Duration::seconds(20);
    let task = native_task(&path, t1);
    let scope = serde_json::to_string(&(&task.host, &task.session_id)).unwrap();
    let cursor = || {
        database(&main).query_row(
        "SELECT current_worktree_id,current_incarnation,observed_at,history_observation_id FROM binding_origin_cursors WHERE source_scope=?1", [&scope],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))).unwrap()
    };
    app.accept_inventory(&mut view, vec![task.clone()], true, t1)
        .unwrap();
    let first_cursor = cursor();
    let first_generation: i64 = database(&main)
        .query_row("SELECT generation FROM store_meta", [], |row| row.get(0))
        .unwrap();
    assert_eq!(first_cursor.0, id);
    let first_json: String = database(&main)
        .query_row(
            "SELECT record_json FROM binding_records WHERE observation_id=?1",
            [&first_cursor.3],
            |row| row.get(0),
        )
        .unwrap();
    recreate_native(&main, &path);
    app.accept_inventory(&mut view, vec![native_task(&path, t2)], true, t2)
        .unwrap();
    let second_cursor = cursor();
    assert_eq!(
        second_cursor.2,
        t2.format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    );
    assert_eq!(
        database(&main)
            .query_row("SELECT generation FROM store_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        first_generation + 1
    );
    assert_eq!(second_cursor.0, id);
    assert_ne!(second_cursor.1, first_cursor.1);
    assert_eq!(
        second_cursor.3, first_cursor.3,
        "same-ID replacement must not invent a public migration"
    );
    assert_eq!(
        database(&main)
            .query_row("SELECT count(*) FROM binding_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        database(&main)
            .query_row(
                "SELECT record_json FROM binding_records WHERE observation_id=?1",
                [&first_cursor.3],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        first_json
    );
    assert_eq!(
        database(&main)
            .query_row(
                "SELECT destination_incarnation FROM binding_origin_links WHERE observation_id=?1",
                [&first_cursor.3],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        first_cursor.1
    );
    let before_retry = sql_snapshot(&main);
    app.accept_inventory(&mut view, vec![native_task(&path, t2)], true, t2)
        .unwrap();
    assert_eq!(sql_snapshot(&main), before_retry);
    let destination = files.path().join("native-b");
    support::git(
        &main.root,
        [
            "worktree",
            "add",
            "-b",
            "native-b",
            destination.to_str().unwrap(),
        ],
    );
    app.accept_inventory(&mut view, vec![native_task(&destination, t3)], true, t3)
        .unwrap();
    let third_cursor = cursor();
    assert_ne!(third_cursor.0, id);
    let model = app.query(&mut view, t3).unwrap().model;
    assert_eq!(
        database(&main)
            .query_row("SELECT count(*) FROM binding_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2,
        "public binding chain must remain parseable with no fake same-ID movement"
    );
    // This public projection invokes sql_binding_snapshot's complete production
    // history parser before qualifying records. Invalid chains cannot produce
    // this destination binding; raw JSON deserialization is not the oracle.
    let binding = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == third_cursor.0)
        .unwrap()
        .bindings
        .iter()
        .find(|binding| binding.task_id == task.session_id)
        .unwrap();
    assert_eq!(binding.from_worktree_id.as_deref(), Some(id.as_str()));
    assert_eq!(binding.previous_observed_at, Some(first_cursor.2.clone()));
    assert_eq!(binding.kind, "task_migration_observed");
    let (source,source_incarnation,destination_id,destination_incarnation): (String,String,String,String) = database(&main).query_row(
        "SELECT source_worktree_id,source_incarnation,destination_worktree_id,destination_incarnation FROM binding_origin_links WHERE observation_id=?1", [&third_cursor.3],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!((source, source_incarnation), (id, second_cursor.1));
    assert_eq!(
        (destination_id, destination_incarnation),
        (third_cursor.0, third_cursor.1)
    );
    assert!(
        model
            .lanes
            .iter()
            .flat_map(|lane| &lane.chats)
            .any(|chat| chat.codex_thread_id.as_deref() == Some(task.session_id.as_str()))
    );
    assert_eq!(backup_bytes(&files.path().join("baseline")), frozen);
}
