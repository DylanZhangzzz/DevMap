use serde::{Deserialize, Serialize};

use crate::error::DevMapError;

pub const SCHEMA_VERSION: &str = "devmap/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAnchor {
    pub repository_fingerprint: String,
    pub remote_url: Option<String>,
    pub head_commit: String,
    pub default_branch: Option<String>,
    pub dirty_at_adoption: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementTrace {
    object_type: RequirementObjectType,
    pub source_path: Option<String>,
    pub anchor: Option<String>,
    pub quoted_requirement: String,
}

impl RequirementTrace {
    pub fn new(
        source_path: Option<String>,
        anchor: Option<String>,
        quoted_requirement: String,
    ) -> Result<Self, DevMapError> {
        let quoted_requirement = required_trimmed(quoted_requirement, "requirement text")?;
        Ok(Self {
            object_type: RequirementObjectType::RequirementTrace,
            source_path,
            anchor,
            quoted_requirement,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequirementObjectType {
    RequirementTrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalScope {
    NotReconstructed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommonGroundDraft {
    pub schema_version: String,
    pub created_at: String,
    pub source: SourceAnchor,
    pub goal: String,
    pub requirements: Vec<RequirementTrace>,
    pub historical_scope: HistoricalScope,
}

impl CommonGroundDraft {
    pub fn new(
        created_at: String,
        source: SourceAnchor,
        goal: String,
        requirements: Vec<RequirementTrace>,
    ) -> Result<Self, DevMapError> {
        Ok(Self {
            schema_version: SCHEMA_VERSION.into(),
            created_at: required_trimmed(created_at, "creation timestamp")?,
            source,
            goal: required_trimmed(goal, "goal")?,
            requirements,
            historical_scope: HistoricalScope::NotReconstructed,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalEvent {
    pub schema_version: String,
    pub actor: String,
    pub approved_at: String,
    pub draft_sha256: String,
}

impl ApprovalEvent {
    pub fn new(
        actor: String,
        approved_at: String,
        draft_sha256: String,
    ) -> Result<Self, DevMapError> {
        Ok(Self {
            schema_version: SCHEMA_VERSION.into(),
            actor: required_trimmed(actor, "approval actor")?,
            approved_at: required_trimmed(approved_at, "approval timestamp")?,
            draft_sha256: required_trimmed(draft_sha256, "draft hash")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommonGround {
    pub schema_version: String,
    pub adopted_at: String,
    pub adoption_boundary_commit: String,
    pub source: SourceAnchor,
    pub goal: String,
    pub requirements: Vec<RequirementTrace>,
    pub historical_scope: HistoricalScope,
    pub approval_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalObjectRef {
    pub id: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommonGroundManifest {
    pub schema_version: String,
    pub common_ground: CanonicalObjectRef,
    pub approval: CanonicalObjectRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentState {
    pub schema_version: String,
    pub lifecycle: String,
    pub manifest_path: String,
    pub common_ground_id: String,
    pub capture_grade: String,
}

impl CommonGround {
    pub fn from_approved_draft(
        draft: CommonGroundDraft,
        approval_id: String,
        adopted_at: String,
    ) -> Result<Self, DevMapError> {
        let approval_id = required_trimmed(approval_id, "approval ID")?;
        let adopted_at = required_trimmed(adopted_at, "adoption timestamp")?;
        let adoption_boundary_commit = required_trimmed(
            draft.source.head_commit.clone(),
            "source adoption boundary commit",
        )?;

        Ok(Self {
            schema_version: draft.schema_version,
            adopted_at,
            adoption_boundary_commit,
            source: draft.source,
            goal: draft.goal,
            requirements: draft.requirements,
            historical_scope: HistoricalScope::NotReconstructed,
            approval_id,
        })
    }
}

fn required_trimmed(value: String, field: &'static str) -> Result<String, DevMapError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    Ok(value.to_owned())
}
