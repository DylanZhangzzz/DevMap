mod support;

use devmap::{git::SourceGitInspector, proxy::QueryProxy};
use time::OffsetDateTime;

fn task(workspace: &std::path::Path, observed_at: OffsetDateTime) -> devmap::dock::ObservedTask {
    devmap::dock::ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: "proxy-task".into(),
        display_title: "Original title".into(),
        host: "codex".into(),
        host_status: "running".into(),
        workspace_path: workspace.to_string_lossy().into(),
        status: devmap::presence::PresenceStatus::Working,
        updated_at: observed_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
    }
}

#[test]
fn direct_proxy_reads_without_creating_store_and_retains_observation_sequence() {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let mut proxy = QueryProxy::open_direct(repo.path()).unwrap();
    let first = proxy.snapshot().clone();
    let next = proxy.refresh(OffsetDateTime::now_utc()).unwrap();
    assert_eq!(first.repository_id, next.repository_id);
    assert_eq!(first.revision, next.revision);
    assert!(next.observation_revision > first.observation_revision);
    assert!(!workspace.git_common_dir.join("devmap").exists());
}

#[test]
fn failed_refresh_never_labels_a_recent_cached_projection_as_fresh() {
    let repo = support::committed_repo();
    let mut proxy = QueryProxy::open_direct(repo.path()).unwrap();
    let original = serde_json::to_value(proxy.snapshot()).unwrap();
    // Damage only this disposable repository after a successful read. The error
    // must remain visible even inside the normal automatic refresh interval.
    std::fs::write(repo.path().join(".git/HEAD"), "invalid Git HEAD\n").unwrap();
    assert!(proxy.refresh(OffsetDateTime::now_utc()).is_err());
    assert!(
        proxy
            .refresh_if_due(std::time::Duration::from_secs(60))
            .is_err()
    );
    assert_eq!(serde_json::to_value(proxy.snapshot()).unwrap(), original);
}

#[test]
fn plain_refresh_keeps_inventory_timestamp_and_does_not_write_again() {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let frozen = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&workspace, &frozen.path().join("frozen")).unwrap();
    let mut proxy = QueryProxy::open_direct(repo.path()).unwrap();
    let observed_at = OffsetDateTime::now_utc();
    proxy
        .accept_inventory(vec![task(repo.path(), observed_at)], false, observed_at)
        .unwrap();
    let generation = devmap::store::RepositoryStore::open_existing(&workspace)
        .unwrap()
        .unwrap()
        .generation()
        .unwrap();
    proxy
        .refresh(observed_at + time::Duration::seconds(5))
        .unwrap();
    assert_eq!(
        proxy.client_view().inventory_observed_at(),
        Some(observed_at)
    );
    assert!(!proxy.client_view().inventory_complete());
    assert_eq!(
        devmap::store::RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        generation
    );
}

#[test]
fn supplied_partial_inventory_replaces_visible_subset_but_late_or_invalid_input_does_not() {
    let repo = support::committed_repo();
    let mut proxy = QueryProxy::open_direct(repo.path()).unwrap();
    let first_at = OffsetDateTime::now_utc();
    let first = task(repo.path(), first_at);
    let mut second = first.clone();
    second.session_id = "second-task".into();
    proxy
        .accept_inventory(vec![first, second.clone()], true, first_at)
        .unwrap();
    assert_eq!(proxy.client_view().observed_tasks().len(), 2);
    let next_at = first_at + time::Duration::seconds(1);
    proxy
        .accept_inventory(vec![second.clone()], false, next_at)
        .unwrap();
    assert_eq!(proxy.client_view().observed_tasks().len(), 1);
    assert_eq!(
        proxy.client_view().observed_tasks()[0].session_id,
        "second-task"
    );
    assert!(!proxy.client_view().inventory_complete());

    proxy.accept_inventory(vec![], true, first_at).unwrap();
    assert_eq!(proxy.client_view().observed_tasks().len(), 1);
    assert_eq!(proxy.client_view().inventory_observed_at(), Some(next_at));
    assert!(!proxy.client_view().inventory_complete());

    assert!(
        proxy
            .accept_inventory(vec![second.clone(), second], false, next_at)
            .is_err()
    );
    assert_eq!(proxy.client_view().observed_tasks().len(), 1);
    assert_eq!(proxy.client_view().inventory_observed_at(), Some(next_at));

    proxy
        .accept_inventory(vec![], false, next_at + time::Duration::seconds(1))
        .unwrap();
    assert!(proxy.client_view().observed_tasks().is_empty());
    assert!(!proxy.client_view().inventory_complete());
}

#[test]
fn viewer_uses_the_supplied_proxy_without_reaccepting_inventory() {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        sync::{Arc, Mutex},
        time::Duration,
    };
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let frozen = tempfile::tempdir().unwrap();
    devmap::store::migration::ensure(&workspace, &frozen.path().join("frozen")).unwrap();
    let proxy = Arc::new(Mutex::new(QueryProxy::open_direct(repo.path()).unwrap()));
    let observed_at = OffsetDateTime::now_utc();
    proxy
        .lock()
        .unwrap()
        .accept_inventory(vec![task(repo.path(), observed_at)], false, observed_at)
        .unwrap();
    let generation = devmap::store::RepositoryStore::open_existing(&workspace)
        .unwrap()
        .unwrap()
        .generation()
        .unwrap();
    let (handle, viewer) = devmap::viewer::start_live_viewer_with_proxy(
        Arc::clone(&proxy),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    assert_eq!(
        devmap::store::RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        generation
    );
    let next_at = observed_at + time::Duration::seconds(1);
    let mut renamed = task(repo.path(), next_at);
    renamed.display_title = "Changed through original proxy".into();
    proxy
        .lock()
        .unwrap()
        .accept_inventory(vec![renamed], false, next_at)
        .unwrap();
    let mut stream = TcpStream::connect_timeout(&handle.address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET /api/v1/dock/events?token={} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        handle.token
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let body = response.split_once("\r\n\r\n").unwrap().1;
    let event_id: u64 = body
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .unwrap()
        .parse()
        .unwrap();
    let model: serde_json::Value = serde_json::from_str(
        body.lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(model["observation_revision"], event_id);
    assert_eq!(
        model["task_observation"]["observed_at"],
        next_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    );
    assert_eq!(model["task_observation"]["complete"], false);
    assert!(body.contains("Changed through original proxy"));
    viewer.shutdown().unwrap();
}
