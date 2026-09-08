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
