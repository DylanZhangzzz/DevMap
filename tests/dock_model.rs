mod support;

#[test]
fn empty_worktree_inventory_returns_error_without_panicking() {
    let fixture = support::dock_reducer_fixture();
    let result = devmap::dock::DockReducer::new(devmap::dock::NoRoutes).reduce(
        &fixture.workspace,
        vec![],
        fixture.presence,
        std::collections::BTreeMap::new(),
        fixture.now,
    );
    assert!(matches!(
        result,
        Err(devmap::error::DevMapError::InvalidPresence(_))
    ));
}

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::process::{Command, Stdio};

use devmap::dock::{DockReducer, DockService, NoRoutes, ObservedTask};
use devmap::journal::{JournalIntegrity, JournalSummary, summarize_existing_sessions};
use devmap::presence::{Confidence, PresenceStatus, StatusSource};
use devmap::worktrees::repository_id;

fn observed_task(workspace_path: &std::path::Path, title: &str) -> ObservedTask {
    ObservedTask {
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: "01a00000-0000-7000-8000-000000000001".into(),
        display_title: title.into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: workspace_path.to_string_lossy().into_owned(),
        status: PresenceStatus::Working,
        updated_at: "2026-09-03T10:00:00Z".into(),
    }
}

#[test]
fn passenger_snapshot_expires_without_changing_chat_lifecycle() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let now = time::OffsetDateTime::now_utc();
    let first = service
        .replace_observed_tasks(vec![observed_task(repo.path(), "Waiting owner")], now)
        .unwrap();
    assert_eq!(first.workspace_facts[0].passengers.state, "occupied");
    let stale = service.refresh(now + time::Duration::seconds(121)).unwrap();
    assert_eq!(stale.workspace_facts[0].passengers.state, "unknown");
    assert_eq!(stale.workspace_facts[0].passengers.observed_count, 1);
    assert!(!stale.workspace_facts[0].passengers.unattended_work);
    assert_eq!(
        stale.lanes[0].chats[0].lifecycle,
        devmap::dock::TaskLifecycle::Present
    );
}

fn import_linear_history(repo: &std::path::Path, commit_count: usize) {
    let base = support::git(repo, ["rev-parse", "HEAD"]);
    let mut input = String::from("blob\nmark :1\ndata 2\nx\n");
    for index in 0..commit_count {
        let mark = index + 2;
        let message = format!("imported history {index}");
        input.push_str("commit refs/heads/main\n");
        input.push_str(&format!("mark :{mark}\n"));
        input.push_str("author DevMap Test <devmap-test@example.test> 1788460000 +0000\n");
        input.push_str("committer DevMap Test <devmap-test@example.test> 1788460000 +0000\n");
        input.push_str(&format!("data {}\n{message}\n", message.len()));
        if index == 0 {
            input.push_str(&format!("from {base}\n"));
        } else {
            input.push_str(&format!("from :{}\n", mark - 1));
        }
        input.push_str("M 100644 :1 imported-history.txt\n\n");
    }
    input.push_str("done\n");

    let mut child = Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git fast-import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn replacing_inventory_updates_a_renamed_task_title() {
    let repo = support::committed_repo();
    let first_sync = time::OffsetDateTime::parse(
        "2026-09-03T10:01:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let second_sync = time::OffsetDateTime::parse(
        "2026-09-03T10:02:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let mut service = DockService::open(repo.path()).unwrap();
    let first_revision = service
        .replace_observed_tasks(vec![observed_task(repo.path(), "Old title")], first_sync)
        .unwrap()
        .revision;

    let renamed = service
        .replace_observed_tasks(vec![observed_task(repo.path(), "New title")], second_sync)
        .unwrap();
    let titles = renamed
        .branch_groups
        .iter()
        .flat_map(|group| &group.lanes)
        .flat_map(|lane| &lane.chats)
        .map(|chat| chat.display_title.as_str())
        .collect::<Vec<_>>();

    assert_eq!(titles, ["New title"]);
    assert_eq!(
        renamed.task_inventory_synced_at.as_deref(),
        Some("2026-09-03T10:02:00Z")
    );
    assert_eq!(renamed.revision, first_revision + 1);
}

#[test]
fn replacing_inventory_reassociates_a_moved_task_to_only_its_exact_workspace() {
    let repo = support::committed_repo();
    let destination = support::linked_worktree(repo.path(), "codex/task-destination");
    let nested_path = destination.path().join("nested-near-match");
    std::fs::create_dir(&nested_path).unwrap();
    let first_sync = time::OffsetDateTime::parse(
        "2026-09-03T10:01:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let second_sync = time::OffsetDateTime::parse(
        "2026-09-03T10:02:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let mut service = DockService::open(repo.path()).unwrap();

    let before = service
        .replace_observed_tasks(vec![observed_task(repo.path(), "Move me")], first_sync)
        .unwrap()
        .clone();
    let source = before
        .lanes
        .iter()
        .find(|lane| lane.is_current)
        .expect("source workspace lane");
    let destination_lane = before
        .lanes
        .iter()
        .find(|lane| lane.branch.as_deref() == Some("codex/task-destination"))
        .expect("destination workspace lane");
    let destination_worktree_id = destination_lane.worktree_id.clone();
    let destination_workspace_path = destination_lane.workspace_path.clone();
    assert!(source.chats.iter().any(|chat| {
        chat.codex_thread_id.as_deref() == Some("01a00000-0000-7000-8000-000000000001")
    }));

    let moved_task = observed_task(destination.path(), "Move me");
    let mut nested_near_match = observed_task(&nested_path, "Do not attach nested task");
    nested_near_match.session_id = "01a00000-0000-7000-8000-000000000002".into();
    let after = service
        .replace_observed_tasks(vec![moved_task, nested_near_match], second_sync)
        .unwrap();
    let matching_lanes = after
        .lanes
        .iter()
        .filter(|lane| {
            lane.chats.iter().any(|chat| {
                chat.codex_thread_id.as_deref() == Some("01a00000-0000-7000-8000-000000000001")
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(
        matching_lanes.len(),
        1,
        "a moved task must not be duplicated"
    );
    assert_eq!(
        matching_lanes[0].worktree_id, destination_worktree_id,
        "the verified task must move to the exact worktree identity"
    );
    assert_eq!(
        matching_lanes[0].workspace_path, destination_workspace_path,
        "the verified task must move to the exact canonical workspace path"
    );
    assert!(!matching_lanes[0].is_current);
    let moved_chat = matching_lanes[0]
        .chats
        .iter()
        .find(|chat| {
            chat.codex_thread_id.as_deref() == Some("01a00000-0000-7000-8000-000000000001")
        })
        .expect("moved chat selected by verified task ID");
    assert_eq!(
        moved_chat.association_source, "codex_task_cwd",
        "association evidence belongs to the verified moved chat"
    );
    assert!(
        after.lanes.iter().all(|lane| lane.chats.iter().all(|chat| {
            chat.codex_thread_id.as_deref() != Some("01a00000-0000-7000-8000-000000000002")
        })),
        "a nested cwd must not fuzzy-match its ancestor worktree"
    );
    assert_eq!(after.counts.tasks, 1);
    assert_eq!(after.revision, before.revision + 1);
}

#[test]
fn host_observed_tasks_keep_verified_codex_navigation_identity() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let model = service
        .replace_observed_tasks(
            vec![observed_task(repo.path(), "Open me")],
            time::OffsetDateTime::now_utc(),
        )
        .unwrap();

    let chat = model
        .lanes
        .iter()
        .flat_map(|lane| &lane.chats)
        .next()
        .unwrap();
    assert_eq!(
        chat.codex_thread_id.as_deref(),
        Some("01a00000-0000-7000-8000-000000000001")
    );
}

#[test]
fn reducer_groups_worktrees_at_the_same_exact_fork_point() {
    let repo = support::committed_repo();
    let dev = support::linked_worktree(repo.path(), "dev");
    std::fs::write(dev.path().join("dev.txt"), "development\n").unwrap();
    support::git(dev.path(), ["add", "dev.txt"]);
    support::git(dev.path(), ["commit", "-m", "development base"]);
    let shared_base = support::git(dev.path(), ["rev-parse", "HEAD"]);
    let alpha = support::linked_worktree_from(repo.path(), "alpha", "dev");
    let beta = support::linked_worktree_from(repo.path(), "beta", "dev");

    let service = DockService::open(repo.path()).unwrap();
    let groups = service
        .snapshot()
        .branch_groups
        .iter()
        .filter(|group| group.target_branch == "dev")
        .collect::<Vec<_>>();

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].fork_point.as_ref().unwrap().commit, shared_base);
    assert_eq!(
        groups[0]
            .lanes
            .iter()
            .filter_map(|lane| lane.branch.as_deref())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(service.snapshot().integration_branches[1].name, "dev");

    drop((alpha, beta, dev));
}

#[test]
fn reducer_keeps_distinct_fork_points_in_history_order() {
    let repo = support::committed_repo();
    let dev = support::linked_worktree(repo.path(), "dev");
    std::fs::write(dev.path().join("first.txt"), "first\n").unwrap();
    support::git(dev.path(), ["add", "first.txt"]);
    support::git(dev.path(), ["commit", "-m", "first development point"]);
    let first_point = support::git(dev.path(), ["rev-parse", "HEAD"]);
    let older = support::linked_worktree_from(repo.path(), "older-feature", "dev");
    std::fs::write(dev.path().join("second.txt"), "second\n").unwrap();
    support::git(dev.path(), ["add", "second.txt"]);
    support::git(dev.path(), ["commit", "-m", "second development point"]);
    let second_point = support::git(dev.path(), ["rev-parse", "HEAD"]);
    let newer = support::linked_worktree_from(repo.path(), "newer-feature", "dev");
    std::fs::write(dev.path().join("third.txt"), "third\n").unwrap();
    support::git(dev.path(), ["add", "third.txt"]);
    support::git(dev.path(), ["commit", "-m", "advance development"]);

    let service = DockService::open(repo.path()).unwrap();
    let commits = service
        .snapshot()
        .branch_groups
        .iter()
        .filter(|group| group.target_branch == "dev")
        .filter_map(|group| group.fork_point.as_ref().map(|fork| fork.commit.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [first_point, second_point]);

    drop((older, newer, dev));
}

#[test]
fn verified_inventory_promotes_matching_presence_without_losing_capture_evidence() {
    let mut fixture = support::dock_reducer_fixture();
    let id = "01a00000-0000-7000-8000-000000000001";
    fixture.presence.records[0].session_id = id.into();
    let record = fixture.presence.records[0].clone();
    let worktree = fixture
        .worktrees
        .iter()
        .find(|row| row.worktree_id == record.worktree_id)
        .unwrap();
    let task = ObservedTask {
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: id.into(),
        display_title: "Verified renamed task".into(),
        host: "local".into(),
        host_status: "idle".into(),
        workspace_path: worktree.root.to_string_lossy().into_owned(),
        status: PresenceStatus::Idle,
        updated_at: "2026-09-02T11:59:00Z".into(),
    };
    let model = DockReducer::new(NoRoutes)
        .reduce_with_tasks(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
            &[task],
        )
        .unwrap();
    let chat = model
        .lanes
        .iter()
        .flat_map(|lane| &lane.chats)
        .find(|chat| chat.session_id == id)
        .unwrap();
    assert_eq!(chat.codex_thread_id.as_deref(), Some(id));
    assert_eq!(chat.association_source, "codex_task_cwd");
    assert_eq!(chat.last_event_at, "2026-09-02T11:59:00Z");
    assert_eq!(chat.status, PresenceStatus::Idle);
    assert_eq!(chat.status_source, StatusSource::HostExplicit);
    assert_eq!(chat.confidence, Confidence::Observed);
    assert_eq!(chat.host, "local");
    assert_eq!(chat.host_status.as_deref(), Some("idle"));
    assert_eq!(chat.actor_id, record.actor_id);
    assert_eq!(chat.capture_grade, record.capture_grade);
    assert_eq!(chat.blocker_count, record.blocker_count);
    assert_eq!(chat.gap_count, record.gap_count);
}

#[test]
fn reducer_projects_workspace_chats_branch_and_merge_target_in_one_lane() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    let lane = model
        .lanes
        .iter()
        .find(|lane| !lane.chats.is_empty())
        .unwrap();
    assert_eq!(lane.chats[0].session_id, "active-session");
    assert_eq!(lane.chats[0].codex_thread_id, None);
    assert_eq!(lane.chats[0].association_source, "presence_worktree_id");
    let serialized = serde_json::to_value(&model).unwrap();
    let serialized_chat = serialized["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|lane| lane["chats"].as_array().unwrap())
        .find(|chat| chat["session_id"] == "active-session")
        .and_then(serde_json::Value::as_object)
        .unwrap();
    assert_eq!(
        serialized_chat.get("codex_thread_id"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(lane.relationship.merge_target.as_deref(), Some("main"));
    assert_eq!(lane.relationship.merged, Some(true));
}

#[test]
fn reducer_does_not_attach_a_chat_without_exact_presence() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.presence.records.clear();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    assert!(model.lanes.iter().all(|lane| lane.chats.is_empty()));
}

#[test]
fn reducer_puts_current_first_and_unknown_worktrees_in_warning_group() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    assert!(model.current[0].is_current);
    assert_eq!(model.active.len(), 1);
    assert_eq!(model.current[0].status, PresenceStatus::Unknown);
    assert_eq!(model.current[0].status_source, StatusSource::GitOnly);
    assert_eq!(model.current[0].confidence, Confidence::Unknown);
}

#[test]
fn reducer_preserves_multiple_agents_and_marks_corrupt_capture_incomplete() {
    let mut fixture = support::dock_reducer_fixture();
    let mut second = fixture.presence.records[0].clone();
    second.session_id = "waiting-session".into();
    second.actor_id = "agent-review".into();
    second.status = PresenceStatus::Waiting;
    second.status_source = StatusSource::HostExplicit;
    fixture.presence.records.push(second.clone());
    fixture.journals.insert(
        second.session_id.clone(),
        JournalSummary {
            session_id: second.session_id,
            records: 1,
            last_sequence: Some(1),
            last_sha256: Some("b".repeat(64)),
            integrity: JournalIntegrity::Corrupt,
        },
    );

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    assert_eq!(model.active.len(), 2);
    assert_eq!(model.active[0].status, PresenceStatus::Waiting);
    assert!(model.active[0].capture_incomplete);
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "journal_corrupt")
    );
}

#[test]
fn reducer_orders_activity_by_instant_not_timestamp_spelling() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.presence.records[0].session_id = "newer-session".into();
    fixture.presence.records[0].last_event_at = "2026-09-02T11:30:00Z".into();
    fixture.journals.clear();
    let mut older = fixture.presence.records[0].clone();
    older.session_id = "older-session".into();
    older.last_event_at = "2026-09-02T13:00:00+02:00".into();
    fixture.presence.records.push(older);
    for session_id in ["newer-session", "older-session"] {
        fixture.journals.insert(
            session_id.into(),
            JournalSummary {
                session_id: session_id.into(),
                records: 1,
                last_sequence: Some(1),
                last_sha256: Some("c".repeat(64)),
                integrity: JournalIntegrity::Verified,
            },
        );
    }

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    assert_eq!(model.active[0].session_id.as_deref(), Some("newer-session"));
}

#[test]
fn reducer_skips_mismatched_presence_and_is_deterministic() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.presence.records[0].repository_id = format!("sha256-{}", "f".repeat(64));
    fixture
        .presence
        .warnings
        .push(devmap::presence::PresenceWarning {
            code: "presence_record_invalid",
            subject_id: Some("corrupt-session".into()),
        });
    fixture.presence.truncated = true;
    let reducer = DockReducer::new(NoRoutes);
    let first = reducer
        .reduce(
            &fixture.workspace,
            fixture.worktrees.clone(),
            fixture.presence.clone(),
            fixture.journals.clone(),
            fixture.now,
        )
        .unwrap();
    let second = reducer
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    assert_eq!(first, second);
    assert!(first.truncated);
    assert_eq!(first.active.len(), 0);
    assert_eq!(first.stale_or_uninstrumented.len(), 1);
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| { warning.code == "presence_repository_mismatch" })
    );
    assert!(first.warnings.iter().any(|warning| {
        warning.code == "presence_record_invalid"
            && warning.subject_id.as_deref() == Some("corrupt-session")
    }));
}

#[test]
fn journal_summary_is_read_only_and_reports_verified_missing_and_corrupt() {
    let repo = support::committed_repo();
    let workspace = devmap::git::SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let journal = devmap::journal::JournalStore::open(&workspace, "valid-session").unwrap();
    let event = devmap::events::EventEnvelope::new(
        devmap::events::EVENT_SCHEMA_VERSION,
        "summary-event",
        devmap::events::EventType::EvidenceRecorded,
        1,
        "2026-09-02T12:00:00Z",
        devmap::events::HostIdentity::new("test", "1").unwrap(),
        devmap::events::ActorIdentity::new("agent", None).unwrap(),
        devmap::events::SessionContext::new(
            "valid-session",
            None,
            workspace.root.to_string_lossy(),
            None,
            None,
            Some(workspace.head.clone()),
        )
        .unwrap(),
        serde_json::json!({}),
    )
    .unwrap();
    journal.append(event).unwrap();
    std::fs::create_dir_all(workspace.git_dir.join("devmap/sessions/corrupt-session")).unwrap();
    std::fs::write(
        workspace
            .git_dir
            .join("devmap/sessions/corrupt-session/events.ndjson"),
        b"not-json\n",
    )
    .unwrap();
    let before = support::source_snapshot(repo.path());

    let summaries = summarize_existing_sessions(
        &workspace,
        &BTreeSet::from([
            "valid-session".into(),
            "missing-session".into(),
            "corrupt-session".into(),
        ]),
    );

    assert_eq!(
        summaries["valid-session"].integrity,
        JournalIntegrity::Verified
    );
    assert_eq!(summaries["valid-session"].last_sequence, Some(1));
    assert_eq!(
        summaries["missing-session"].integrity,
        JournalIntegrity::Missing
    );
    assert_eq!(
        summaries["corrupt-session"].integrity,
        JournalIntegrity::Corrupt
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn journal_summary_finds_sessions_recorded_in_other_worktrees() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/summary-other");
    let main = devmap::git::SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let other = devmap::git::SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();
    let event = devmap::events::EventEnvelope::new(
        devmap::events::EVENT_SCHEMA_VERSION,
        "other-summary-event",
        devmap::events::EventType::EvidenceRecorded,
        1,
        "2026-09-02T12:00:00Z",
        devmap::events::HostIdentity::new("test", "1").unwrap(),
        devmap::events::ActorIdentity::new("other-agent", None).unwrap(),
        devmap::events::SessionContext::new(
            "other-session",
            None,
            other.root.to_string_lossy(),
            None,
            other.branch.clone(),
            Some(other.head.clone()),
        )
        .unwrap(),
        serde_json::json!({}),
    )
    .unwrap();
    devmap::journal::JournalStore::open(&other, "other-session")
        .unwrap()
        .append(event)
        .unwrap();

    let summaries = summarize_existing_sessions(&main, &BTreeSet::from(["other-session".into()]));
    assert_eq!(
        summaries["other-session"].integrity,
        JournalIntegrity::Verified
    );
}

#[test]
fn dock_service_revision_changes_only_when_content_changes() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let first = service.snapshot().clone();
    let generated_at = time::OffsetDateTime::parse(
        &first.generated_at,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    service.refresh(generated_at).unwrap();
    assert_eq!(service.snapshot().revision, first.revision);

    let _linked = support::linked_worktree(repo.path(), "codex/revision-change");
    service.refresh(generated_at).unwrap();
    assert_eq!(service.snapshot().revision, first.revision + 1);
    assert_ne!(
        service.snapshot().content_hash().unwrap(),
        first.content_hash().unwrap()
    );
}

#[test]
fn unchanged_task_inventory_refreshes_observation_without_changing_structure() {
    let repo = support::committed_repo();
    let first_observation = time::OffsetDateTime::parse(
        "2026-09-03T10:01:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let second_observation = time::OffsetDateTime::parse(
        "2026-09-03T10:02:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let mut service = DockService::open(repo.path()).unwrap();
    let first = service
        .replace_observed_tasks(
            vec![observed_task(repo.path(), "Same title")],
            first_observation,
        )
        .unwrap()
        .clone();
    let second = service
        .replace_observed_tasks(
            vec![observed_task(repo.path(), "Same title")],
            second_observation,
        )
        .unwrap()
        .clone();

    assert_eq!(second.revision, first.revision);
    assert!(second.observation_revision > first.observation_revision);
    assert_eq!(
        second.content_hash().unwrap(),
        first.content_hash().unwrap()
    );
    assert_eq!(
        second.task_observation.observed_at.as_deref(),
        Some("2026-09-03T10:02:00Z")
    );
    assert!(second.task_observation.complete);
    assert!(
        second
            .workspace_facts
            .iter()
            .all(|facts| { facts.task_observed_at.as_deref() == Some("2026-09-03T10:02:00Z") })
    );
}

#[test]
fn workspace_facts_keep_included_and_dirty_as_independent_truths() {
    let repo = support::committed_repo();
    let feature = support::linked_worktree(repo.path(), "codex/included-dirty");
    std::fs::write(feature.path().join("feature.txt"), "feature\n").unwrap();
    support::git(feature.path(), ["add", "feature.txt"]);
    support::git(feature.path(), ["commit", "-m", "included feature"]);
    support::git(repo.path(), ["merge", "--ff-only", "codex/included-dirty"]);
    std::fs::write(feature.path().join("dirty.txt"), "dirty\n").unwrap();

    let service = DockService::open(repo.path()).unwrap();
    let facts = service
        .snapshot()
        .workspace_facts
        .iter()
        .find(|facts| {
            facts.worktree_id
                == service
                    .snapshot()
                    .lanes
                    .iter()
                    .find(|lane| lane.branch.as_deref() == Some("codex/included-dirty"))
                    .unwrap()
                    .worktree_id
        })
        .unwrap();

    assert_eq!(facts.integration, "included");
    assert_eq!(facts.working_state, "dirty");
    assert_eq!(facts.merge_commit_oid, None);
}

#[test]
fn detached_head_is_protected_when_a_stable_ref_reaches_it() {
    let repo = support::committed_repo();
    let protected_oid = support::git(repo.path(), ["rev-parse", "HEAD"]);
    std::fs::write(repo.path().join("later.txt"), "later\n").unwrap();
    support::git(repo.path(), ["add", "later.txt"]);
    support::git(repo.path(), ["commit", "-m", "later main commit"]);
    let detached = tempfile::tempdir().unwrap();
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "--detach",
            detached.path().to_str().unwrap(),
            protected_oid.as_str(),
        ],
    );

    let service = DockService::open(repo.path()).unwrap();
    let lane = service
        .snapshot()
        .lanes
        .iter()
        .find(|lane| lane.head == protected_oid)
        .unwrap();
    let facts = service
        .snapshot()
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == lane.worktree_id)
        .unwrap();

    assert!(facts.detached);
    assert_eq!(facts.head_ref_coverage, "protected");
}

#[test]
fn failed_git_status_produces_unknown_working_state() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.worktrees[0].root = fixture.workspace.root.join("missing-worktree");
    let worktree_id = fixture.worktrees[0].worktree_id.clone();

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    let facts = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == worktree_id)
        .unwrap();

    assert_eq!(facts.working_state, "unknown");
}

#[test]
fn dock_v4_has_exact_envelope_and_unique_counts() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    service
        .replace_observed_tasks(
            vec![observed_task(repo.path(), "Count once")],
            time::OffsetDateTime::parse(
                "2026-09-03T10:01:00Z",
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap(),
        )
        .unwrap();
    let value = serde_json::to_value(service.snapshot()).unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    assert_eq!(service.snapshot().schema_version, "devmap/dock/4");
    assert_eq!(
        keys,
        BTreeSet::from([
            "active",
            "branch_groups",
            "counts",
            "current",
            "current_worktree_id",
            "development_target",
            "generated_at",
            "integration_branches",
            "lanes",
            "observation_revision",
            "repository_id",
            "revision",
            "route_plans",
            "schema_version",
            "stale_or_uninstrumented",
            "task_inventory_synced_at",
            "task_observation",
            "topology",
            "truncated",
            "warnings",
            "workspace_facts",
        ])
    );
    assert_eq!(
        value["counts"],
        serde_json::json!({"workspaces": 1, "tasks": 1})
    );
}

#[test]
fn bounded_output_marks_a_partially_retained_task_roster() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let tasks = (0..100)
        .map(|index| ObservedTask {
            lifecycle: devmap::dock::TaskLifecycle::Present,
            session_id: format!("01a00000-0000-7000-8000-{index:012}"),
            display_title: format!("task-{index}-{}", "x".repeat(16 * 1024)),
            host: "local".into(),
            host_status: "active".into(),
            workspace_path: repo.path().to_string_lossy().into_owned(),
            status: PresenceStatus::Working,
            updated_at: "2026-09-03T10:00:00Z".into(),
        })
        .collect();

    let model = service
        .replace_observed_tasks(
            tasks,
            time::OffsetDateTime::parse(
                "2026-09-03T10:01:00Z",
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap(),
        )
        .unwrap();
    let bytes = devmap::canonical::canonical_json(model).unwrap();

    assert!(bytes.len() <= devmap::dock::MAX_DOCK_MODEL_BYTES);
    assert!(model.truncated);
    assert!(!model.task_observation.complete);
    assert_eq!(model.counts.tasks, 100);
    assert!(
        model
            .workspace_facts
            .iter()
            .all(|facts| facts.passengers.state == "unknown"
                && !facts.passengers.unattended_work
                && !facts.passengers.cleanup_review)
    );
}

#[test]
fn budget_preserves_all_256_real_workspaces_and_late_dirty_unprotected_facts() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let base = tempfile::tempdir().unwrap();
    let head = support::git(repo.path(), ["rev-parse", "HEAD"]);
    let tree = support::git(repo.path(), ["rev-parse", "HEAD^{tree}"]);
    let unique = support::git(
        repo.path(),
        [
            "commit-tree",
            tree.as_str(),
            "-p",
            head.as_str(),
            "-m",
            "unprotected checkout work",
        ],
    );
    for index in 1..256 {
        let path = base
            .path()
            .join(format!("workspace-{index:03}-{}", "long-name-".repeat(8)));
        let checkout = if index == 255 { &unique } else { &head };
        support::git(
            repo.path(),
            [
                "worktree",
                "add",
                "--detach",
                path.to_str().unwrap(),
                checkout,
            ],
        );
        if index == 255 {
            std::fs::write(path.join("unfinished.txt"), "dirty work\n").unwrap();
        }
    }
    let observed_at = time::OffsetDateTime::now_utc();
    let tasks = (0..100)
        .map(|index| {
            let mut task = observed_task(
                repo.path(),
                &format!("Task {index} {}", "x".repeat(16 * 1024 - 40)),
            );
            task.session_id = format!("01a00000-0000-7000-8000-{index:012}");
            task
        })
        .collect();
    let model = service
        .replace_observed_tasks_with_completeness(tasks, true, observed_at)
        .unwrap();
    assert_eq!(model.counts.workspaces, 256);
    assert_eq!(
        model.lanes.len(),
        256,
        "every canonical workspace identity must survive"
    );
    assert_eq!(
        model.workspace_facts.len(),
        256,
        "every workspace retains factual risk and observation times"
    );
    let late = model
        .lanes
        .iter()
        .find(|lane| lane.workspace_path.contains("workspace-255-"))
        .unwrap();
    let facts = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == late.worktree_id)
        .unwrap();
    assert_eq!(late.head, unique);
    assert_eq!(facts.head_oid, unique);
    assert_eq!(facts.working_state, "dirty");
    assert_eq!(facts.head_ref_coverage, "unprotected");
    assert!(facts.detached);
    assert_eq!(late.relationship.changed_file_count, 1);
    assert!(facts.git_observed_at.is_some());
    assert_eq!(facts.task_observed_at, model.task_observation.observed_at);
    assert!(model.task_observation.observed_at.is_some());
    assert!(!model.task_observation.complete);
    assert_eq!(model.counts.tasks, 100);
    assert!(
        model
            .lanes
            .iter()
            .map(|lane| lane.chats.len())
            .sum::<usize>()
            < 100
    );
    assert!(model.truncated);
    assert!(
        devmap::canonical::canonical_json(model).unwrap().len()
            <= devmap::dock::MAX_DOCK_MODEL_BYTES
    );
}

#[test]
fn topology_and_compatibility_share_the_dock_byte_budget_with_boundaries() {
    let repo = support::committed_repo();
    for index in 0..96 {
        std::fs::write(repo.path().join("history.txt"), format!("{index}\n")).unwrap();
        support::git(repo.path(), ["add", "history.txt"]);
        let subject = format!("history-{index}-{}", "x".repeat(8 * 1024));
        support::git(repo.path(), ["commit", "-m", subject.as_str()]);
    }

    let service = DockService::open(repo.path()).unwrap();
    let model = service.snapshot();
    let bytes = devmap::canonical::canonical_json(model).unwrap();

    assert!(bytes.len() <= devmap::dock::MAX_DOCK_MODEL_BYTES);
    assert!(model.truncated);
    assert!(
        model.topology.complete
            || model
                .topology
                .boundaries
                .iter()
                .any(|boundary| boundary.reason == "history_limit")
    );
}

#[test]
fn unused_topology_budget_retains_a_detached_head_when_history_fits() {
    let repo = support::committed_repo();
    let detached_head = support::git(repo.path(), ["rev-parse", "HEAD"]);
    import_linear_history(repo.path(), 1_800);
    let detached = tempfile::tempdir().unwrap();
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "--detach",
            detached.path().to_str().unwrap(),
            detached_head.as_str(),
        ],
    );

    let service = DockService::open(repo.path()).unwrap();
    let model = service.snapshot();

    assert!(
        model
            .topology
            .commits
            .iter()
            .any(|commit| commit.oid == detached_head)
    );
    assert!(
        devmap::canonical::canonical_json(model).unwrap().len()
            <= devmap::dock::MAX_DOCK_MODEL_BYTES
    );
}

#[test]
fn dock_opens_an_unborn_repository_without_inventing_a_commit() {
    let repo = tempfile::tempdir().unwrap();
    support::git(repo.path(), ["init", "-b", "main"]);

    let service = DockService::open(repo.path()).unwrap();
    let model = service.snapshot();

    assert_eq!(model.schema_version, "devmap/dock/4");
    assert!(model.topology.commits.is_empty());
    assert!(
        model
            .workspace_facts
            .iter()
            .all(|facts| facts.head_oid.is_empty()
                || facts.head_oid.bytes().all(|byte| byte == b'0'))
    );
}

#[test]
fn topology_cache_invalidates_on_ref_changes_without_caching_dirty_facts() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let first_revision = service.snapshot().revision;
    support::git(repo.path(), ["tag", "cache-invalidation"]);
    std::fs::write(repo.path().join("dirty-after-cache.txt"), "dirty\n").unwrap();

    service.refresh(time::OffsetDateTime::now_utc()).unwrap();
    let model = service.snapshot();
    let current = model
        .workspace_facts
        .iter()
        .find(|facts| facts.worktree_id == model.current_worktree_id)
        .unwrap();

    assert!(model.revision > first_revision);
    assert!(
        model
            .topology
            .refs
            .iter()
            .any(|reference| reference.ref_name == "refs/tags/cache-invalidation")
    );
    assert_eq!(current.working_state, "dirty");
}

#[test]
fn agents_json_is_canonical_bounded_and_does_not_change_source_git_state() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let output = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args([
            "agents",
            "--source",
            repo.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() < 1024 * 1024);
    let model: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(model["schema_version"], "devmap/dock/4");
    assert_eq!(
        model["repository_id"],
        repository_id(
            &devmap::git::SourceGitInspector::open(repo.path())
                .unwrap()
                .workspace()
                .unwrap()
        )
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn no_routes_never_fabricates_a_route() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            BTreeMap::new(),
            fixture.now,
        )
        .unwrap();
    assert!(
        model
            .current
            .iter()
            .chain(&model.active)
            .chain(&model.stale_or_uninstrumented)
            .all(|entry| entry.route_id.is_none())
    );
}

#[test]
fn reducer_bounds_large_multi_agent_output_without_hiding_truncation() {
    let mut fixture = support::dock_reducer_fixture();
    let template = fixture.presence.records.remove(0);
    fixture.journals.clear();
    for index in 0..100 {
        let mut record = template.clone();
        record.session_id = format!("large-session-{index:03}");
        record.actor_id = format!("agent-{index:03}-{}", "x".repeat(16 * 1024 - 16));
        fixture.journals.insert(
            record.session_id.clone(),
            JournalSummary {
                session_id: record.session_id.clone(),
                records: 1,
                last_sequence: Some(1),
                last_sha256: Some("d".repeat(64)),
                integrity: JournalIntegrity::Verified,
            },
        );
        fixture.presence.records.push(record);
    }

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    let bytes = devmap::canonical::canonical_json(&model).unwrap();
    assert!(model.truncated);
    assert!(!model.lanes.is_empty());
    assert!(model.lanes.iter().any(|lane| lane.is_current));
    assert!(model.lanes.iter().any(|lane| !lane.chats.is_empty()));
    assert!(bytes.len() <= devmap::dock::MAX_DOCK_MODEL_BYTES);
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "dock_output_truncated")
    );
}
