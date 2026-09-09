mod support;
use devmap::{
    application::{ClientView, RepositoryApplication},
    git::{SourceGitInspector, SourceWorkspace},
};
use time::OffsetDateTime;
fn workspace(path: &std::path::Path) -> SourceWorkspace {
    SourceGitInspector::open(path)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap()
}
#[test]
fn missing_registered_worktree_is_not_a_current_execution_report() {
    let repo = support::committed_repo();
    let directory = tempfile::tempdir().unwrap();
    let linked = directory.path().join("linked");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "missing-report",
            linked.to_str().unwrap(),
        ],
    );
    let w = workspace(repo.path());
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    // Move only this test's owned directory; retain Git's registered old path.
    std::fs::rename(&linked, directory.path().join("moved")).unwrap();
    let worktrees = devmap::worktrees::WorktreeScanner::scan(&w).unwrap();
    let registered = worktrees
        .iter()
        .find(|row| row.root == linked && row.is_prunable)
        .unwrap();
    let now = OffsetDateTime::now_utc();
    let mut row = task(&w, "missing-execution-report", now);
    row.working_directory = Some(devmap::dock::WorkingDirectoryObservation {
        path: registered.root.to_string_lossy().into(),
        observed_at: now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
        source: "agent_report".into(),
    });
    let result = app.accept_inventory(&mut view, vec![row], false, now);
    assert!(
        matches!(
            result,
            Err(devmap::error::DevMapError::InvalidDomain(
                "codex_tasks.workingDirectory"
            ))
        ),
        "{result:?}"
    );
    assert!(view.observed_tasks().is_empty());
    assert!(view.inventory_observed_at().is_none());
}
#[test]
fn four_views_share_git_but_keep_current_workspace_and_revisions() {
    let repo = support::committed_repo();
    let temp = tempfile::tempdir().unwrap();
    let linked = temp.path().join("linked");
    support::git(
        repo.path(),
        ["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let w = workspace(repo.path());
    let mut app = RepositoryApplication::open(&w)
        .unwrap()
        .with_git_max_age(std::time::Duration::from_secs(60))
        .unwrap();
    let mut views = [
        ClientView::new(w.clone()),
        ClientView::new(workspace(&linked)),
        ClientView::new(w.clone()),
        ClientView::new(workspace(&linked)),
    ];
    let now = OffsetDateTime::now_utc();
    let mut currents = Vec::new();
    for view in &mut views {
        let result = app.query(view, now).unwrap();
        currents.push(result.model.current_worktree_id.clone());
        assert_eq!(result.git_cycle, 1);
    }
    assert_ne!(currents[0], currents[1]);
    assert_eq!(currents[0], currents[2]);
    assert_eq!(currents[1], currents[3]);
    assert!(!w.git_common_dir.join("devmap").exists());
    let first = app.query(&mut views[0], now).unwrap();
    assert_eq!(first.git_cycle, 1);
    let cached = app
        .query(&mut views[0], now + time::Duration::seconds(1))
        .unwrap();
    assert_eq!(cached.git_observed_at, first.git_observed_at);
    assert!(!cached.model.workspace_facts.is_empty());
    for facts in &cached.model.workspace_facts {
        assert_eq!(
            facts.git_observed_at.as_deref(),
            Some(first.git_observed_at.as_str()),
            "public cached Git facts must retain their actual observation time"
        );
    }
    app.change_hint();
    assert_eq!(app.query(&mut views[0], now).unwrap().git_cycle, 2);
    let mut restarted = RepositoryApplication::open(&w).unwrap();
    let second = restarted.query(&mut views[0], now).unwrap();
    assert!(second.model.observation_revision > first.model.observation_revision);
    assert_eq!(second.model.revision, first.model.revision);
}
fn task(w: &SourceWorkspace, id: &str, time: OffsetDateTime) -> devmap::dock::ObservedTask {
    devmap::dock::ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: id.into(),
        display_title: id.into(),
        host: "codex".into(),
        host_status: "running".into(),
        workspace_path: w.root.to_string_lossy().into(),
        status: devmap::presence::PresenceStatus::Working,
        updated_at: time
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
    }
}
#[test]
fn partial_late_inventory_and_plain_refresh_preserve_source_watermarks() {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    let backup = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&w, &backup.path().join("frozen")).unwrap();
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    let now = OffsetDateTime::now_utc();
    app.accept_inventory(
        &mut view,
        vec![task(&w, "a", now), task(&w, "b", now)],
        true,
        now,
    )
    .unwrap();
    let generation = devmap::store::RepositoryStore::open_existing(&w)
        .unwrap()
        .unwrap()
        .generation()
        .unwrap();
    let first = app.query(&mut view, now).unwrap();
    let refresh = app
        .query(&mut view, now + time::Duration::seconds(10))
        .unwrap();
    assert_eq!(first.store_generation, Some(generation));
    assert_eq!(refresh.store_generation, Some(generation));
    assert_eq!(
        first.store_inputs_observed_at,
        refresh.store_inputs_observed_at
    );
    assert_eq!(
        first.model.task_inventory_synced_at,
        refresh.model.task_inventory_synced_at
    );
    app.accept_inventory(&mut view, vec![], true, now - time::Duration::seconds(1))
        .unwrap();
    assert_eq!(view.observed_tasks().len(), 2);
    let mut late = task(&w, "a", now - time::Duration::seconds(5));
    late.display_title = "old title".into();
    app.accept_inventory(
        &mut view,
        vec![late],
        false,
        now + time::Duration::seconds(1),
    )
    .unwrap();
    assert_eq!(view.observed_tasks().len(), 2);
    assert!(!view.inventory_complete());
    assert_eq!(view.observed_tasks()[0].display_title, "a");
    assert_eq!(
        devmap::store::RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        generation + 1
    );
}
#[test]
fn malformed_legacy_domains_warn_without_repairs() {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    std::fs::create_dir_all(w.git_common_dir.join("devmap/presence/v1")).unwrap();
    std::fs::write(w.git_common_dir.join("devmap/presence/v1/bad.json"), b"{").unwrap();
    std::fs::write(w.git_common_dir.join("devmap/route-plans.jsonl"), b"{").unwrap();
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    let model = app
        .query(&mut view, OffsetDateTime::now_utc())
        .unwrap()
        .model;
    assert!(
        model
            .warnings
            .iter()
            .any(|w| w.code == "presence_record_invalid")
    );
    assert!(
        model
            .warnings
            .iter()
            .any(|w| w.code == "route_plans_unavailable")
    );
    assert_eq!(
        std::fs::read(w.git_common_dir.join("devmap/route-plans.jsonl")).unwrap(),
        b"{"
    );
    assert!(!w.git_common_dir.join("devmap/state.sqlite3").exists());
}

#[test]
fn proxy_owned_counters_and_history_survive_serialized_owner_restart() {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    let old = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(
        repo.path(),
        [
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "next",
        ],
    );
    let mut owner = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    let now = OffsetDateTime::now_utc();
    let before = owner.query(&mut view, now).unwrap();
    support::git(repo.path(), ["reset", "--hard", old.trim()]);
    owner.change_hint();
    let changed = owner.query(&mut view, now).unwrap();
    assert!(
        changed
            .model
            .warnings
            .iter()
            .any(|w| w.code == "workspace_history_changed")
    );
    let wire = serde_json::to_vec(&view.query_input().unwrap()).unwrap();
    let request = serde_json::from_slice(&wire).unwrap();
    let mut restarted = RepositoryApplication::open(&w).unwrap();
    let raw = restarted.project(&w, &request, now).unwrap();
    let result = view
        .apply_projection(serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap())
        .unwrap();
    assert!(result.model.observation_revision > changed.model.observation_revision);
    assert!(result.model.revision > before.model.revision);
    assert!(
        result
            .model
            .warnings
            .iter()
            .any(|w| w.code == "workspace_history_changed")
    );
    assert_eq!(view.inventory_observed_at(), None);
}
#[test]
fn active_errors_never_fall_back_to_legacy() {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    let backup = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&w, &backup.path().join("frozen")).unwrap();
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    app.query(&mut view, OffsetDateTime::now_utc()).unwrap();
    std::fs::write(w.git_common_dir.join("devmap/route-plans.jsonl"), b"{\n").unwrap();
    assert!(app.query(&mut view, OffsetDateTime::now_utc()).is_err());
    assert_eq!(
        std::fs::read(w.git_common_dir.join("devmap/route-plans.jsonl")).unwrap(),
        b"{\n"
    );
}
fn binding_state(w: &SourceWorkspace) -> (u64, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let store = devmap::store::RepositoryStore::open_existing(w)
        .unwrap()
        .unwrap();
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let rows = |sql: &str| {
        connection
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    (
        store.generation().unwrap(),
        rows("SELECT record_json FROM binding_records ORDER BY observation_id"),
        rows("SELECT record_json FROM binding_watermarks ORDER BY source_scope"),
        rows(
            "SELECT json_array(observation_id,destination_worktree_id,destination_incarnation,destination_qualification,source_worktree_id,source_incarnation,source_qualification) FROM binding_origin_links ORDER BY observation_id",
        ),
        rows(
            "SELECT json_array(source_scope,observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id) FROM binding_origin_cursors ORDER BY source_scope",
        ),
    )
}
fn rejected_partial_preserves_all_state(large_titles: bool) {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    let backup = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&w, &backup.path().join("frozen")).unwrap();
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    let now = OffsetDateTime::now_utc();
    app.accept_inventory(&mut view, vec![task(&w, "baseline", now)], true, now)
        .unwrap();
    app.query(&mut view, now).unwrap();
    let mut prior = view.query_input().unwrap();
    let mut incoming = task(&w, "new-task", now);
    if large_titles {
        let mut old = task(&w, "old-task", now);
        old.display_title = "x".repeat(1100 * 1024);
        incoming.display_title = "y".repeat(1100 * 1024);
        prior.tasks = vec![old];
    } else {
        prior.tasks = (0..2048)
            .map(|i| task(&w, &format!("old-{i}"), now))
            .collect();
    }
    view.apply_inventory(prior).unwrap();
    let before_bindings = binding_state(&w);
    let before_view = serde_json::to_value(view.query_input().unwrap()).unwrap();
    let before_model = serde_json::to_value(view.snapshot()).unwrap();
    let result = app.accept_inventory(
        &mut view,
        vec![incoming],
        false,
        now + time::Duration::seconds(1),
    );
    assert!(
        result.is_err(),
        "the merged partial inventory must be rejected before accepting bindings"
    );
    assert_eq!(
        binding_state(&w),
        before_bindings,
        "generation, binding records and watermarks must be unchanged"
    );
    assert_eq!(
        serde_json::to_value(view.query_input().unwrap()).unwrap(),
        before_view
    );
    assert_eq!(serde_json::to_value(view.snapshot()).unwrap(), before_model);
}
#[test]
fn partial_inventory_count_overflow_is_atomic() {
    rejected_partial_preserves_all_state(false);
}
#[test]
fn partial_inventory_byte_overflow_is_atomic() {
    rejected_partial_preserves_all_state(true);
}

fn projection_behavior(model: &devmap::dock::DockReadModel) -> serde_json::Value {
    let mut value = serde_json::to_value(model).unwrap();
    value.as_object_mut().unwrap().remove("revision");
    value
        .as_object_mut()
        .unwrap()
        .remove("observation_revision");
    // These independent collectors observe at different actual times. Exact
    // retained Git timestamps are checked by the shared-cycle test above.
    value.as_object_mut().unwrap().remove("generated_at");
    for facts in value["workspace_facts"].as_array_mut().unwrap() {
        facts.as_object_mut().unwrap().remove("git_observed_at");
    }
    value
}
#[test]
fn worktree_specific_targets_match_dock_service_for_either_owner_and_after_reconcile() {
    let repo = support::committed_repo();
    let temp = tempfile::tempdir().unwrap();
    let linked = temp.path().join("linked");
    support::git(repo.path(), ["branch", "target-a"]);
    support::git(repo.path(), ["branch", "target-b"]);
    support::git(
        repo.path(),
        ["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    support::git(repo.path(), ["config", "extensions.worktreeConfig", "true"]);
    support::git(
        repo.path(),
        [
            "config",
            "--worktree",
            "devmap.developmentTarget",
            "target-a",
        ],
    );
    support::git(
        &linked,
        [
            "config",
            "--worktree",
            "devmap.developmentTarget",
            "target-b",
        ],
    );
    let main = workspace(repo.path());
    let child = workspace(&linked);
    let now = OffsetDateTime::now_utc();
    for owner in [&main, &child] {
        let mut app = RepositoryApplication::open(owner)
            .unwrap()
            .with_git_max_age(std::time::Duration::from_secs(60))
            .unwrap();
        for (source, expected) in [(&main, "target-a"), (&child, "target-b")] {
            let mut view = ClientView::new(source.clone());
            let actual = app.query(&mut view, now).unwrap().model;
            let mut direct = devmap::dock::DockService::open(&source.root).unwrap();
            let expected_model = direct.refresh(now).unwrap();
            assert_eq!(actual.development_target.as_ref().unwrap().name, expected);
            assert_eq!(
                projection_behavior(&actual),
                projection_behavior(expected_model)
            );
        }
        support::git(
            &linked,
            [
                "config",
                "--worktree",
                "devmap.developmentTarget",
                "target-a",
            ],
        );
        app.reconcile();
        let mut view = ClientView::new(child.clone());
        let actual = app.query(&mut view, now).unwrap().model;
        let mut direct = devmap::dock::DockService::open(&linked).unwrap();
        assert_eq!(actual.development_target.as_ref().unwrap().name, "target-a");
        assert_eq!(
            projection_behavior(&actual),
            projection_behavior(direct.refresh(now).unwrap())
        );
        support::git(
            &linked,
            [
                "config",
                "--worktree",
                "devmap.developmentTarget",
                "target-b",
            ],
        );
    }
}
#[test]
fn inventory_ipc_acceptance_retains_original_previous_heads() {
    let repo = support::committed_repo();
    let w = workspace(repo.path());
    let mut app = RepositoryApplication::open(&w).unwrap();
    let mut view = ClientView::new(w.clone());
    let now = OffsetDateTime::now_utc();
    app.query(&mut view, now).unwrap();
    let prior = view.query_input().unwrap();
    assert!(!prior.previous_heads.is_empty());
    let expected = serde_json::to_value(&prior.previous_heads).unwrap();
    let accepted = app
        .accept_inventory_query(&w, prior, vec![task(&w, "new-task", now)], false, now)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&accepted.previous_heads).unwrap(),
        expected
    );
    assert_eq!(accepted.tasks.len(), 1);
    view.apply_inventory(accepted).unwrap();
    assert_eq!(
        serde_json::to_value(view.query_input().unwrap().previous_heads).unwrap(),
        expected
    );
}
