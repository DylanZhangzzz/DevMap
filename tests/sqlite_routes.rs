mod support;
use devmap::{
    git::SourceGitInspector,
    route_plan::{PlanInput, RoutePlanStore},
    store::RepositoryStore,
    worktrees::WorktreeScanner,
};
#[test]
fn sql_routes_keep_retry_start_and_cas() {
    let repo = support::committed_repo();
    let w = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let s = RoutePlanStore::open(&w).unwrap();
    let mut i = PlanInput {
        delivery: Default::default(),
        request_id: "a".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: WorktreeScanner::scan(&w).unwrap()[0].worktree_id.clone(),
        goal: "goal".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let p = s.set(i.clone()).unwrap();
    assert_eq!(s.set(i.clone()).unwrap(), p);
    assert_eq!(db.generation().unwrap(), 1);
    i.goal = "changed".into();
    assert!(s.set(i.clone()).is_err());
    i.request_id = "b".into();
    i.route_id = Some(p.route_id.clone());
    i.expected_revision = 1;
    let p2 = s.set(i.clone()).unwrap();
    assert_eq!(p.start_commit, p2.start_commit);
    i.request_id = "c".into();
    assert!(s.set(i).is_err());
    assert_eq!(s.list().unwrap(), vec![p2]);
    assert!(!w.git_common_dir.join("devmap/route-plans.jsonl").exists());
}

#[test]
fn concurrent_sql_cas_has_one_winner_and_original_retry_survives_updates() {
    let repo = support::committed_repo();
    let w = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    rusqlite::Connection::open(db.path())
        .unwrap()
        .execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let s = RoutePlanStore::open(&w).unwrap();
    let i = PlanInput {
        delivery: Default::default(),
        request_id: "initial".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: WorktreeScanner::scan(&w).unwrap()[0].worktree_id.clone(),
        goal: "goal".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let original = s.set(i.clone()).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|n| {
            let w = w.clone();
            let mut next = i.clone();
            next.route_id = Some(original.route_id.clone());
            next.expected_revision = 1;
            next.request_id = format!("update-{n}");
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                RoutePlanStore::open(&w).unwrap().set(next)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results.iter().any(|r| matches!(
        r,
        Err(devmap::error::DevMapError::RoutePlanConflict {
            revision: 2,
            current_plan: Some(_)
        })
    )));
    assert_eq!(s.set(i).unwrap(), original);
    assert_eq!(db.generation().unwrap(), 2);
}

#[test]
fn imported_routes_are_byte_equivalent_and_shadow_is_legacy() {
    let repo = support::committed_repo();
    let w = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let s = RoutePlanStore::open(&w).unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    let mut i = PlanInput {
        delivery: Default::default(),
        request_id: "initial".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: WorktreeScanner::scan(&w).unwrap()[0].worktree_id.clone(),
        goal: "goal".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let original = s.set(i.clone()).unwrap();
    i.route_id = Some(original.route_id);
    i.request_id = "update".into();
    i.expected_revision = 1;
    i.abandoned = true;
    s.set(i).unwrap();
    let before = serde_json::to_vec(&s.list().unwrap()).unwrap();
    let c = rusqlite::Connection::open(db.path()).unwrap();
    assert_eq!(db.generation().unwrap(), 0);
    for line in std::fs::read_to_string(w.git_common_dir.join("devmap/route-plans.jsonl"))
        .unwrap()
        .lines()
    {
        let r: serde_json::Value = serde_json::from_str(line).unwrap();
        c.execute(
            "INSERT INTO route_records VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![
                r["plan"]["route_id"].as_str().unwrap(),
                r["plan"]["revision"].as_i64().unwrap(),
                r["input"]["request_id"].as_str().unwrap(),
                r["input"].to_string(),
                r["plan"].to_string()
            ],
        )
        .unwrap();
    }
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    assert_eq!(before, serde_json::to_vec(&s.list().unwrap()).unwrap());
    c.execute("UPDATE route_records SET plan_json=json_set(plan_json,'$.start_commit',?1) WHERE revision=2",["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]).unwrap();
    assert!(s.list().is_err());
}

fn setup() -> (tempfile::TempDir, RoutePlanStore, PlanInput) {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let worktree = WorktreeScanner::scan(&workspace)
        .unwrap()
        .into_iter()
        .find(|w| w.is_current)
        .unwrap();
    let db = RepositoryStore::open(&workspace).unwrap();
    rusqlite::Connection::open(db.path())
        .unwrap()
        .execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let store = RoutePlanStore::open(&workspace).unwrap();
    let input = PlanInput {
        delivery: Default::default(),
        request_id: "request-1".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: worktree.worktree_id,
        goal: "Improve login".into(),
        target_ref: Some("refs/heads/main".into()),
        milestones: vec!["Verify login".into()],
        source: "User requested login improvements".into(),
        abandoned: false,
    };
    (repo, store, input)
}

#[test]
fn invalid_targets_and_unknown_workspaces_never_write_plans() {
    let (_repo, store, mut input) = setup();
    input.target_ref = Some("refs/heads/../outside".into());
    assert!(store.set(input.clone()).is_err());
    input.target_ref = None;
    input.worktree_id = "unknown".into();
    assert!(store.set(input).is_err());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn removed_workspace_does_not_prevent_abandoning_existing_intent() {
    let (repo, store, mut input) = setup();
    let linked = repo.path().join("linked");
    support::git(
        repo.path(),
        ["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    input.worktree_id = WorktreeScanner::scan(&workspace)
        .unwrap()
        .into_iter()
        .find(|w| !w.is_current)
        .unwrap()
        .worktree_id;
    let first = store.set(input.clone()).unwrap();
    support::git(
        repo.path(),
        ["worktree", "remove", linked.to_str().unwrap()],
    );
    input.route_id = Some(first.route_id);
    input.request_id = "abandon".into();
    input.expected_revision = 1;
    input.abandoned = true;
    assert!(store.set(input).unwrap().abandoned);
}
