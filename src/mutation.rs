//! Closed domain commands. Prepare once before transport; execute only with an
//! authenticated workspace. This layer neither migrates stores nor retries IPC.
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    capture::{AgentDecisionInput, CaptureKernel, EvidenceInput, RequirementTraceInput},
    cli::AdapterHost,
    error::DevMapError,
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity,
        SessionContext, host_capabilities,
    },
    git::SourceWorkspace,
    hook::PreparedHook,
    journal::JournalStore,
    route_plan::{PlanInput, RoutePlan, RoutePlanStore},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonCaptureIdentity {
    pub session_id: String,
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub route_id: Option<String>,
    pub event_id: String,
    pub occurred_at: String,
    pub received_at: String,
    /// Historical observation only; never permission or current Git authority.
    pub branch: Option<String>,
    pub head: Option<String>,
}

impl CommonCaptureIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        workspace: &SourceWorkspace,
        session_id: String,
        agent_id: String,
        parent_agent_id: Option<String>,
        route_id: Option<String>,
        event_id: Option<String>,
        occurred_at: Option<String>,
        received_at: OffsetDateTime,
    ) -> Result<Self, DevMapError> {
        let received_at = received_at.format(&Rfc3339)?;
        let value = Self {
            session_id,
            agent_id,
            parent_agent_id,
            route_id,
            event_id: event_id.map(Ok).unwrap_or_else(|| random_event_id("mcp"))?,
            occurred_at: occurred_at.unwrap_or_else(|| received_at.clone()),
            received_at,
            branch: workspace.branch.clone(),
            head: Some(workspace.head.clone()),
        };
        value.validate(workspace)?;
        Ok(value)
    }

    fn validate(&self, workspace: &SourceWorkspace) -> Result<OffsetDateTime, DevMapError> {
        if !crate::journal::is_normal_session_component(&self.session_id) {
            return Err(DevMapError::JournalCorruption(
                "session ID must be a non-empty path component".into(),
            ));
        }
        for (field, value) in [
            ("session_id", Some(self.session_id.as_str())),
            ("agent_id", Some(self.agent_id.as_str())),
            ("parent_agent_id", self.parent_agent_id.as_deref()),
            ("route_id", self.route_id.as_deref()),
            ("event_id", Some(self.event_id.as_str())),
        ] {
            if let Some(value) = value {
                bounded_text(field, value, 512)?;
            }
        }
        let head = self
            .head
            .as_deref()
            .ok_or(DevMapError::InvalidDomain("capture head"))?;
        validate_observation(self.branch.as_deref(), Some(head))?;
        parse_time(&self.occurred_at)?;
        let received_at = parse_time(&self.received_at)?;
        self.context(workspace)?;
        ActorIdentity::new(self.agent_id.clone(), self.parent_agent_id.clone())?;
        Ok(received_at)
    }

    fn context(&self, workspace: &SourceWorkspace) -> Result<SessionContext, DevMapError> {
        let root = workspace.root.to_string_lossy().into_owned();
        SessionContext::new(
            self.session_id.clone(),
            self.route_id.clone(),
            root.clone(),
            Some(root),
            self.branch.clone(),
            self.head.clone(),
        )
    }

    fn kernel(&self, workspace: &SourceWorkspace) -> Result<CaptureKernel, DevMapError> {
        let received_at = self.validate(workspace)?;
        Ok(CaptureKernel::new(
            JournalStore::open(workspace, &self.session_id)?,
            host_capabilities(AdapterHost::GenericMcp),
            HostIdentity::new("generic_mcp", "devmap-mcp/1")?,
            ActorIdentity::new(self.agent_id.clone(), self.parent_agent_id.clone())?,
            self.context(workspace)?,
        )?
        .with_received_at(received_at))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationCommand {
    SetRoute {
        input: PlanInput,
    },
    RecordRequirement {
        common: CommonCaptureIdentity,
        input: RequirementTraceInput,
        raw_transcript_opt_in: bool,
    },
    RecordDecision {
        common: CommonCaptureIdentity,
        input: AgentDecisionInput,
    },
    RecordEvidence {
        common: CommonCaptureIdentity,
        input: EvidenceInput,
    },
    CaptureHook {
        prepared: PreparedHook,
    },
}

/// Local dispatch only; this is not a serialized permission supplied by a client.
pub(crate) enum StartupAdmission {
    Strict,
    QualifiedOrigins,
    RouteFreeJournal { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationResult {
    Route { plan: Box<RoutePlan> },
    CaptureAccepted { sha256: String },
    HookAccepted { sha256: Vec<String> },
}

impl MutationCommand {
    pub(crate) fn startup_admission(
        &self,
        workspace: &SourceWorkspace,
    ) -> Result<StartupAdmission, DevMapError> {
        let session = match self {
            Self::SetRoute { .. } => return Ok(StartupAdmission::QualifiedOrigins),
            Self::RecordRequirement { common, .. }
            | Self::RecordDecision { common, .. }
            | Self::RecordEvidence { common, .. } => {
                common.route_id.is_none().then(|| common.session_id.clone())
            }
            Self::CaptureHook { prepared } => prepared.route_free_session(workspace)?,
        };
        Ok(match session {
            Some(session_id) => StartupAdmission::RouteFreeJournal { session_id },
            None => StartupAdmission::Strict,
        })
    }
    /// Validate all prepared payload/envelope structure before any store setup.
    /// Current route state and compare-and-swap remain transaction-authoritative.
    pub fn validate_prepared(&self, workspace: &SourceWorkspace) -> Result<(), DevMapError> {
        if let Self::CaptureHook { prepared } = self {
            return prepared.validate_prepared(workspace);
        }
        if serde_json::to_vec(self)?.len() > 2 * 1024 * 1024 {
            return Err(DevMapError::ResourceLimit {
                resource: "mutation command",
                limit: 2 * 1024 * 1024,
            });
        }
        let capabilities = host_capabilities(AdapterHost::GenericMcp);
        let (common, event_type, payload) = match self {
            Self::SetRoute { input } => return crate::route_plan::validate_prepared_input(input),
            Self::RecordRequirement {
                common,
                input,
                raw_transcript_opt_in,
            } => (
                common,
                EventType::InstructionObserved,
                crate::capture::requirement_payload(
                    &capabilities,
                    input.clone(),
                    *raw_transcript_opt_in,
                )?,
            ),
            Self::RecordDecision { common, input } => (
                common,
                EventType::DecisionRecorded,
                crate::capture::decision_payload(&capabilities, input.clone())?,
            ),
            Self::RecordEvidence { common, input } => (
                common,
                EventType::EvidenceRecorded,
                crate::capture::evidence_payload(&capabilities, input.clone())?,
            ),
            Self::CaptureHook { .. } => unreachable!(),
        };
        common.validate(workspace)?;
        EventEnvelope::new(
            EVENT_SCHEMA_VERSION,
            &common.event_id,
            event_type,
            1,
            &common.occurred_at,
            HostIdentity::new("generic_mcp", "devmap-mcp/1")?,
            ActorIdentity::new(common.agent_id.clone(), common.parent_agent_id.clone())?,
            common.context(workspace)?,
            payload,
        )?;
        Ok(())
    }

    /// Revalidates deserialized commands. RoutePlanConflict is deliberately
    /// returned unchanged so the adapter retains its structured current plan.
    pub fn execute(&self, workspace: &SourceWorkspace) -> Result<MutationResult, DevMapError> {
        self.validate_prepared(workspace)?;
        if let Self::CaptureHook { prepared } = self {
            return prepared.execute(workspace);
        }
        let record =
            match self {
                Self::SetRoute { input } => {
                    return Ok(MutationResult::Route {
                        plan: Box::new(RoutePlanStore::open(workspace)?.set(input.clone())?),
                    });
                }
                Self::RecordRequirement {
                    common,
                    input,
                    raw_transcript_opt_in,
                } => common.kernel(workspace)?.record_requirement(
                    &common.event_id,
                    &common.occurred_at,
                    input.clone(),
                    *raw_transcript_opt_in,
                )?,
                Self::RecordDecision { common, input } => common
                    .kernel(workspace)?
                    .record_decision(&common.event_id, &common.occurred_at, input.clone())?,
                Self::RecordEvidence { common, input } => common
                    .kernel(workspace)?
                    .record_evidence(&common.event_id, &common.occurred_at, input.clone())?,
                Self::CaptureHook { .. } => unreachable!(),
            };
        Ok(MutationResult::CaptureAccepted {
            sha256: record.sha256,
        })
    }
}

pub(crate) fn random_event_id(prefix: &str) -> Result<String, DevMapError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(format!("{prefix}-{}", crate::canonical::sha256_hex(&bytes)))
}
pub(crate) fn bounded_text(
    field: &'static str,
    value: &str,
    limit: usize,
) -> Result<(), DevMapError> {
    if value.trim().is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    if value.len() > limit {
        return Err(DevMapError::ResourceLimit {
            resource: field,
            limit,
        });
    }
    Ok(())
}
pub(crate) fn parse_time(value: &str) -> Result<OffsetDateTime, DevMapError> {
    bounded_text("capture timestamp", value, 512)?;
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| DevMapError::InvalidDomain("capture timestamp"))
}
pub(crate) fn validate_observation(
    branch: Option<&str>,
    head: Option<&str>,
) -> Result<(), DevMapError> {
    if let Some(branch) = branch {
        bounded_text("capture branch", branch, 16 * 1024)?;
    }
    if let Some(head) = head {
        bounded_text("capture head", head, 64)?;
    }
    SessionContext::new(
        "validation",
        None,
        "validation",
        None,
        branch.map(str::to_owned),
        head.map(str::to_owned),
    )?;
    Ok(())
}

#[cfg(test)]
mod prepared_validation_tests {
    use super::*;
    use serde_json::json;

    fn workspace() -> (tempfile::TempDir, SourceWorkspace) {
        let temp = tempfile::tempdir().unwrap();
        let git = temp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        let workspace = SourceWorkspace {
            root: temp.path().to_owned(),
            git_dir: git.clone(),
            git_common_dir: git,
            branch: Some("main".into()),
            head: "a".repeat(40),
        };
        (temp, workspace)
    }
    fn common(workspace: &SourceWorkspace) -> CommonCaptureIdentity {
        CommonCaptureIdentity::prepare(
            workspace,
            "session".into(),
            "actor".into(),
            None,
            None,
            Some("event".into()),
            None,
            OffsetDateTime::now_utc(),
        )
        .unwrap()
    }
    fn evidence(workspace: &SourceWorkspace) -> MutationCommand {
        MutationCommand::RecordEvidence {
            common: common(workspace),
            input: EvidenceInput {
                kind: "test".into(),
                target: format!("commit:{}", workspace.head),
                command: None,
                outcome: "passed".into(),
            },
        }
    }

    #[test]
    fn invalid_semantic_execute_never_opens_legacy_journal() {
        let (_temp, workspace) = workspace();
        let mut command = evidence(&workspace);
        let MutationCommand::RecordEvidence { input, .. } = &mut command else {
            unreachable!()
        };
        input.target = "invalid target".into();
        assert!(matches!(
            command.validate_prepared(&workspace),
            Err(DevMapError::InvalidEvidenceTarget(_))
        ));
        assert!(matches!(
            command.execute(&workspace),
            Err(DevMapError::InvalidEvidenceTarget(_))
        ));
        assert!(!workspace.git_dir.join("devmap").exists());
    }

    #[test]
    fn complete_envelope_size_and_session_path_are_validated_before_storage() {
        let (_temp, workspace) = workspace();
        let mut command = evidence(&workspace);
        let MutationCommand::RecordEvidence {
            common: identity, ..
        } = &mut command
        else {
            unreachable!()
        };
        identity.session_id = "../escape".into();
        assert!(command.execute(&workspace).is_err());
        let command = MutationCommand::RecordDecision {
            common: common(&workspace),
            input: AgentDecisionInput {
                decision: "d".repeat(16 * 1024),
                basis: vec!["basis".into()],
                alternatives: vec!["other".into()],
                rationale: "r".repeat(16 * 1024),
                scope: "s".repeat(16 * 1024),
                authority: "a".repeat(16 * 1024),
                revisit_trigger: "later".into(),
            },
        };
        assert!(matches!(
            command.execute(&workspace),
            Err(DevMapError::ResourceLimit { .. })
        ));
        assert!(!workspace.git_dir.join("devmap").exists());
    }

    #[test]
    fn valid_semantic_and_native_gap_preflight_are_read_only() {
        let (_temp, workspace) = workspace();
        evidence(&workspace).validate_prepared(&workspace).unwrap();
        let requirement = MutationCommand::RecordRequirement {
            common: common(&workspace),
            input: RequirementTraceInput {
                source_kind: "user".into(),
                source_locator: None,
                quoted_text: "retain".into(),
            },
            raw_transcript_opt_in: false,
        };
        requirement.validate_prepared(&workspace).unwrap();
        let hook = PreparedHook::prepare(
            AdapterHost::Claude,
            "PostToolUse",
            json!({"event_id":"gap"}),
            &workspace,
            OffsetDateTime::now_utc(),
        )
        .unwrap();
        MutationCommand::CaptureHook { prepared: hook }
            .validate_prepared(&workspace)
            .unwrap();
        assert!(!workspace.git_dir.join("devmap").exists());
    }

    #[test]
    fn invalid_hook_session_and_route_structure_never_create_storage() {
        let (_temp, workspace) = workspace();
        let mut hook = PreparedHook::prepare(
            AdapterHost::Claude,
            "SessionStart",
            json!({"session_id":"session"}),
            &workspace,
            OffsetDateTime::now_utc(),
        )
        .unwrap();
        hook.body.insert("session_id".into(), json!("../escape"));
        assert!(
            MutationCommand::CaptureHook { prepared: hook }
                .execute(&workspace)
                .is_err()
        );
        let mut input = PlanInput {
            delivery: Default::default(),
            request_id: "request".into(),
            route_id: None,
            expected_revision: 0,
            worktree_id: "worktree".into(),
            goal: String::new(),
            target_ref: None,
            milestones: vec![],
            source: "user".into(),
            abandoned: false,
        };
        assert!(
            MutationCommand::SetRoute {
                input: input.clone()
            }
            .execute(&workspace)
            .is_err()
        );
        input.goal = "goal".into();
        input.target_ref = Some("refs/heads/invalid..ref".into());
        assert!(
            MutationCommand::SetRoute { input }
                .validate_prepared(&workspace)
                .is_err()
        );
        assert!(!workspace.git_dir.join("devmap").exists());
    }
}
