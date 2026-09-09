mod support;
use devmap::{
    git::{SourceGitInspector, SourceWorkspace},
    route_plan::{PlanInput, RoutePlanStore},
    store::RepositoryStore,
    worktrees::WorktreeScanner,
};
use rusqlite::Connection;
use std::{collections::BTreeMap, path::Path};

fn workspace(path: &Path) -> SourceWorkspace {
    SourceGitInspector::open(path).unwrap().workspace().unwrap()
}
fn id(w: &SourceWorkspace, path: &Path) -> String {
    WorktreeScanner::scan(w)
        .unwrap()
        .into_iter()
        .find(|r| r.root == path)
        .unwrap()
        .worktree_id
}
fn input(target: String, request: &str) -> PlanInput {
    PlanInput {
        delivery: Default::default(),
        request_id: request.into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: target,
        goal: "intent".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    }
}
fn snapshot(c: &Connection) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let tx = c.unchecked_transaction().unwrap();
    let names=tx.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
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
fn absent_route_receipt_historical_edit_and_explicit_retarget_preserve_identity() {
    let repo = support::committed_repo();
    let external = tempfile::tempdir().unwrap();
    let a = external.path().join("a");
    let b = external.path().join("b");
    for (path, branch) in [(&a, "route-a"), (&b, "route-b")] {
        support::git(
            repo.path(),
            ["worktree", "add", "-b", branch, path.to_str().unwrap()],
        );
    }
    let w = workspace(repo.path());
    let aid = id(&w, &a);
    let bid = id(&w, &b);
    let db = RepositoryStore::open(&w).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let store = RoutePlanStore::open(&w).unwrap();
    let first_input = input(aid.clone(), "initial");
    let first = store.set(first_input.clone()).unwrap();
    let first_link:(String,String)=c.query_row("SELECT incarnation,qualification FROM route_origin_links WHERE route_id=?1 AND revision=1",[&first.route_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(first_link.1, "native_verified");
    assert_eq!(
        c.query_row("SELECT count(*) FROM journal_sessions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    support::git(repo.path(), ["worktree", "remove", a.to_str().unwrap()]);
    let before = snapshot(&c);
    assert_eq!(store.set(first_input.clone()).unwrap(), first);
    let mut changed = first_input.clone();
    changed.goal = "changed retry".into();
    assert!(store.set(changed).is_err());
    assert_eq!(snapshot(&c), before);
    let mut edit = first_input.clone();
    edit.request_id = "abandon-absent".into();
    edit.route_id = Some(first.route_id.clone());
    edit.expected_revision = 1;
    edit.abandoned = true;
    let historical = store.set(edit.clone()).unwrap();
    assert!(historical.abandoned);
    let inherited:(String,String)=c.query_row("SELECT incarnation,qualification FROM route_origin_links WHERE route_id=?1 AND revision=2",[&first.route_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(inherited, first_link);
    let mut stale = edit.clone();
    stale.request_id = "stale".into();
    let before = snapshot(&c);
    match store.set(stale) {
        Err(devmap::error::DevMapError::RoutePlanConflict {
            revision,
            current_plan,
        }) => {
            assert_eq!(revision, 2);
            assert_eq!(*current_plan.unwrap(), historical);
        }
        _ => panic!("stale revision must remain typed CAS"),
    }
    assert_eq!(snapshot(&c), before);
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "route-a-replacement",
            a.to_str().unwrap(),
        ],
    );
    assert_eq!(id(&w, &a), aid);
    edit.request_id = "same-id-historical".into();
    edit.expected_revision = 2;
    let replaced_edit = store.set(edit.clone()).unwrap();
    assert_eq!(replaced_edit.start_commit, first.start_commit);
    let inherited:(String,String)=c.query_row("SELECT incarnation,qualification FROM route_origin_links WHERE route_id=?1 AND revision=3",[&first.route_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        inherited, first_link,
        "same ID must not rebind historical intent"
    );
    edit.request_id = "retarget-b".into();
    edit.expected_revision = 3;
    edit.worktree_id = bid.clone();
    edit.abandoned = false;
    let retarget = store.set(edit).unwrap();
    assert_eq!(retarget.start_commit, first.start_commit);
    let new_id: String = c
        .query_row(
            "SELECT worktree_id FROM route_origin_links WHERE route_id=?1 AND revision=4",
            [&first.route_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new_id, bid);
    let before = snapshot(&c);
    assert_eq!(store.set(first_input).unwrap(), first);
    assert_eq!(snapshot(&c), before);
}

#[test]
fn route_origin_link_failure_rolls_back_registry_record_and_generation() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "route-rollback");
    let w = workspace(repo.path());
    let db = RepositoryStore::open(&w).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    c.execute_batch("CREATE TRIGGER reject_origin BEFORE INSERT ON route_origin_links BEGIN SELECT RAISE(ABORT,'injected origin failure'); END;").unwrap();
    let before = snapshot(&c);
    assert!(
        RoutePlanStore::open(&w)
            .unwrap()
            .set(input(id(&w, linked.path()), "reject"))
            .is_err()
    );
    assert_eq!(snapshot(&c), before);
}

#[cfg(unix)]
#[test]
fn newline_repository_and_linked_paths_preserve_route_and_binding_origin_identity() {
    use devmap::dock::{DockService, ObservedTask, TaskLifecycle};
    use devmap::presence::PresenceStatus;
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    let seed = support::committed_repo();
    let owned = tempfile::tempdir().unwrap();
    let main_path = owned.path().join("main\nrepository");
    support::git(
        owned.path(),
        [
            "clone",
            "--local",
            seed.path().to_str().unwrap(),
            main_path.to_str().unwrap(),
        ],
    );
    let linked_path = owned.path().join("linked\nworkspace");
    support::git(
        &main_path,
        [
            "worktree",
            "add",
            "-b",
            "newline-linked",
            linked_path.to_str().unwrap(),
        ],
    );
    let main = workspace(&main_path);
    let linked = workspace(&linked_path);
    assert!(main.git_common_dir.to_string_lossy().contains('\n'));
    assert!(linked.root.to_string_lossy().contains('\n'));
    let db = RepositoryStore::open(&main).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let plans = RoutePlanStore::open(&main).unwrap();
    let now = OffsetDateTime::now_utc();
    let mut dock = DockService::open(&main.root).unwrap();
    let mut tasks = Vec::new();
    for (index, target) in [&main, &linked].into_iter().enumerate() {
        let worktree_id = id(&main, &target.root);
        let plan = plans
            .set(input(
                worktree_id.clone(),
                &format!("newline-route-{index}"),
            ))
            .unwrap();
        let saved:(String,String,String) = c.query_row(
            "SELECT r.git_dir,r.workspace_path,l.qualification FROM route_origin_links l JOIN worktree_registry r USING(worktree_id,incarnation) WHERE l.route_id=?1 AND l.revision=1",
            [&plan.route_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            saved,
            (
                target
                    .git_dir
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                target.root.to_string_lossy().into_owned(),
                "native_verified".into()
            )
        );
        tasks.push(ObservedTask {
            working_directory: None,
            subagents: None,
            lifecycle: TaskLifecycle::Present,
            session_id: format!("01a00000-0000-7000-8000-00000000002{index}"),
            display_title: format!("newline task {index}"),
            host: "local".into(),
            host_status: "active".into(),
            workspace_path: target.root.to_string_lossy().into_owned(),
            status: PresenceStatus::Working,
            updated_at: now.format(&Rfc3339).unwrap(),
        });
    }
    dock.replace_observed_tasks(tasks, now).unwrap();
    let paths=c.prepare("SELECT r.workspace_path FROM binding_origin_links l JOIN worktree_registry r ON r.worktree_id=l.destination_worktree_id AND r.incarnation=l.destination_incarnation WHERE l.destination_qualification='native_verified' ORDER BY r.workspace_path")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut expected = vec![
        main.root.to_string_lossy().into_owned(),
        linked.root.to_string_lossy().into_owned(),
    ];
    expected.sort();
    assert_eq!(paths, expected);
    let before = snapshot(&c);
    let view = dock.refresh(OffsetDateTime::now_utc()).unwrap();
    assert_eq!(view.route_plans.len(), 2);
    assert_eq!(snapshot(&c), before);
}
