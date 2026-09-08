use std::io::Read;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::canonical::sha256_hex;
use crate::cli::{AdapterHost, HookHandleArgs};
use crate::error::DevMapError;
use crate::events::{
    ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    host_capabilities,
};
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::JournalStore;

const ADAPTER_VERSION: &str = "devmap-hook/1";
const MAX_IDENTIFIER_BYTES: usize = 512;
pub const MAX_HOOK_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparedHookHost {
    Codex,
    Claude,
    GenericMcp,
}
impl From<AdapterHost> for PreparedHookHost {
    fn from(host: AdapterHost) -> Self {
        match host {
            AdapterHost::Codex => Self::Codex,
            AdapterHost::Claude => Self::Claude,
            AdapterHost::GenericMcp => Self::GenericMcp,
        }
    }
}
impl From<PreparedHookHost> for AdapterHost {
    fn from(host: PreparedHookHost) -> Self {
        match host {
            PreparedHookHost::Codex => Self::Codex,
            PreparedHookHost::Claude => Self::Claude,
            PreparedHookHost::GenericMcp => Self::GenericMcp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedHookIdentity {
    pub event_id: String,
    pub occurred_at: String,
    pub received_at: String,
    pub branch: Option<String>,
    pub head: String,
}

/// Immutable invocation data. Paths are deliberately absent. Native body fields
/// remain data interpreted by the same normalizer used by direct hook capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedHook {
    pub host: PreparedHookHost,
    pub event: String,
    pub body: Map<String, Value>,
    pub identity: PreparedHookIdentity,
}
impl PreparedHook {
    pub fn prepare(
        host: AdapterHost,
        event: &str,
        input: Value,
        workspace: &SourceWorkspace,
        received_at: OffsetDateTime,
    ) -> Result<Self, DevMapError> {
        let map = input.as_object().ok_or_else(|| {
            DevMapError::MalformedAdapterConfig("hook input must be one JSON object".into())
        })?;
        validate_body(map)?;
        let body = require_object(input)?;
        let session_id =
            identifier_field(&body, &["session_id"]).unwrap_or_else(|| "missing-session".into());
        let event_id = if source_identifier(event, &body).is_some() {
            stable_event_id(event, &session_id, &body)
        } else {
            crate::mutation::random_event_id("hook-invocation")?
        };
        let received_at = received_at.format(&Rfc3339)?;
        let prepared = Self {
            host: host.into(),
            event: event.into(),
            identity: PreparedHookIdentity {
                event_id,
                occurred_at: native_timestamp(&body).unwrap_or_else(|| received_at.clone()),
                received_at,
                branch: workspace.branch.clone(),
                head: workspace.head.clone(),
            },
            body,
        };
        prepared.normalized(workspace, 1)?;
        Ok(prepared)
    }

    fn normalized(
        &self,
        workspace: &SourceWorkspace,
        sequence: u64,
    ) -> Result<Vec<EventEnvelope>, DevMapError> {
        use crate::mutation::{bounded_text, parse_time, validate_observation};
        bounded_text("native hook event", &self.event, 512)?;
        validate_body(&self.body)?;
        bounded_text("native hook event id", &self.identity.event_id, 512)?;
        parse_time(&self.identity.occurred_at)?;
        parse_time(&self.identity.received_at)?;
        validate_observation(self.identity.branch.as_deref(), Some(&self.identity.head))?;
        let mut historical = workspace.clone();
        historical.branch = self.identity.branch.clone();
        historical.head = self.identity.head.clone();
        let mut context =
            NormalizedContext::from_input(self.host.into(), &self.event, &self.body, &historical)?;
        context.event_id = self.identity.event_id.clone();
        context.occurred_at = self.identity.occurred_at.clone();
        context.sequence = sequence;
        normalize_with_context(self.host.into(), &self.event, &self.body, context)
    }

    pub(crate) fn execute(
        &self,
        workspace: &SourceWorkspace,
    ) -> Result<crate::mutation::MutationResult, DevMapError> {
        // Validate before any journal open and again after crossing the wire.
        let validated = self.normalized(workspace, 1)?;
        let session_id = validated
            .first()
            .ok_or(DevMapError::InvalidDomain("empty hook capture"))?
            .context()
            .session_id()
            .to_owned();
        let journal = JournalStore::open(workspace, &session_id)?;
        let records = journal.append_capture_batch_with(
            crate::mutation::parse_time(&self.identity.received_at)?,
            |sequence| self.normalized(workspace, sequence),
        )?;
        Ok(crate::mutation::MutationResult::HookAccepted {
            sha256: records.into_iter().map(|record| record.sha256).collect(),
        })
    }
}

fn validate_body(input: &Map<String, Value>) -> Result<(), DevMapError> {
    // Check structure before recursive serialization/normalization. Inbound IPC
    // separately caps its frame and aggregate upload allocation.
    if input.len() > 16384 {
        return Err(DevMapError::ResourceLimit {
            resource: "native hook structure",
            limit: 16384,
        });
    }
    let mut pending: Vec<_> = input.values().map(|value| (value, 1usize)).collect();
    let mut text_bytes: usize = input.keys().map(String::len).sum();
    let mut nodes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        if depth > 32 || nodes > 16384 {
            return Err(DevMapError::ResourceLimit {
                resource: "native hook structure",
                limit: 16384,
            });
        }
        match value {
            Value::Array(values) => {
                if nodes + pending.len() + values.len() > 16384 {
                    return Err(DevMapError::ResourceLimit {
                        resource: "native hook array",
                        limit: 16384,
                    });
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if nodes + pending.len() + values.len() > 16384 {
                    return Err(DevMapError::ResourceLimit {
                        resource: "native hook object",
                        limit: 16384,
                    });
                }
                text_bytes += values.keys().map(String::len).sum::<usize>();
                pending.extend(values.values().map(|value| (value, depth + 1)));
            }
            Value::String(value) => text_bytes += value.len(),
            _ => {}
        }
        if text_bytes > MAX_HOOK_BODY_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "native hook body",
                limit: MAX_HOOK_BODY_BYTES,
            });
        }
    }
    if serde_json::to_vec(input)?.len() > MAX_HOOK_BODY_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "native hook body",
            limit: MAX_HOOK_BODY_BYTES,
        });
    }
    Ok(())
}

pub fn handle_hook(
    args: HookHandleArgs,
    stdin: &mut dyn Read,
) -> Result<CommandOutput, DevMapError> {
    let mut bytes = Vec::with_capacity(4096);
    stdin
        .take((MAX_HOOK_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_HOOK_BODY_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "native hook body",
            limit: MAX_HOOK_BODY_BYTES,
        });
    }
    let input = require_object(serde_json::from_slice(&bytes)?)?;
    let workspace = SourceGitInspector::open(&args.source)?.workspace()?;
    let session_id =
        identifier_field(&input, &["session_id"]).unwrap_or_else(|| "missing-session".to_owned());
    let journal = JournalStore::open(&workspace, &session_id)?;
    journal.append_capture_batch_with(OffsetDateTime::now_utc(), |next_sequence| {
        let mut sequenced_input = input;
        sequenced_input.insert("sequence".into(), Value::from(next_sequence));
        normalize_hook_input(
            args.host,
            &args.event,
            Value::Object(sequenced_input),
            &workspace,
        )
    })?;

    Ok(CommandOutput {
        stdout: "{}\n".to_owned(),
        exit_code: 0,
    })
}

/// Executable-only entrypoint. Embedded callers retain the direct handler above.
pub fn handle_hook_shared(
    args: HookHandleArgs,
    stdin: &mut dyn Read,
) -> Result<CommandOutput, DevMapError> {
    crate::git_process::with_operation(|| handle_hook_shared_inner(args, stdin))
}
fn handle_hook_shared_inner(
    args: HookHandleArgs,
    stdin: &mut dyn Read,
) -> Result<CommandOutput, DevMapError> {
    let mut bytes = Vec::with_capacity(4096);
    stdin
        .take((MAX_HOOK_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_HOOK_BODY_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "native hook body",
            limit: MAX_HOOK_BODY_BYTES,
        });
    }
    let input = serde_json::from_slice(&bytes)?;
    let workspace = SourceGitInspector::open(&args.source)?.workspace()?;
    let prepared = PreparedHook::prepare(
        args.host,
        &args.event,
        input,
        &workspace,
        OffsetDateTime::now_utc(),
    )?;
    let command = crate::mutation::MutationCommand::CaptureHook { prepared };
    let mut proxy = crate::mutation_proxy::MutationProxy::new(crate::proxy::ProxyMode::Shared);
    match proxy.execute(&workspace, &command)? {
        crate::mutation::MutationResult::HookAccepted { .. } => Ok(CommandOutput {
            stdout: "{}\n".into(),
            exit_code: 0,
        }),
        _ => Err(DevMapError::InvalidDomain("native hook mutation result")),
    }
}

pub fn normalize_hook_input(
    host: AdapterHost,
    event: &str,
    input: Value,
    workspace: &SourceWorkspace,
) -> Result<Vec<EventEnvelope>, DevMapError> {
    let input = require_object(input)?;
    let context = NormalizedContext::from_input(host, event, &input, workspace)?;
    normalize_with_context(host, event, &input, context)
}

fn normalize_with_context(
    host: AdapterHost,
    event: &str,
    input: &Map<String, Value>,
    context: NormalizedContext,
) -> Result<Vec<EventEnvelope>, DevMapError> {
    let Some(event_types) = event_types(host, event, input) else {
        return context.gap("unsupported_host_event", status_payload(event, input));
    };
    let Some(payload_event) = identifier_field(input, &["hook_event_name"]) else {
        return context.gap("missing_hook_event_name", status_payload(event, input));
    };
    let Some(payload_event) = canonical_event_name(&payload_event) else {
        return context.gap("invalid_hook_event_name", status_payload(event, input));
    };
    if canonical_event_name(event) != Some(payload_event) {
        return context.gap(
            "host_event_mismatch",
            json!({
                "cli_event": canonical_event_name(event).unwrap_or("unsupported"),
                "payload_event": payload_event,
            }),
        );
    }
    if context.missing_session {
        return context.gap("missing_mandatory_context", status_payload(event, input));
    }

    event_types
        .into_iter()
        .enumerate()
        .map(|(offset, event_type)| {
            context.envelope(
                event_type.clone(),
                offset as u64,
                event_payload(host, event, &event_type, input),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct NormalizedContext {
    event_id: String,
    sequence: u64,
    occurred_at: String,
    host_kind: AdapterHost,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
    missing_session: bool,
}

impl NormalizedContext {
    fn from_input(
        host: AdapterHost,
        event: &str,
        input: &Map<String, Value>,
        workspace: &SourceWorkspace,
    ) -> Result<Self, DevMapError> {
        let session_id = identifier_field(input, &["session_id"]);
        let missing_session = session_id.is_none();
        let session_id = session_id.unwrap_or_else(|| "missing-session".to_owned());
        let actor_id = identifier_field(input, &["agent_id"])
            .unwrap_or_else(|| format!("{}:{session_id}", host_name(host)));
        let supplied_parent = identifier_field(input, &["parent_agent_id", "parentAgentId"]);
        let parent = supplied_parent.or_else(|| {
            matches!(
                normalize_event_name(event).as_str(),
                "subagentstart" | "subagentstop"
            )
            .then(|| format!("{}:{session_id}", host_name(host)))
        });
        let occurred_at = native_timestamp(input).unwrap_or(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(DevMapError::from)?,
        );
        let sequence = input.get("sequence").and_then(Value::as_u64).unwrap_or(1);
        let event_id = stable_event_id(event, &session_id, input);
        let repository = workspace.root.to_string_lossy().into_owned();
        let context = SessionContext::new(
            session_id,
            None,
            repository.clone(),
            Some(repository),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )?;

        Ok(Self {
            event_id,
            sequence,
            occurred_at,
            host_kind: host,
            host: HostIdentity::new(host_name(host), ADAPTER_VERSION)?,
            actor: ActorIdentity::new(actor_id, parent)?,
            context,
            missing_session,
        })
    }

    fn envelope(
        &self,
        event_type: EventType,
        offset: u64,
        payload: Value,
    ) -> Result<EventEnvelope, DevMapError> {
        let event_id = if offset == 0 {
            self.event_id.clone()
        } else {
            format!("{}-{offset}", self.event_id)
        };
        EventEnvelope::new(
            EVENT_SCHEMA_VERSION,
            event_id,
            event_type,
            self.sequence + offset,
            self.occurred_at.clone(),
            self.host.clone(),
            self.actor.clone(),
            self.context.clone(),
            payload,
        )
    }

    fn gap(&self, reason: &str, detail: Value) -> Result<Vec<EventEnvelope>, DevMapError> {
        Ok(vec![self.envelope(
            EventType::CaptureGap,
            0,
            json!({
                "capture_grade": host_capabilities(self.host_kind).grade(),
                "reason": reason,
                "detail": detail,
            }),
        )?])
    }
}

fn event_types(
    host: AdapterHost,
    event: &str,
    input: &Map<String, Value>,
) -> Option<Vec<EventType>> {
    if host == AdapterHost::GenericMcp {
        return None;
    }
    match normalize_event_name(event).as_str() {
        "sessionstart" => Some(vec![EventType::SessionStarted]),
        "userpromptsubmit" => Some(vec![EventType::InstructionObserved]),
        "pretooluse" => Some(vec![EventType::ToolRequested]),
        "posttooluse" => {
            let mut events = vec![EventType::ToolCompleted];
            if write_capable(input) {
                events.push(EventType::CaptureGap);
            }
            Some(events)
        }
        "precompact" => Some(vec![EventType::ContextCompacting]),
        "postcompact" => Some(vec![EventType::ContextCompacted]),
        "subagentstart" => Some(vec![EventType::AgentStarted]),
        "subagentstop" => Some(vec![EventType::AgentStopped]),
        "stop" => Some(vec![EventType::TurnCompleted]),
        "sessionend" => Some(vec![EventType::SessionStopped]),
        _ => None,
    }
}

fn event_payload(
    host: AdapterHost,
    event: &str,
    event_type: &EventType,
    input: &Map<String, Value>,
) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "capture_grade".into(),
        json!(host_capabilities(host).grade()),
    );
    payload.insert(
        "activity".into(),
        Value::String(activity_name(event_type).to_owned()),
    );
    let status = status_payload(event, input);
    if status.as_object().is_some_and(|status| !status.is_empty()) {
        payload.insert("status".into(), status);
    }
    match event_type {
        EventType::InstructionObserved => {
            let digest = input
                .get("prompt")
                .and_then(Value::as_str)
                .map(|prompt| format!("sha256-{}", sha256_hex(prompt.as_bytes())));
            payload.insert(
                "instruction_activity".into(),
                json!({
                    "content_sha256": digest,
                    "content_stored": false,
                    "semantic_requirement": false,
                }),
            );
        }
        EventType::ToolRequested | EventType::ToolCompleted => {
            payload.insert("tool".into(), tool_activity(input));
        }
        EventType::CaptureGap => {
            payload.insert("reason".into(), Value::String("mutation_unverified".into()));
            payload.insert("tool".into(), tool_activity(input));
            payload.insert(
                "verification".into(),
                Value::String("before_after_state_unavailable".into()),
            );
        }
        _ => {}
    }
    Value::Object(payload)
}

fn activity_name(event_type: &EventType) -> &'static str {
    match event_type {
        EventType::SessionStarted => "session_started",
        EventType::SessionStopped => "session_stopped",
        EventType::TurnCompleted => "turn_completed",
        EventType::InstructionObserved => "prompt_submitted",
        EventType::AgentStarted => "agent_started",
        EventType::AgentStopped => "agent_stopped",
        EventType::ToolRequested => "tool_requested",
        EventType::ToolCompleted => "tool_completed",
        EventType::ContextCompacting => "context_compacting",
        EventType::ContextCompacted => "context_compacted",
        EventType::CaptureGap => "capture_gap",
        _ => "semantic_capture",
    }
}

fn tool_activity(input: &Map<String, Value>) -> Value {
    json!({
        "name": identifier_field(input, &["tool_name"]).unwrap_or_else(|| "unknown".into()),
        "tool_use_id": identifier_field(input, &["tool_use_id"]),
    })
}

fn status_payload(event: &str, input: &Map<String, Value>) -> Value {
    let mut status = Map::new();
    status.insert(
        "host_event".into(),
        Value::String(
            canonical_event_name(event)
                .unwrap_or("unsupported")
                .to_owned(),
        ),
    );
    if let Some(value) = enumerated_field(
        input,
        "permission_mode",
        &[
            "default",
            "acceptEdits",
            "plan",
            "auto",
            "dontAsk",
            "bypassPermissions",
        ],
    ) {
        status.insert("permission_mode".into(), Value::String(value));
    }
    if let Some(value) =
        enumerated_field(input, "source", &["startup", "resume", "clear", "compact"])
    {
        status.insert("source".into(), Value::String(value));
    }
    if let Some(value) = enumerated_field(input, "trigger", &["manual", "auto"]) {
        status.insert("trigger".into(), Value::String(value));
    }
    if let Some(value) = enumerated_field(
        input,
        "reason",
        &[
            "clear",
            "resume",
            "logout",
            "prompt_input_exit",
            "bypass_permissions_disabled",
            "other",
        ],
    ) {
        status.insert("reason".into(), Value::String(value));
    }
    if let Some(value) = identifier_field(input, &["agent_type"]) {
        status.insert("agent_type".into(), Value::String(value));
    }
    if let Some(value) = input.get("stop_hook_active").and_then(Value::as_bool) {
        status.insert("stop_hook_active".into(), Value::Bool(value));
    }
    Value::Object(status)
}

fn stable_event_id(event: &str, session_id: &str, input: &Map<String, Value>) -> String {
    let normalized = normalize_event_name(event);
    let source_identifier = source_identifier(event, input);
    let fallback = serde_json::to_vec(input).unwrap_or_default();
    let material = match source_identifier {
        Some(identifier) => format!("{normalized}\0{session_id}\0{identifier}").into_bytes(),
        None => {
            let mut material = format!("{normalized}\0{session_id}\0").into_bytes();
            material.extend_from_slice(&fallback);
            material
        }
    };
    format!("hook-{}", sha256_hex(&material))
}

fn source_identifier(event: &str, input: &Map<String, Value>) -> Option<String> {
    let normalized = normalize_event_name(event);
    identifier_field(input, &["event_id", "hook_event_id"]).or_else(|| match normalized.as_str() {
        "pretooluse" | "posttooluse" => identifier_field(input, &["tool_use_id"]),
        "subagentstart" | "subagentstop" => identifier_field(input, &["agent_id"]),
        "userpromptsubmit" | "precompact" | "postcompact" | "stop" => {
            identifier_field(input, &["turn_id", "prompt_id"])
        }
        _ => None,
    })
}

fn native_timestamp(input: &Map<String, Value>) -> Option<String> {
    ["occurred_at", "timestamp", "time"]
        .iter()
        .filter_map(|field| input.get(*field).and_then(Value::as_str))
        .find(|value| OffsetDateTime::parse(value, &Rfc3339).is_ok())
        .map(str::to_owned)
}

fn write_capable(input: &Map<String, Value>) -> bool {
    let Some(name) = identifier_field(input, &["tool_name"]) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    [
        "bash",
        "powershell",
        "shell",
        "exec",
        "write",
        "edit",
        "patch",
        "delete",
        "remove",
        "move",
        "rename",
        "mkdir",
        "apply",
    ]
    .iter()
    .any(|verb| name.contains(verb))
}

fn identifier_field(input: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        input
            .get(*name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .filter(|value| valid_identifier(value))
            .map(str::to_owned)
    })
}

fn enumerated_field(input: &Map<String, Value>, name: &str, allowed: &[&str]) -> Option<String> {
    input
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| allowed.contains(value))
        .map(str::to_owned)
}

fn valid_identifier(value: &str) -> bool {
    value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/' | b'@' | b'+')
        })
}

fn normalize_event_name(event: &str) -> String {
    event
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_event_name(event: &str) -> Option<&'static str> {
    match normalize_event_name(event).as_str() {
        "sessionstart" => Some("SessionStart"),
        "userpromptsubmit" => Some("UserPromptSubmit"),
        "pretooluse" => Some("PreToolUse"),
        "posttooluse" => Some("PostToolUse"),
        "precompact" => Some("PreCompact"),
        "postcompact" => Some("PostCompact"),
        "subagentstart" => Some("SubagentStart"),
        "subagentstop" => Some("SubagentStop"),
        "stop" => Some("Stop"),
        "sessionend" => Some("SessionEnd"),
        _ => None,
    }
}

fn require_object(input: Value) -> Result<Map<String, Value>, DevMapError> {
    input.as_object().cloned().ok_or_else(|| {
        DevMapError::MalformedAdapterConfig("hook input must be one JSON object".to_owned())
    })
}

fn host_name(host: AdapterHost) -> &'static str {
    match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => "generic_mcp",
    }
}
