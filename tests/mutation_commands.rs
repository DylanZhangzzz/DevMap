mod support;
use devmap::{
    capture::{AgentDecisionInput, EvidenceInput, RequirementTraceInput},
    git::{SourceGitInspector, SourceWorkspace},
    hook::PreparedHook,
    mutation::{CommonCaptureIdentity, MutationCommand, MutationResult},
    store::{RepositoryStore, migration},
};
use serde_json::json;
use time::OffsetDateTime;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, SourceWorkspace) {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let backup = tempfile::tempdir().unwrap();
    migration::ensure(&workspace, &backup.path().join("frozen")).unwrap();
    (repo, backup, workspace)
}
fn state(w: &SourceWorkspace) -> Vec<String> {
    let store = RepositoryStore::open_existing(w).unwrap().unwrap();
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let mut result = vec![store.generation().unwrap().to_string()];
    for query in [
        "SELECT record_json FROM journal_records ORDER BY session_id,sequence",
        "SELECT record_json FROM presence_records ORDER BY session_id",
        "SELECT covered_sequence || ':' || covered_sha256 FROM presence_projection ORDER BY session_id",
    ] {
        result.extend(
            connection
                .prepare(query)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    result
}
#[test]
fn prepared_capture_survives_head_change_and_does_not_renew_presence() {
    let (repo, _backup, workspace) = setup();
    let common = CommonCaptureIdentity::prepare(
        &workspace,
        "s".into(),
        "a".into(),
        None,
        None,
        None,
        None,
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    let command = MutationCommand::RecordEvidence {
        common,
        input: EvidenceInput {
            kind: "test".into(),
            target: format!("commit:{}", workspace.head),
            command: None,
            outcome: "passed".into(),
        },
    };
    let accepted = command.execute(&workspace).unwrap();
    let before = state(&workspace);
    support::git(repo.path(), ["commit", "--allow-empty", "-m", "advance"]);
    let moved = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert_ne!(workspace.head, moved.head);
    let retry: MutationCommand =
        serde_json::from_slice(&serde_json::to_vec(&command).unwrap()).unwrap();
    assert_eq!(retry.execute(&moved).unwrap(), accepted);
    assert_eq!(state(&moved), before);
    let mut changed = command;
    if let MutationCommand::RecordEvidence { input, .. } = &mut changed {
        input.outcome = "failed".into();
    }
    assert!(changed.execute(&moved).is_err());
    assert_eq!(state(&moved), before);
}
#[test]
fn anonymous_hook_invocations_are_distinct_but_each_retry_is_exact() {
    let (_repo, _backup, w) = setup();
    let now = OffsetDateTime::now_utc();
    let body = json!({"session_id":"hook-session", "hook_event_name":"SessionStart"});
    let first = MutationCommand::CaptureHook {
        prepared: PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            "SessionStart",
            body.clone(),
            &w,
            now,
        )
        .unwrap(),
    };
    let second = MutationCommand::CaptureHook {
        prepared: PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            "SessionStart",
            body,
            &w,
            now,
        )
        .unwrap(),
    };
    assert_ne!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    let accepted = first.execute(&w).unwrap();
    let before = state(&w);
    assert_eq!(first.execute(&w).unwrap(), accepted);
    assert_eq!(state(&w), before);
    second.execute(&w).unwrap();
    assert_ne!(state(&w), before);
}
#[test]
fn native_id_and_malformed_hook_gap_retry_preserve_sql_state() {
    let (_repo, _backup, w) = setup();
    let now = OffsetDateTime::now_utc();
    for mut body in [
        json!({"session_id":"native", "hook_event_name":"PostToolUse", "event_id":"e", "tool_name":"Write"}),
        json!({"event_id":"gap"}),
    ] {
        body.as_object_mut().unwrap().insert(
            "occurred_at".into(),
            json!(
                now.format(&time::format_description::well_known::Rfc3339)
                    .unwrap()
            ),
        );
        let expected = devmap::hook::normalize_hook_input(
            devmap::cli::AdapterHost::Claude,
            "PostToolUse",
            body.clone(),
            &w,
        )
        .unwrap();
        assert_eq!(
            expected.len(),
            if body.get("session_id").is_some() {
                2
            } else {
                1
            }
        );
        let prepared = PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            "PostToolUse",
            body.clone(),
            &w,
            now,
        )
        .unwrap();
        let repeated = PreparedHook::prepare(
            devmap::cli::AdapterHost::Claude,
            "PostToolUse",
            body,
            &w,
            now,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&prepared).unwrap(),
            serde_json::to_value(&repeated).unwrap()
        );
        let command = MutationCommand::CaptureHook { prepared };
        let accepted = command.execute(&w).unwrap();
        let records = devmap::journal::JournalStore::open(&w, expected[0].context().session_id())
            .unwrap()
            .replay()
            .unwrap();
        assert_eq!(
            records
                .into_iter()
                .map(|record| record.event)
                .collect::<Vec<_>>(),
            expected
        );
        let before = state(&w);
        assert_eq!(command.execute(&w).unwrap(), accepted);
        assert_eq!(state(&w), before);
        if expected.len() == 2 {
            let mut changed = command;
            let MutationCommand::CaptureHook { prepared } = &mut changed else {
                unreachable!()
            };
            prepared.body.insert("tool_name".into(), json!("Edit"));
            assert!(changed.execute(&w).is_err());
            assert_eq!(state(&w), before);
        }
    }
}
#[test]
fn closed_command_rejects_unknown_fields_and_missing_identity() {
    assert!(
        serde_json::from_value::<MutationCommand>(
            json!({"kind":"execute_sql","sql":"DROP TABLE store_meta"})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<CommonCaptureIdentity>(json!({"session_id":"s"})).is_err());
}

#[test]
fn requirement_and_decision_commands_revalidate_and_retry_without_writes() {
    let (_repo, _backup, w) = setup();
    let common = CommonCaptureIdentity::prepare(
        &w,
        "typed".into(),
        "a".into(),
        None,
        None,
        Some("required-event".into()),
        None,
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    let requirement = MutationCommand::RecordRequirement {
        common: common.clone(),
        input: RequirementTraceInput {
            source_kind: "user".into(),
            source_locator: None,
            quoted_text: "retain history".into(),
        },
        raw_transcript_opt_in: false,
    };
    let mut decision_common = common;
    decision_common.event_id = "decision-event".into();
    let decision = MutationCommand::RecordDecision {
        common: decision_common,
        input: AgentDecisionInput {
            decision: "preserve".into(),
            basis: vec!["requirement".into()],
            alternatives: vec!["discard".into()],
            rationale: "history".into(),
            scope: "local".into(),
            authority: "user".into(),
            revisit_trigger: "new instruction".into(),
        },
    };
    for command in [requirement, decision] {
        let accepted = command.execute(&w).unwrap();
        let before = state(&w);
        let retry: MutationCommand =
            serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
        assert_eq!(retry.execute(&w).unwrap(), accepted);
        assert_eq!(state(&w), before);
        let mut changed = command.clone();
        match &mut changed {
            MutationCommand::RecordRequirement { input, .. } => {
                input.quoted_text = "changed".into()
            }
            MutationCommand::RecordDecision { input, .. } => input.decision = "changed".into(),
            _ => unreachable!(),
        }
        assert!(changed.execute(&w).is_err());
        assert_eq!(state(&w), before);
        let mut invalid = command;
        match &mut invalid {
            MutationCommand::RecordRequirement { common, .. }
            | MutationCommand::RecordDecision { common, .. } => {
                common.head = Some("invalid head".into())
            }
            _ => unreachable!(),
        }
        assert!(invalid.execute(&w).is_err());
        assert_eq!(state(&w), before);
        match &mut invalid {
            MutationCommand::RecordRequirement { common, .. }
            | MutationCommand::RecordDecision { common, .. } => common.head = None,
            _ => unreachable!(),
        }
        let missing_head: MutationCommand =
            serde_json::from_value(serde_json::to_value(invalid).unwrap()).unwrap();
        assert!(missing_head.execute(&w).is_err());
        assert_eq!(state(&w), before);
    }
}

#[test]
fn route_conflict_keeps_current_plan_and_retry_returns_original() {
    let (_repo, _backup, w) = setup();
    let mut input = devmap::route_plan::PlanInput {
        delivery: Default::default(),
        request_id: "first".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: devmap::worktrees::WorktreeScanner::scan(&w).unwrap()[0]
            .worktree_id
            .clone(),
        goal: "first goal".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let original = MutationCommand::SetRoute {
        input: input.clone(),
    };
    let first = original.execute(&w).unwrap();
    let MutationResult::Route { plan } = &first else {
        panic!("expected route");
    };
    input.route_id = Some(plan.route_id.clone());
    input.expected_revision = plan.revision;
    input.request_id = "second".into();
    input.goal = "second goal".into();
    let second = MutationCommand::SetRoute {
        input: input.clone(),
    }
    .execute(&w)
    .unwrap();
    input.request_id = "stale".into();
    match (MutationCommand::SetRoute { input }).execute(&w) {
        Err(devmap::error::DevMapError::RoutePlanConflict {
            revision,
            current_plan,
        }) => {
            let MutationResult::Route { plan } = second else {
                panic!("expected route");
            };
            assert_eq!(revision, plan.revision);
            assert_eq!(current_plan.as_deref(), Some(plan.as_ref()));
        }
        other => panic!("expected structured conflict: {other:?}"),
    }
    let before = state(&w);
    assert_eq!(original.execute(&w).unwrap(), first);
    assert_eq!(state(&w), before);
}

#[test]
fn deserialized_hook_limits_and_historical_context_are_revalidated() {
    let (_repo, _backup, w) = setup();
    let prepared = PreparedHook::prepare(
        devmap::cli::AdapterHost::Claude,
        "SessionStart",
        json!({"session_id":"limits", "hook_event_name":"SessionStart"}),
        &w,
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    let before = state(&w);
    for invalid in ["bad-head", ""] {
        let mut changed = prepared.clone();
        changed.identity.head = invalid.into();
        let command: MutationCommand = serde_json::from_value(
            serde_json::to_value(MutationCommand::CaptureHook { prepared: changed }).unwrap(),
        )
        .unwrap();
        assert!(command.execute(&w).is_err());
    }
    let mut oversized = prepared.clone();
    oversized.body.insert(
        "large".into(),
        json!("x".repeat(devmap::hook::MAX_HOOK_BODY_BYTES)),
    );
    assert!(
        MutationCommand::CaptureHook {
            prepared: oversized
        }
        .execute(&w)
        .is_err()
    );
    let mut nested = json!(null);
    for _ in 0..34 {
        nested = json!([nested]);
    }
    let mut deep = prepared;
    deep.body.insert("deep".into(), nested);
    assert!(
        MutationCommand::CaptureHook { prepared: deep }
            .execute(&w)
            .is_err()
    );
    assert_eq!(state(&w), before);
}
