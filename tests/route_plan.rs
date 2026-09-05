mod support;

use devmap::git::SourceGitInspector;
use devmap::route_plan::{PlanInput, RoutePlanStore};
use devmap::worktrees::WorktreeScanner;

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
fn delivery_requires_explicit_conditions_and_can_be_revoked() {
    use devmap::route_plan::{Delivery, DeliveryMode};
    let (_repo, store, mut input) = setup();
    input.delivery = Delivery {
        mode: DeliveryMode::AutoMerge,
        conditions: vec![],
        authorization_source: None,
    };
    assert!(store.set(input.clone()).is_err());
    input.delivery.conditions = vec!["Tests pass".into()];
    assert!(store.set(input.clone()).is_err());
    input.delivery.authorization_source = Some("User asked for automatic merge".into());
    let plan = store.set(input.clone()).unwrap();
    input.route_id = Some(plan.route_id);
    input.expected_revision = 1;
    input.request_id = "revoke".into();
    input.delivery = Delivery::default();
    assert_eq!(
        store.set(input).unwrap().delivery.mode,
        DeliveryMode::Manual
    );
    assert_eq!(store.list().unwrap()[0].delivery.mode, DeliveryMode::Manual);
}

#[test]
fn old_journal_without_delivery_remains_readable() {
    let (repo, store, input) = setup();
    store.set(input).unwrap();
    let path = repo.path().join(".git/devmap/route-plans.jsonl");
    let mut record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    record["input"].as_object_mut().unwrap().remove("delivery");
    record["plan"].as_object_mut().unwrap().remove("delivery");
    std::fs::write(path, format!("{record}\n")).unwrap();
    assert_eq!(store.list().unwrap()[0].delivery, Default::default());
}

#[test]
fn plan_survives_restart_without_source_git_changes() {
    let (repo, store, input) = setup();
    let before = support::source_snapshot(repo.path());
    let created = store.set(input.clone()).unwrap();
    assert_eq!(created.revision, 1);
    assert_eq!(store.set(input).unwrap(), created);
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let reopened = RoutePlanStore::open(&workspace).unwrap();
    assert_eq!(reopened.list().unwrap(), vec![created]);
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn revisions_are_compare_and_swap_and_start_is_immutable() {
    let (_repo, store, mut input) = setup();
    let first = store.set(input.clone()).unwrap();
    input.route_id = Some(first.route_id.clone());
    input.request_id = "request-2".into();
    input.expected_revision = 1;
    input.goal = "Updated goal".into();
    let second = store.set(input.clone()).unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(second.start_commit, first.start_commit);
    assert_eq!(second.route_id, first.route_id);
    input.request_id = "stale-request".into();
    assert!(
        store
            .set(input)
            .unwrap_err()
            .to_string()
            .contains("revision")
    );
    assert_eq!(store.list().unwrap(), vec![second]);
}

#[test]
fn retry_identity_cannot_be_reused_for_different_content() {
    let (_repo, store, mut input) = setup();
    store.set(input.clone()).unwrap();
    input.goal = "Unrelated goal".into();
    assert!(store.set(input).is_err());
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

#[test]
fn unborn_branch_cannot_provide_a_route_start() {
    let (repo, store, input) = setup();
    support::git(repo.path(), ["checkout", "--orphan", "empty"]);
    assert!(store.set(input).is_err());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn corrupt_record_cannot_rewrite_the_read_model() {
    let (repo, store, input) = setup();
    store.set(input).unwrap();
    let path = repo.path().join(".git/devmap/route-plans.jsonl");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["plan"]["goal"] = serde_json::json!("Forged result differs from input");
    std::fs::write(&path, format!("{record}\n")).unwrap();
    assert!(store.list().is_err());
}

#[test]
fn competing_updates_have_one_winner() {
    let (repo, store, mut input) = setup();
    let first = store.set(input.clone()).unwrap();
    input.route_id = Some(first.route_id);
    input.expected_revision = 1;
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let handles: Vec<_> = (0..2)
        .map(|n| {
            let workspace = workspace.clone();
            let mut input = input.clone();
            input.request_id = format!("race-{n}");
            std::thread::spawn(move || RoutePlanStore::open(&workspace).unwrap().set(input).is_ok())
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(store.list().unwrap()[0].revision, 2);
}
