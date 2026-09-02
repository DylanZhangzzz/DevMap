use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::canonical::{canonical_json, ensure_no_floating_points, sha256_hex};
use crate::cli::AdapterHost;
use crate::error::DevMapError;

pub const EVENT_SCHEMA_VERSION: &str = "devmap/event/1";
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    SessionStopped,
    TurnCompleted,
    InstructionObserved,
    AgentStarted,
    AgentStopped,
    ToolRequested,
    ToolCompleted,
    MutationObserved,
    DecisionRecorded,
    EvidenceRecorded,
    ContextCompacting,
    ContextCompacted,
    GitActionProposed,
    GitActionAuthorized,
    GitActionExecuted,
    GitActionFailed,
    AuthorityChanged,
    CaptureGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    name: String,
    adapter_version: String,
}

impl HostIdentity {
    pub fn new(
        name: impl Into<String>,
        adapter_version: impl Into<String>,
    ) -> Result<Self, DevMapError> {
        Ok(Self {
            name: required("host.name", name.into())?,
            adapter_version: required("host.adapter_version", adapter_version.into())?,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }

    fn validate(&self) -> Result<(), DevMapError> {
        required("host.name", self.name.clone())?;
        required("host.adapter_version", self.adapter_version.clone())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorIdentity {
    agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_agent_id: Option<String>,
}

impl ActorIdentity {
    pub fn new(
        agent_id: impl Into<String>,
        parent_agent_id: Option<String>,
    ) -> Result<Self, DevMapError> {
        Ok(Self {
            agent_id: required("actor.agent_id", agent_id.into())?,
            parent_agent_id: parent_agent_id
                .map(|value| required("actor.parent_agent_id", value))
                .transpose()?,
        })
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn parent_agent_id(&self) -> Option<&str> {
        self.parent_agent_id.as_deref()
    }

    fn validate(&self) -> Result<(), DevMapError> {
        required("actor.agent_id", self.agent_id.clone())?;
        if let Some(parent_agent_id) = &self.parent_agent_id {
            required("actor.parent_agent_id", parent_agent_id.clone())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContext {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    route_id: Option<String>,
    repository: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
}

impl SessionContext {
    pub fn new(
        session_id: impl Into<String>,
        route_id: Option<String>,
        repository: impl Into<String>,
        worktree: Option<String>,
        branch: Option<String>,
        head: Option<String>,
    ) -> Result<Self, DevMapError> {
        let head = head
            .map(|value| {
                required("context.head", value)
                    .and_then(|value| validate_lowercase_sha("context.head", value))
            })
            .transpose()?;
        Ok(Self {
            session_id: required("context.session_id", session_id.into())?,
            route_id: optional("context.route_id", route_id)?,
            repository: required("context.repository", repository.into())?,
            worktree: optional("context.worktree", worktree)?,
            branch: optional("context.branch", branch)?,
            head,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn route_id(&self) -> Option<&str> {
        self.route_id.as_deref()
    }
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn worktree(&self) -> Option<&str> {
        self.worktree.as_deref()
    }
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }
    pub fn head(&self) -> Option<&str> {
        self.head.as_deref()
    }

    fn validate(&self) -> Result<(), DevMapError> {
        required("context.session_id", self.session_id.clone())?;
        required("context.repository", self.repository.clone())?;
        if let Some(route_id) = &self.route_id {
            required("context.route_id", route_id.clone())?;
        }
        if let Some(worktree) = &self.worktree {
            required("context.worktree", worktree.clone())?;
        }
        if let Some(branch) = &self.branch {
            required("context.branch", branch.clone())?;
        }
        if let Some(head) = &self.head {
            validate_lowercase_sha("context.head", required("context.head", head.clone())?)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventEnvelope {
    schema_version: String,
    event_id: String,
    event_type: EventType,
    sequence: u64,
    occurred_at: String,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
    payload: Value,
}

impl EventEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schema_version: impl Into<String>,
        event_id: impl Into<String>,
        event_type: EventType,
        sequence: u64,
        occurred_at: impl Into<String>,
        host: HostIdentity,
        actor: ActorIdentity,
        context: SessionContext,
        payload: Value,
    ) -> Result<Self, DevMapError> {
        let schema_version = schema_version.into();
        if schema_version != EVENT_SCHEMA_VERSION {
            return Err(invalid("unsupported schema version"));
        }
        if sequence == 0 {
            return Err(invalid("sequence must be greater than zero"));
        }
        if !payload.is_object() {
            return Err(invalid("structured event payload must be a JSON object"));
        }
        host.validate()?;
        actor.validate()?;
        context.validate()?;
        ensure_no_floating_points(&payload)?;
        let occurred_at = required("occurred_at", occurred_at.into())?;
        OffsetDateTime::parse(&occurred_at, &Rfc3339)
            .map_err(|_| invalid("occurred_at must be RFC 3339"))?;
        if serde_json::to_vec(&payload)?.len() > MAX_EVENT_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "event payload",
                limit: MAX_EVENT_BYTES,
            });
        }

        let event = Self {
            schema_version,
            event_id: required("event_id", event_id.into())?,
            event_type,
            sequence,
            occurred_at,
            host,
            actor,
            context,
            payload,
        };
        if serde_json::to_vec(&event)?.len() > MAX_EVENT_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "event envelope",
                limit: MAX_EVENT_BYTES,
            });
        }
        Ok(event)
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
    pub fn event_type(&self) -> &EventType {
        &self.event_type
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }
    pub fn host(&self) -> &HostIdentity {
        &self.host
    }
    pub fn actor(&self) -> &ActorIdentity {
        &self.actor
    }
    pub fn context(&self) -> &SessionContext {
        &self.context
    }
    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, DevMapError> {
        canonical_json(self)
    }

    pub fn sha256(&self) -> Result<String, DevMapError> {
        Ok(sha256_hex(&self.canonical_bytes()?))
    }
}

#[derive(Deserialize)]
struct EventEnvelopeInput {
    schema_version: String,
    event_id: String,
    event_type: EventType,
    sequence: u64,
    occurred_at: String,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
    payload: Value,
}

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = EventEnvelopeInput::deserialize(deserializer)?;
        Self::new(
            input.schema_version,
            input.event_id,
            input.event_type,
            input.sequence,
            input.occurred_at,
            input.host,
            input.actor,
            input.context,
            input.payload,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureCapabilities {
    pub lifecycle_events: Vec<EventType>,
    pub pre_mutation_blocking: bool,
    pub subagent_lifecycle: bool,
    pub workspace_rebind: bool,
    pub tool_results: bool,
    pub commit_mapping: bool,
    pub raw_transcript: bool,
}

impl CaptureCapabilities {
    pub fn grade(&self) -> CaptureGrade {
        let complete_lifecycle =
            self.has(EventType::SessionStarted) && self.has(EventType::SessionStopped);
        let mutation_and_evidence =
            self.has(EventType::MutationObserved) && self.has(EventType::EvidenceRecorded);

        if complete_lifecycle
            && mutation_and_evidence
            && self.pre_mutation_blocking
            && self.subagent_lifecycle
            && self.tool_results
            && self.commit_mapping
        {
            CaptureGrade::A
        } else if complete_lifecycle
            && mutation_and_evidence
            && self.tool_results
            && self.commit_mapping
        {
            CaptureGrade::B
        } else if self.tool_results && self.has(EventType::MutationObserved) {
            CaptureGrade::C
        } else {
            CaptureGrade::D
        }
    }

    fn has(&self, event_type: EventType) -> bool {
        self.lifecycle_events.contains(&event_type)
    }
}

pub fn host_capabilities(host: AdapterHost) -> CaptureCapabilities {
    match host {
        AdapterHost::Codex | AdapterHost::Claude => CaptureCapabilities {
            lifecycle_events: vec![
                EventType::SessionStarted,
                EventType::SessionStopped,
                EventType::TurnCompleted,
                EventType::InstructionObserved,
                EventType::AgentStarted,
                EventType::AgentStopped,
                EventType::ToolRequested,
                EventType::ToolCompleted,
                EventType::ContextCompacting,
                EventType::ContextCompacted,
            ],
            pre_mutation_blocking: false,
            subagent_lifecycle: true,
            workspace_rebind: false,
            tool_results: false,
            commit_mapping: false,
            raw_transcript: false,
        },
        AdapterHost::GenericMcp => CaptureCapabilities {
            lifecycle_events: vec![
                EventType::InstructionObserved,
                EventType::DecisionRecorded,
                EventType::EvidenceRecorded,
            ],
            pre_mutation_blocking: false,
            subagent_lifecycle: false,
            workspace_rebind: false,
            tool_results: false,
            commit_mapping: false,
            raw_transcript: false,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureGrade {
    A,
    B,
    C,
    D,
}

fn required(field: &'static str, value: String) -> Result<String, DevMapError> {
    if value.trim().is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    Ok(value)
}

fn optional(field: &'static str, value: Option<String>) -> Result<Option<String>, DevMapError> {
    value.map(|value| required(field, value)).transpose()
}

fn validate_lowercase_sha(field: &'static str, value: String) -> Result<String, DevMapError> {
    let valid = matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if !valid {
        return Err(invalid(format!(
            "{field} must be a lower-case 40- or 64-hex identifier"
        )));
    }
    Ok(value)
}

fn invalid(message: impl Into<String>) -> DevMapError {
    DevMapError::InvalidEventEnvelope(message.into())
}
