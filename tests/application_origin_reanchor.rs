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

fn move_anchor_reoccupation(foreign: bool) {
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let old_path = external.path().join("anchor-old");
    let new_path = external.path().join("anchor-moved");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "anchor-move",
            old_path.to_str().unwrap(),
        ],
    );
    let main = workspace(repo.path());
    let old = workspace(&old_path);
    if !foreign {
        support::git(repo.path(), ["config", "extensions.worktreeConfig", "true"]);
        support::git(repo.path(), ["branch", "target-one"]);
        support::git(repo.path(), ["branch", "target-two"]);
        support::git(
            &old.root,
            [
                "config",
                "--worktree",
                "devmap.developmentTarget",
                "target-one",
            ],
        );
        assert_eq!(
            support::git(&old.root, ["config", "--get", "devmap.developmentTarget"]),
            "target-one"
        );
    }

    capture(&main, "main-move-survivor");
    capture(&old, "moving-anchor-history");
    let ids = WorktreeScanner::scan(&main).unwrap();
    let main_id = ids
        .iter()
        .find(|w| w.root == main.root)
        .unwrap()
        .worktree_id
        .clone();
    let old_id = ids
        .iter()
        .find(|w| w.root == old.root)
        .unwrap()
        .worktree_id
        .clone();
    let backup = external.path().join("frozen-anchor");
    migration::freeze(&main, &backup, OffsetDateTime::now_utc()).unwrap();
    migration::import_shadow(&main, &backup).unwrap();
    migration::activate(&main, &backup).unwrap();
    let before = sql_rows(&main);
    let frozen = backup_tree(&backup);
    let legacy_path = old
        .git_dir
        .join("devmap/sessions/moving-anchor-history/events.ndjson");
    let legacy_bytes = fs::read(&legacy_path).unwrap();
    let mut app = RepositoryApplication::open(&old)
        .unwrap()
        .with_git_max_age(std::time::Duration::from_secs(60))
        .unwrap();
    let mut old_view = ClientView::new(old.clone());
    let warm = app.query(&mut old_view, OffsetDateTime::now_utc()).unwrap();
    assert!(
        warm.model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == old_id && lane.is_current)
    );
    if !foreign {
        assert_eq!(
            warm.model.development_target.as_ref().unwrap().ref_name,
            "refs/heads/target-one"
        );
    }
    // Relationship configuration is client-owned, not inherited from the anchor.
    let main_target = app
        .query(
            &mut ClientView::new(main.clone()),
            OffsetDateTime::now_utc(),
        )
        .unwrap()
        .model
        .development_target;
    assert_eq!(sql_rows(&main), before);
    support::git(
        repo.path(),
        [
            "worktree",
            "move",
            old_path.to_str().unwrap(),
            new_path.to_str().unwrap(),
        ],
    );
    let moved = workspace(&new_path);
    assert_eq!(moved.git_dir, old.git_dir);
    assert_eq!(moved.head, old.head);
    assert_eq!(moved.branch, old.branch);
    // Reoccupy before the application's first query after the move.
    if foreign {
        let other = support::committed_repo();
        support::git(
            external.path(),
            [
                "clone",
                "--no-hardlinks",
                other.path().to_str().unwrap(),
                old_path.to_str().unwrap(),
            ],
        );
    } else {
        support::git(
            repo.path(),
            [
                "worktree",
                "add",
                "-b",
                "anchor-occupant",
                old_path.to_str().unwrap(),
            ],
        );
    }
    let occupant = workspace(&old_path);
    if !foreign {
        support::git(
            &occupant.root,
            [
                "config",
                "--worktree",
                "devmap.developmentTarget",
                "target-two",
            ],
        );
        assert_eq!(
            support::git(&moved.root, ["config", "--get", "devmap.developmentTarget"]),
            "target-one"
        );
        assert_eq!(
            support::git(
                &occupant.root,
                ["config", "--get", "devmap.developmentTarget"]
            ),
            "target-two"
        );
        for target in ["refs/heads/target-one", "refs/heads/target-two"] {
            assert_eq!(support::git(&main.root, ["rev-parse", target]), main.head);
        }
    }

    assert_ne!(occupant.git_dir, old.git_dir);
    if foreign {
        assert_ne!(occupant.git_common_dir, main.git_common_dir);
    } else {
        assert_eq!(occupant.git_common_dir, main.git_common_dir);
    }
    // Freeze actual Git administrative bytes after the requested Git operations.
    // Only reads follow; no implicit repair or writes to the occupant are allowed.
    let main_head = fs::read(main.git_dir.join("HEAD")).unwrap();
    let main_config = fs::read(main.git_common_dir.join("config")).unwrap();
    let refs = backup_tree(&main.git_common_dir.join("refs"));
    let moved_head = fs::read(moved.git_dir.join("HEAD")).unwrap();
    let backlink = fs::read(moved.git_dir.join("gitdir")).unwrap();
    let foreign_tree = foreign.then(|| backup_tree(&occupant.git_dir));
    let scoped_configs = (!foreign).then(|| {
        (
            fs::read(moved.git_dir.join("config.worktree")).unwrap(),
            fs::read(occupant.git_dir.join("config.worktree")).unwrap(),
        )
    });

    let mut main_view = ClientView::new(main.clone());
    let result = app.query(&mut main_view, OffsetDateTime::now_utc());
    assert_eq!(
        sql_rows(&main),
        before,
        "failed or successful reanchor is read-only"
    );
    assert_eq!(backup_tree(&backup), frozen);
    assert_eq!(fs::read(&legacy_path).unwrap(), legacy_bytes);
    assert_eq!(fs::read(main.git_dir.join("HEAD")).unwrap(), main_head);
    assert_eq!(
        fs::read(main.git_common_dir.join("config")).unwrap(),
        main_config
    );
    assert_eq!(backup_tree(&main.git_common_dir.join("refs")), refs);
    assert_eq!(fs::read(moved.git_dir.join("HEAD")).unwrap(), moved_head);
    assert_eq!(fs::read(moved.git_dir.join("gitdir")).unwrap(), backlink);
    if let Some(bytes) = &foreign_tree {
        assert_eq!(&backup_tree(&occupant.git_dir), bytes);
    }
    if let Some((original, occupant_config)) = &scoped_configs {
        assert_eq!(
            &fs::read(moved.git_dir.join("config.worktree")).unwrap(),
            original
        );
        assert_eq!(
            &fs::read(occupant.git_dir.join("config.worktree")).unwrap(),
            occupant_config
        );
    }
    let next = result.expect(
        "same application must resolve the original moved anchor despite old-path reoccupation",
    );
    assert_eq!(
        next.model.development_target, main_target,
        "main client retains its own effective target after anchor relocation"
    );
    assert_eq!(next.store_generation, warm.store_generation);
    assert!(
        next.git_cycle > warm.git_cycle,
        "same-generation warm topology must refresh"
    );
    assert!(
        next.model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == main_id && lane.is_current)
    );
    let moved_lane = next
        .model
        .lanes
        .iter()
        .find(|lane| lane.worktree_id == old_id)
        .unwrap();
    assert_eq!(moved_lane.workspace_path, moved.root.to_string_lossy());
    assert!(
        moved_lane
            .chats
            .iter()
            .any(|chat| chat.session_id == "moving-anchor-history")
    );
    for lane in next
        .model
        .lanes
        .iter()
        .filter(|lane| lane.worktree_id != old_id)
    {
        assert!(
            !lane
                .chats
                .iter()
                .any(|chat| chat.session_id == "moving-anchor-history")
        );
    }
    let mut moved_view = ClientView::new(moved.clone());
    let fresh = app
        .query(&mut moved_view, OffsetDateTime::now_utc())
        .unwrap();
    assert!(
        fresh
            .model
            .lanes
            .iter()
            .any(|lane| lane.worktree_id == old_id
                && lane.is_current
                && lane.workspace_path == moved.root.to_string_lossy())
    );
    if !foreign {
        assert_eq!(
            fresh.model.development_target.as_ref().unwrap().ref_name,
            "refs/heads/target-one"
        );
    }
    assert!(
        app.query(&mut old_view, OffsetDateTime::now_utc()).is_err(),
        "stale authenticated source must not bind to the occupant"
    );
    assert_eq!(sql_rows(&main), before);
    assert_eq!(backup_tree(&backup), frozen);
    assert_eq!(fs::read(&legacy_path).unwrap(), legacy_bytes);
    assert_eq!(workspace(&new_path).head, moved.head);
    assert_eq!(workspace(&new_path).branch, moved.branch);
    assert_eq!(fs::read(main.git_dir.join("HEAD")).unwrap(), main_head);
    assert_eq!(
        fs::read(main.git_common_dir.join("config")).unwrap(),
        main_config
    );
    assert_eq!(backup_tree(&main.git_common_dir.join("refs")), refs);
    assert_eq!(fs::read(moved.git_dir.join("HEAD")).unwrap(), moved_head);
    assert_eq!(fs::read(moved.git_dir.join("gitdir")).unwrap(), backlink);
    if let Some((original, occupant_config)) = scoped_configs {
        assert_eq!(
            fs::read(moved.git_dir.join("config.worktree")).unwrap(),
            original
        );
        assert_eq!(
            fs::read(occupant.git_dir.join("config.worktree")).unwrap(),
            occupant_config
        );
    }
    if let Some(bytes) = foreign_tree {
        assert_eq!(backup_tree(&occupant.git_dir), bytes);
    }
}

#[test]
fn moved_anchor_with_same_repository_old_path_occupant_uses_original_identity() {
    move_anchor_reoccupation(false);
}
#[test]
fn moved_anchor_with_foreign_old_path_occupant_uses_original_identity() {
    move_anchor_reoccupation(true);
}
