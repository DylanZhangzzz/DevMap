use devmap::domain::{
    ApprovalEvent, CommonGround, CommonGroundDraft, HistoricalScope, RequirementTrace,
    SourceAnchor,
};

fn source_anchor() -> SourceAnchor {
    SourceAnchor {
        repository_fingerprint: "sha256-repository".into(),
        remote_url: Some("https://example.test/acme/payments.git".into()),
        head_commit: "0123456789abcdef".into(),
        default_branch: Some("main".into()),
        dirty_at_adoption: true,
    }
}

#[test]
fn approved_common_ground_preserves_the_adoption_boundary() {
    let requirement = RequirementTrace::new(
        Some("docs/spec.md".into()),
        Some("payment-lock".into()),
        "Use a PostgreSQL advisory lock.".into(),
    )
    .unwrap();
    let draft = CommonGroundDraft::new(
        "2026-08-26T12:00:00Z".into(),
        source_anchor(),
        "Prevent duplicate payment capture".into(),
        vec![requirement],
    )
    .unwrap();
    let approval = ApprovalEvent::new(
        "Dylan".into(),
        "2026-08-26T12:05:00Z".into(),
        "abc123".into(),
    )
    .unwrap();
    let common_ground = CommonGround::from_approved_draft(
        draft,
        "approval:sha256-deadbeef".into(),
        approval.approved_at.clone(),
    )
    .unwrap();

    assert_eq!(common_ground.adoption_boundary_commit, "0123456789abcdef");
    assert_eq!(common_ground.historical_scope, HistoricalScope::NotReconstructed);
    assert_eq!(approval.draft_sha256, "abc123");
    assert_eq!(common_ground.requirements.len(), 1);
}

#[test]
fn human_requirement_serializes_as_requirement_trace() {
    let requirement = RequirementTrace::new(None, None, "Keep events durable".into()).unwrap();
    let value = serde_json::to_value(requirement).unwrap();

    assert_eq!(value["object_type"], "requirement_trace");
    assert!(value.get("decision").is_none());
}

#[test]
fn constructors_reject_blank_human_inputs() {
    let draft_error = CommonGroundDraft::new(
        "2026-08-26T12:00:00Z".into(),
        source_anchor(),
        "   ".into(),
        vec![],
    )
    .unwrap_err();
    assert!(draft_error.to_string().contains("goal"));

    let approval_error = ApprovalEvent::new(
        "  ".into(),
        "2026-08-26T12:05:00Z".into(),
        "abc123".into(),
    )
    .unwrap_err();
    assert!(approval_error.to_string().contains("actor"));

    let requirement_error = RequirementTrace::new(None, None, "\n\t".into()).unwrap_err();
    assert!(requirement_error.to_string().contains("requirement"));
}

#[test]
fn historical_scope_cannot_claim_reconstruction() {
    let serialized = serde_json::to_string(&HistoricalScope::NotReconstructed).unwrap();
    assert_eq!(serialized, r#""not_reconstructed""#);
}

