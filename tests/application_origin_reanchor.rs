mod support;

use devmap::{
    application::{ClientView, RepositoryApplication},
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStore},
    store::migration,
    worktrees::WorktreeScanner,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
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

#[test]
fn main_client_keeps_same_application_usable_after_its_linked_anchor_is_removed() {
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let linked_path = external.path().join("original-owner-anchor");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "owner-anchor",
            linked_path.to_str().unwrap(),
        ],
    );
    let main = workspace(repo.path());
    let linked = workspace(&linked_path);
    capture(&main, "main-survivor");
    capture(&linked, "old-anchor-history");
    let rows = WorktreeScanner::scan(&main).unwrap();
    let main_id = rows
        .iter()
        .find(|row| row.root == main.root)
        .unwrap()
        .worktree_id
        .clone();
    let linked_id = rows
        .iter()
        .find(|row| row.root == linked.root)
        .unwrap()
        .worktree_id
        .clone();
    let backup = external.path().join("frozen");
    migration::freeze(&main, &backup, OffsetDateTime::now_utc()).unwrap();
    migration::import_shadow(&main, &backup).unwrap();
    migration::activate(&main, &backup).unwrap();
    let sql_before = sql_rows(&main);
    let backup_before = backup_tree(&backup);
    let original_main_journal = main
        .git_dir
        .join("devmap/sessions/main-survivor/events.ndjson");
    let main_bytes = fs::read(&original_main_journal).unwrap();

    // This application is deliberately constructed from the soon-to-disappear
    // linked root. A main-anchored removal test does not exercise this boundary.
    let mut app = RepositoryApplication::open(&linked)
        .unwrap()
        .with_git_max_age(std::time::Duration::from_secs(60))
        .unwrap();
    let mut main_view = ClientView::new(main.clone());
    let warm = app
        .query(&mut main_view, OffsetDateTime::now_utc())
        .unwrap();
    assert!(
        warm.model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == main_id && lane.is_current)
    );
    assert!(
        warm.model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == linked_id)
    );
    assert_eq!(sql_rows(&main), sql_before);

    assert!(linked_path.starts_with(external.path()));
    support::git(
        repo.path(),
        ["worktree", "remove", linked_path.to_str().unwrap()],
    );
    assert!(!linked.root.exists());
    assert!(!linked.git_dir.exists());
    // The requesting client remains a physically valid member of this repository.
    let still_main = workspace(repo.path());
    assert_eq!(still_main.git_common_dir, main.git_common_dir);
    assert_eq!(still_main.git_dir, main.git_dir);
    let result = app.query(&mut main_view, OffsetDateTime::now_utc());
    assert_eq!(
        sql_rows(&main),
        sql_before,
        "query failure or reanchor must not mutate SQL"
    );
    assert_eq!(backup_tree(&backup), backup_before);
    assert_eq!(fs::read(&original_main_journal).unwrap(), main_bytes);
    let next = result.expect("same application must serve the surviving main client after its original linked anchor disappears");
    assert_eq!(next.store_generation, warm.store_generation);
    assert!(
        next.git_cycle > warm.git_cycle,
        "warm Git cache must not retain removed anchor topology"
    );
    assert!(
        next.model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == main_id && lane.is_current)
    );
    assert!(
        !next
            .model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == linked_id)
    );
    assert!(
        next.model
            .warnings
            .iter()
            .any(|warning| warning.code == "legacy_origin_unavailable"
                && warning.subject_id.as_deref() == Some(linked_id.as_str()))
    );
    assert!(
        !next
            .model
            .current
            .iter()
            .chain(&next.model.active)
            .chain(&next.model.stale_or_uninstrumented)
            .any(|entry| entry.session_id.as_deref() == Some("old-anchor-history"))
    );
    assert!(
        next.model
            .current
            .iter()
            .chain(&next.model.active)
            .any(|entry| entry.session_id.as_deref() == Some("main-survivor"))
    );
    let repeated = app
        .query(&mut main_view, OffsetDateTime::now_utc())
        .unwrap();
    assert_eq!(repeated.store_generation, next.store_generation);
    assert_eq!(sql_rows(&main), sql_before);
    assert_eq!(backup_tree(&backup), backup_before);
}
