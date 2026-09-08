mod support;

#[test]
fn complete_dock_model_roundtrips_without_changing_public_json() {
    let repo = support::committed_repo();
    let service = devmap::dock::DockService::open(repo.path()).unwrap();
    let before = serde_json::to_vec(service.snapshot()).unwrap();
    let decoded: devmap::dock::DockReadModel = serde_json::from_slice(&before).unwrap();
    assert_eq!(&decoded, service.snapshot());
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), before);
}

#[test]
fn incompatible_fixed_contract_tags_are_rejected_on_decode() {
    let repo = support::committed_repo();
    let service = devmap::dock::DockService::open(repo.path()).unwrap();
    let model = serde_json::to_value(service.snapshot()).unwrap();
    for pointer in [
        "/schema_version",
        "/task_observation/scope",
        "/workspace_facts/0/facts_schema_version",
        "/workspace_facts/0/identity/current_context_source",
        "/workspace_facts/0/origin/recorded_creation/kind",
    ] {
        let mut invalid = model.clone();
        *invalid.pointer_mut(pointer).unwrap() = serde_json::json!("unsupported_wire_value");
        assert!(
            serde_json::from_value::<devmap::dock::DockReadModel>(invalid).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn supported_origin_and_task_association_tags_roundtrip() {
    for kind in ["unknown", "plan_start", "common_ancestor"] {
        let value = serde_json::json!({"kind":kind,"oid":null,"source":null,"event_at":null,"route_id":null});
        let decoded: devmap::dock::OriginEvidence = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
    let repo = support::committed_repo();
    let mut service = devmap::dock::DockService::open(repo.path()).unwrap();
    let now = time::OffsetDateTime::now_utc();
    let task = devmap::dock::ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: "019a0000-0000-7000-8000-000000000001".into(),
        display_title: "Wire task".into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: repo.path().to_string_lossy().into_owned(),
        status: devmap::presence::PresenceStatus::Working,
        updated_at: now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
    };
    let wire = serde_json::to_vec(&task).unwrap();
    assert_eq!(
        serde_json::from_slice::<devmap::dock::ObservedTask>(&wire).unwrap(),
        task
    );
    service.replace_observed_tasks(vec![task], now).unwrap();
    let chat = &service
        .snapshot()
        .lanes
        .iter()
        .find(|lane| !lane.chats.is_empty())
        .unwrap()
        .chats[0];
    for tag in [
        "presence_worktree_id",
        "codex_task_cwd",
        "agent_reported_working_directory",
    ] {
        let mut value = serde_json::to_value(chat).unwrap();
        value["association_source"] = serde_json::json!(tag);
        let decoded: devmap::dock::DockChat = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
    let mut invalid = serde_json::to_value(chat).unwrap();
    invalid["association_source"] = serde_json::json!("unsupported");
    assert!(serde_json::from_value::<devmap::dock::DockChat>(invalid).is_err());
}
