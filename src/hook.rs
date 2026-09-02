use std::io::Read;

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
use crate::presence::{PresenceSignal, PresenceStore};

const ADAPTER_VERSION: &str = "devmap-hook/1";
const MAX_IDENTIFIER_BYTES: usize = 512;
pub const MAX_HOOK_BODY_BYTES: usize = 1024 * 1024;

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
    let records = journal.append_batch_with(|next_sequence| {
        let mut sequenced_input = input;
        sequenced_input.insert("sequence".into(), Value::from(next_sequence));
        normalize_hook_input(
            args.host,
            &args.event,
            Value::Object(sequenced_input),
            &workspace,
        )
    })?;
    if let Err(error) = PresenceStore::open(&workspace).and_then(|store| {
        store
            .observe(
                PresenceSignal::AcceptedRecords(&records),
                OffsetDateTime::now_utc(),
            )
            .map(|_| ())
    }) {
        eprintln!("devmap: presence update skipped: {error}");
    }

    Ok(CommandOutput {
        stdout: "{}\n".to_owned(),
        exit_code: 0,
    })
}

pub fn normalize_hook_input(
    host: AdapterHost,
    event: &str,
    input: Value,
    workspace: &SourceWorkspace,
) -> Result<Vec<EventEnvelope>, DevMapError> {
    let input = require_object(input)?;
    let context = NormalizedContext::from_input(host, event, &input, workspace)?;
    let Some(event_types) = event_types(host, event, &input) else {
        return context.gap("unsupported_host_event", status_payload(event, &input));
    };
    let Some(payload_event) = identifier_field(&input, &["hook_event_name"]) else {
        return context.gap("missing_hook_event_name", status_payload(event, &input));
    };
    let Some(payload_event) = canonical_event_name(&payload_event) else {
        return context.gap("invalid_hook_event_name", status_payload(event, &input));
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
        return context.gap("missing_mandatory_context", status_payload(event, &input));
    }

    event_types
        .into_iter()
        .enumerate()
        .map(|(offset, event_type)| {
            context.envelope(
                event_type.clone(),
                offset as u64,
                event_payload(host, event, &event_type, &input),
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
    let source_identifier = identifier_field(input, &["event_id", "hook_event_id"]).or_else(|| {
        match normalized.as_str() {
            "pretooluse" | "posttooluse" => identifier_field(input, &["tool_use_id"]),
            "subagentstart" | "subagentstop" => identifier_field(input, &["agent_id"]),
            "userpromptsubmit" | "precompact" | "postcompact" | "stop" => {
                identifier_field(input, &["turn_id", "prompt_id"])
            }
            _ => None,
        }
    });
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
