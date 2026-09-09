mod support;

use devmap::{
    dock::{DockService, ObservedTask, TaskLifecycle},
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::JournalStore,
    presence::PresenceStatus,
    route_plan::{PlanInput, RoutePlanStore},
    store::RepositoryStore,
    worktrees::WorktreeScanner,
};
use rusqlite::Connection;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const SESSION: &str = "01a00000-0000-7000-8000-000000000031";
fn workspace(path: &Path) -> SourceWorkspace {
    SourceGitInspector::open(path).unwrap().workspace().unwrap()
}
// Public OS identity observation only. No private production test API is opened.
#[cfg(windows)]
fn directory_identity(path: &Path) -> String {
    use std::{
        fs::OpenOptions,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        GetFileInformationByHandle,
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    assert!(file.metadata().unwrap().is_dir());
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the File owns a live handle; info provides writable output storage.
    assert_ne!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    // SAFETY: successful GetFileInformationByHandle initializes the output.
    let info = unsafe { info.assume_init() };
    format!(
        "windows:{}:{}",
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
    )
}
#[cfg(unix)]
fn directory_identity(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    format!("unix:{}:{}", metadata.dev(), metadata.ino())
}
fn incarnation(w: &SourceWorkspace) -> String {
    format!(
        "{}|{}",
        directory_identity(&w.git_dir),
        directory_identity(&w.root)
    )
}
fn event(w: &SourceWorkspace, sequence: u64, id: &str) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        id,
        EventType::SessionStarted,
        sequence,
        "2026-09-09T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("actor", None).unwrap(),
        SessionContext::new(
            SESSION,
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
fn snapshot(c: &Connection) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let tx = c.unchecked_transaction().unwrap();
    let names=tx.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let expected = [
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
    assert_eq!(
        names.iter().cloned().collect::<BTreeSet<_>>(),
        expected.into_iter().map(str::to_owned).collect()
    );
    let result = names
        .into_iter()
        .map(|name| {
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            let count = tx
                .prepare(&format!("SELECT * FROM {quoted}"))
                .unwrap()
                .column_count();
            let order = (1..=count)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let rows = tx
                .prepare(&format!("SELECT * FROM {quoted} ORDER BY {order}"))
                .unwrap()
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect();
    tx.commit().unwrap();
    result
}

#[test]
fn real_native_worktree_move_keeps_identity_and_new_path_replays_old_session() {
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let old_path = external.path().join("a");
    let new_path = external.path().join("a-new");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "move-native",
            old_path.to_str().unwrap(),
        ],
    );
    let main = workspace(repo.path());
    let old = workspace(&old_path);
    let old_id = WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|r| r.root == old.root)
        .unwrap()
        .worktree_id;
    let before_incarnation = incarnation(&old);
    let db = RepositoryStore::open(&main).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let handle = JournalStore::open(&old, SESSION)
        .unwrap()
        .with_presence_projection();
    let record = handle.append(event(&old, 1, "first")).unwrap();
    RoutePlanStore::open(&main)
        .unwrap()
        .set(PlanInput {
            delivery: Default::default(),
            request_id: "move-route".into(),
            route_id: None,
            expected_revision: 0,
            worktree_id: old_id.clone(),
            goal: "retain identity through move".into(),
            target_ref: None,
            milestones: vec![],
            source: "user".into(),
            abandoned: false,
        })
        .unwrap();
    let mut dock = DockService::open(&main.root).unwrap();
    dock.replace_observed_tasks(
        vec![ObservedTask {
            working_directory: None,
            subagents: None,
            lifecycle: TaskLifecycle::Present,
            session_id: SESSION.into(),
            display_title: "moving task".into(),
            host: "local".into(),
            host_status: "active".into(),
            workspace_path: old.root.to_string_lossy().into_owned(),
            status: PresenceStatus::Working,
            updated_at: "2026-09-09T10:00:00Z".into(),
        }],
        time::OffsetDateTime::from_unix_timestamp(1788948000).unwrap(),
    )
    .unwrap();
    drop(dock);
    let registered: (String, String) = c
        .query_row(
            "SELECT incarnation,workspace_path FROM worktree_registry WHERE worktree_id=?1",
            [&old_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(registered.0, before_incarnation);
    assert_eq!(registered.1, old.root.to_string_lossy());
    for table in [
        "journal_records",
        "presence_records",
        "route_origin_links",
        "binding_origin_links",
        "binding_origin_cursors",
    ] {
        let count: i64 = c
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "{table} fixture evidence");
    }
    let before = snapshot(&c);
    // A real Git move, not a copy/recreated root. Both targets are owned fixture paths.
    support::git(
        repo.path(),
        [
            "worktree",
            "move",
            old_path.to_str().unwrap(),
            new_path.to_str().unwrap(),
        ],
    );
    assert!(!old_path.exists());
    let moved = workspace(&new_path);
    let moved_id = WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|r| r.root == moved.root)
        .unwrap()
        .worktree_id;
    assert_eq!(moved_id, old_id);
    assert_eq!(moved.git_dir, old.git_dir);
    assert_eq!(
        incarnation(&moved),
        before_incarnation,
        "filesystem must preserve both physical identities for this move case"
    );
    assert!(
        handle.replay().is_err(),
        "old-path handle must not silently follow move"
    );
    let built = std::cell::Cell::new(false);
    assert!(
        handle
            .append_batch_with(|n| {
                built.set(true);
                Ok(vec![event(&old, n, "stale")])
            })
            .is_err()
    );
    assert!(!built.get());
    let replay = JournalStore::open(&moved, SESSION).and_then(|j| j.replay());
    // Check read/refusal purity even on the expected pre-fix failure path.
    assert_eq!(
        snapshot(&c),
        before,
        "move/read must not rewrite historical SQL identity or JSON"
    );
    let root: String = c
        .query_row(
            "SELECT workspace_path FROM worktree_registry WHERE worktree_id=?1",
            [&old_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(root, registered.1);
    assert_eq!(
        replay.expect(
            "verified real move must permit a fresh new-path handle to replay the original session"
        ),
        vec![record.clone()]
    );

    let resumed = JournalStore::open(&moved, SESSION)
        .unwrap()
        .with_presence_projection();
    let before_retry = snapshot(&c);
    let repeated = resumed
        .append_capture_batch_with(time::OffsetDateTime::now_utc(), |_| {
            Ok(vec![record.event.clone()])
        })
        .unwrap();
    assert_eq!(repeated, vec![record.clone()]);
    assert_eq!(
        snapshot(&c),
        before_retry,
        "exact old-event retry must preserve all fourteen tables"
    );

    let generation = db.generation().unwrap();
    let second = resumed
        .append_capture_batch_with(time::OffsetDateTime::now_utc(), |sequence| {
            assert_eq!(sequence, 2);
            Ok(vec![event(&moved, sequence, "after-move")])
        })
        .unwrap()
        .remove(0);
    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_sha256, Some(record.sha256.clone()));
    assert_eq!(
        resumed.replay().unwrap(),
        vec![record.clone(), second.clone()]
    );
    assert_eq!(
        db.generation().unwrap(),
        generation + 1,
        "atomic capture advances generation once"
    );
    let coverage:(i64,String,i64,String)=c.query_row(
        "SELECT h.record_count,h.last_sha256,p.covered_sequence,p.covered_sha256 FROM journal_heads h JOIN presence_projection p USING(session_id) WHERE h.session_id=?1",
        [SESSION], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        coverage,
        (2, second.sha256.clone(), 2, second.sha256.clone())
    );
    let presence: devmap::presence::PresenceRecord = serde_json::from_str(
        &c.query_row(
            "SELECT record_json FROM presence_records WHERE session_id=?1",
            [SESSION],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(presence.session_id, SESSION);
    assert_eq!(presence.worktree_id, old_id);
    let after_capture = snapshot(&c);
    for (table, rows) in &before_retry {
        if !matches!(
            table.as_str(),
            "store_meta"
                | "journal_records"
                | "journal_heads"
                | "presence_records"
                | "presence_projection"
        ) {
            assert_eq!(&after_capture[table], rows, "capture must preserve {table}");
        }
    }
    assert_eq!(
        after_capture["journal_records"][0], before_retry["journal_records"][0],
        "old serialized record stays exact"
    );
    let registry_root: String = c
        .query_row(
            "SELECT workspace_path FROM worktree_registry WHERE worktree_id=?1",
            [&old_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        registry_root, registered.1,
        "registry path is historical evidence"
    );

    // A2 occupies the historical path while the same A1 still exists at its
    // new location. This is not recreation of A1's physical directory.
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "move-old-path-occupant",
            old_path.to_str().unwrap(),
        ],
    );
    let occupant = workspace(&old_path);
    let occupant_id = WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|r| r.root == occupant.root)
        .unwrap()
        .worktree_id;
    assert_ne!(occupant_id, old_id);
    assert_ne!(incarnation(&occupant), before_incarnation);
    assert_eq!(incarnation(&workspace(&new_path)), before_incarnation);
    let before_refusal = snapshot(&c);
    assert!(
        JournalStore::open(&occupant, SESSION)
            .and_then(|j| j.replay())
            .is_err()
    );
    let occupant_built = std::cell::Cell::new(false);
    assert!(
        JournalStore::open(&occupant, SESSION)
            .and_then(|j| j.append_batch_with(|n| {
                occupant_built.set(true);
                Ok(vec![event(&occupant, n, "occupant-cannot-inherit")])
            }))
            .is_err()
    );
    assert!(!occupant_built.get());
    assert!(handle.replay().is_err());
    let stale_built = std::cell::Cell::new(false);
    assert!(
        handle
            .append_batch_with(|n| {
                stale_built.set(true);
                Ok(vec![event(&old, n, "stale-after-reoccupation")])
            })
            .is_err()
    );
    assert!(!stale_built.get());
    assert_eq!(snapshot(&c), before_refusal);

    let mut current_dock = DockService::open(&main.root).unwrap();
    let model = current_dock
        .refresh(time::OffsetDateTime::now_utc())
        .unwrap();
    let moved_lane = model
        .lanes
        .iter()
        .find(|lane| lane.worktree_id == old_id)
        .unwrap();
    assert_eq!(moved_lane.workspace_path, moved.root.to_string_lossy());
    assert!(
        moved_lane
            .chats
            .iter()
            .any(|chat| chat.session_id == SESSION),
        "presence stays on moved A1"
    );
    let occupant_lane = model
        .lanes
        .iter()
        .find(|lane| lane.worktree_id == occupant_id)
        .unwrap();
    assert!(
        !occupant_lane
            .chats
            .iter()
            .any(|chat| chat.session_id == SESSION)
    );
    assert!(
        model
            .route_plans
            .iter()
            .any(|plan| plan.worktree_id == old_id)
    );
    assert!(
        !model
            .route_plans
            .iter()
            .any(|plan| plan.worktree_id == occupant_id)
    );
    let moved_facts = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == old_id)
        .unwrap();
    assert!(
        moved_facts
            .bindings
            .iter()
            .any(|binding| binding.task_id == SESSION && binding.worktree_id == old_id)
    );
    let occupant_facts = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == occupant_id)
        .unwrap();
    assert!(
        !occupant_facts
            .bindings
            .iter()
            .any(|binding| binding.task_id == SESSION)
    );
    assert_eq!(
        snapshot(&c),
        before_refusal,
        "projection after old-path reuse remains read-only"
    );
}

#[test]
fn moved_origin_accepts_inventory_and_route_revision_without_rewriting_history() {
    let repo = support::committed_repo();
    let files = tempfile::tempdir().unwrap();
    let old_path = files.path().join("write-old");
    let new_path = files.path().join("write-moved");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "move-write",
            old_path.to_str().unwrap(),
        ],
    );
    let main = workspace(repo.path());
    let old = workspace(&old_path);
    let id = WorktreeScanner::scan(&main)
        .unwrap()
        .into_iter()
        .find(|row| row.root == old.root)
        .unwrap()
        .worktree_id;
    let physical = incarnation(&old);
    let db = RepositoryStore::open(&main).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let input = PlanInput {
        delivery: Default::default(),
        request_id: "move-write-first".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: id.clone(),
        goal: "before move".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let first = RoutePlanStore::open(&old)
        .unwrap()
        .set(input.clone())
        .unwrap();
    let t1 = time::OffsetDateTime::from_unix_timestamp(1788948000).unwrap();
    let t2 = t1 + time::Duration::seconds(10);
    let observed = |path: &Path, at: time::OffsetDateTime| ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: TaskLifecycle::Present,
        session_id: SESSION.into(),
        display_title: "moving task".into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: path.to_string_lossy().into_owned(),
        status: PresenceStatus::Working,
        updated_at: at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
    };
    let mut initial_dock = DockService::open(&old.root).unwrap();
    initial_dock
        .replace_observed_tasks(vec![observed(&old.root, t1)], t1)
        .unwrap();
    drop(initial_dock);
    assert_eq!(db.generation().unwrap(), 2);
    let before_move = snapshot(&c);
    assert_eq!(before_move["binding_records"].len(), 1);
    assert_eq!(before_move["binding_origin_cursors"].len(), 1);
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
    assert_eq!(incarnation(&moved), physical);
    assert_eq!(moved.git_dir, old.git_dir);
    let mut moved_dock = DockService::open(&moved.root).unwrap();
    assert_eq!(
        snapshot(&c),
        before_move,
        "opening at moved origin must remain read-only"
    );
    moved_dock
        .replace_observed_tasks(vec![observed(&moved.root, t2)], t2)
        .unwrap();
    assert_eq!(
        db.generation().unwrap(),
        3,
        "new inventory watermark advances once"
    );
    let after_inventory = snapshot(&c);
    for (table, rows) in &before_move {
        if !["store_meta", "binding_watermarks", "binding_origin_cursors"].contains(&table.as_str())
        {
            assert_eq!(
                &after_inventory[table], rows,
                "same-origin inventory must preserve {table}"
            );
        }
    }
    let (cursor_at, cursor_id, cursor_inc, qualification, history): (String, String, String, String, String) = c.query_row(
        "SELECT observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id FROM binding_origin_cursors",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap();
    assert_eq!(
        cursor_at,
        t2.format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    );
    assert_eq!(cursor_id, id);
    assert_eq!(cursor_inc, physical);
    assert_eq!(qualification, "native_verified");
    let original_history: String = c
        .query_row("SELECT observation_id FROM binding_records", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(history, original_history);
    let facts = moved_dock
        .snapshot()
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == id)
        .unwrap();
    assert_eq!(
        facts.bindings.len(),
        1,
        "move must not fabricate public migration history"
    );
    assert_eq!(facts.bindings[0].task_id, SESSION);
    assert_eq!(facts.bindings[0].worktree_id, id);
    assert!(facts.bindings[0].from_worktree_id.is_none());
    moved_dock
        .replace_observed_tasks(vec![observed(&moved.root, t2)], t2)
        .unwrap();
    assert_eq!(
        snapshot(&c),
        after_inventory,
        "inventory retry is exact no-op"
    );

    let store = RoutePlanStore::open(&moved).unwrap();
    let mut revision = input.clone();
    revision.request_id = "move-write-revision".into();
    revision.route_id = Some(first.route_id.clone());
    revision.expected_revision = 1;
    revision.goal = "after move".into();
    let second = store.set(revision.clone()).unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(second.start_commit, first.start_commit);
    assert_eq!(second.worktree_id, id);
    assert_eq!(db.generation().unwrap(), 4, "route revision advances once");
    let after_route = snapshot(&c);
    for (table, rows) in &after_inventory {
        if ["route_records", "route_origin_links"].contains(&table.as_str()) {
            assert_eq!(after_route[table].len(), rows.len() + 1);
            assert!(
                rows.iter().all(|row| after_route[table].contains(row)),
                "old {table} remains exact"
            );
        } else if table != "store_meta" {
            assert_eq!(&after_route[table], rows);
        }
    }
    let identity: (String, String) = c.query_row("SELECT incarnation,qualification FROM route_origin_links WHERE route_id=?1 AND revision=2", [&second.route_id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(identity, (physical, "native_verified".into()));
    let saved_root: String = c
        .query_row(
            "SELECT workspace_path FROM worktree_registry WHERE worktree_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        saved_root,
        old.root.to_string_lossy(),
        "historical registry path must not be rewritten"
    );
    assert_eq!(
        store.set(input).unwrap(),
        first,
        "old receipt remains exact after revision and move"
    );
    assert_eq!(store.set(revision.clone()).unwrap(), second);
    revision.request_id = "move-write-stale-cas".into();
    match store.set(revision).unwrap_err() {
        devmap::error::DevMapError::RoutePlanConflict {
            revision,
            current_plan,
        } => {
            assert_eq!(revision, 2);
            assert_eq!(current_plan, Some(Box::new(second)));
        }
        error => panic!("stale CAS must retain structured conflict: {error:?}"),
    }
    assert_eq!(
        snapshot(&c),
        after_route,
        "receipts and stale CAS must preserve all 14 tables"
    );
}

// Include directories as well as exact file bytes, including the manifest.
fn frozen_inventory(root: &Path) -> BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<std::path::PathBuf, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if metadata.is_dir() {
                out.insert(relative, None);
                walk(root, &path, out);
            } else {
                out.insert(relative, Some(std::fs::read(path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn real_frozen_worktree_move_preserves_backup_and_rejects_legacy_tamper() {
    use devmap::store::migration;
    use std::io::Write;
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let old_path = external.path().join("frozen-old");
    let new_path = external.path().join("frozen-moved");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "frozen-move",
            old_path.to_str().unwrap(),
        ],
    );
    let main = workspace(repo.path());
    let old = workspace(&old_path);
    let old_incarnation = incarnation(&old);
    let first = JournalStore::open(&old, SESSION)
        .unwrap()
        .with_presence_projection()
        .append(event(&old, 1, "frozen-first"))
        .unwrap();
    let backup = external.path().join("backup");
    let manifest = migration::freeze(&main, &backup, time::OffsetDateTime::now_utc()).unwrap();
    assert_eq!(manifest.origins.len(), 2);
    let origin = manifest
        .origins
        .iter()
        .find(|o| {
            std::fs::canonicalize(&o.git_dir).unwrap()
                == std::fs::canonicalize(&old.git_dir).unwrap()
        })
        .unwrap();
    let old_id = origin.worktree_id.clone();
    let saved_root = origin.workspace_path.to_string_lossy().into_owned();
    assert_eq!(
        std::fs::canonicalize(&origin.workspace_path).unwrap(),
        std::fs::canonicalize(&old.root).unwrap()
    );
    assert_eq!(origin.incarnation, old_incarnation);
    migration::import_shadow(&main, &backup).unwrap();
    migration::activate(&main, &backup).unwrap();
    let frozen = frozen_inventory(&backup);
    assert!(!frozen.is_empty());
    let c = Connection::open_with_flags(
        main.git_common_dir.join("devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before = snapshot(&c);
    let old_handle = JournalStore::open(&old, SESSION).unwrap();
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
    assert_eq!(incarnation(&moved), old_incarnation);
    let current = JournalStore::open(&moved, SESSION)
        .unwrap()
        .with_presence_projection();
    assert_eq!(current.replay().unwrap(), vec![first.clone()]);
    assert!(old_handle.replay().is_err());
    let mut dock = DockService::open(&main.root).unwrap();
    let model = dock.refresh(time::OffsetDateTime::now_utc()).unwrap();
    assert!(
        model.lanes.iter().any(|lane| lane.worktree_id == old_id
            && lane.workspace_path == moved.root.to_string_lossy())
    );
    assert_eq!(
        snapshot(&c),
        before,
        "qualified reads preserve all 14 tables"
    );
    assert_eq!(frozen_inventory(&backup), frozen);
    let second = current.append(event(&moved, 2, "frozen-second")).unwrap();
    assert_eq!(second.sequence, 2);
    assert_eq!(
        current.replay().unwrap(),
        vec![first.clone(), second.clone()]
    );
    let after_capture = snapshot(&c);
    for (table, rows) in &before {
        if ![
            "store_meta",
            "journal_records",
            "journal_heads",
            "presence_records",
            "presence_projection",
        ]
        .contains(&table.as_str())
        {
            assert_eq!(
                &after_capture[table], rows,
                "historical {table} must not change"
            );
        }
    }
    assert_eq!(
        &after_capture["journal_records"][..1],
        &before["journal_records"]
    );
    let historical_root: String = c
        .query_row(
            "SELECT workspace_path FROM worktree_registry WHERE worktree_id=?1",
            [&old_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(historical_root, saved_root);
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "frozen-occupant",
            old_path.to_str().unwrap(),
        ],
    );
    let occupant = workspace(&old_path);
    assert_ne!(incarnation(&occupant), old_incarnation);
    assert_eq!(incarnation(&workspace(&new_path)), old_incarnation);
    let built = std::cell::Cell::new(false);
    assert!(
        JournalStore::open(&occupant, SESSION)
            .and_then(|j| j.append_batch_with(|n| {
                built.set(true);
                Ok(vec![event(&occupant, n, "cannot-inherit-frozen")])
            }))
            .is_err()
    );
    assert!(!built.get());
    assert!(old_handle.replay().is_err());
    assert_eq!(current.replay().unwrap(), vec![first, second]);
    let model = dock.refresh(time::OffsetDateTime::now_utc()).unwrap();
    let moved_lane = model
        .lanes
        .iter()
        .find(|lane| lane.worktree_id == old_id)
        .unwrap();
    assert_eq!(moved_lane.workspace_path, moved.root.to_string_lossy());
    assert!(
        moved_lane
            .chats
            .iter()
            .any(|chat| chat.session_id == SESSION)
    );
    let occupant_lane = model
        .lanes
        .iter()
        .find(|lane| lane.workspace_path == occupant.root.to_string_lossy())
        .unwrap();
    assert_ne!(occupant_lane.worktree_id, old_id);
    assert!(
        !occupant_lane
            .chats
            .iter()
            .any(|chat| chat.session_id == SESSION)
    );
    assert_eq!(snapshot(&c), after_capture);
    assert_eq!(frozen_inventory(&backup), frozen);
    // Tamper the retained original admin bytes after relocation and old-path reuse.
    let legacy_file = old
        .git_dir
        .join("devmap/sessions")
        .join(SESSION)
        .join("events.ndjson");
    std::fs::OpenOptions::new()
        .append(true)
        .open(legacy_file)
        .unwrap()
        .write_all(b" ")
        .unwrap();
    let error = current.replay().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy source inventory/hash drift"),
        "{error}"
    );
    let built = std::cell::Cell::new(false);
    assert!(
        current
            .append_batch_with(|n| {
                built.set(true);
                Ok(vec![event(&moved, n, "tampered")])
            })
            .is_err()
    );
    assert!(!built.get());
    assert_eq!(snapshot(&c), after_capture);
    assert_eq!(frozen_inventory(&backup), frozen);
}
