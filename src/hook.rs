use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{Map, Value, json};
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::canonical::sha256_hex;
use crate::cli::{AdapterHost, HookHandleArgs};
use crate::error::DevMapError;
use crate::events::{
    ActorIdentity, CaptureGrade, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity,
    SessionContext,
};
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::JournalStore;

const ADAPTER_VERSION: &str = "devmap-hook/1";
const MAX_METADATA_FIELDS: usize = 16;
const MAX_METADATA_DEPTH: usize = 4;
const MAX_METADATA_ITEMS: usize = 8;
const MAX_METADATA_STRING_BYTES: usize = 1024;

pub fn handle_hook(
    args: HookHandleArgs,
    stdin: &mut dyn Read,
) -> Result<CommandOutput, DevMapError> {
    let input = require_object(serde_json::from_reader(stdin)?)?;
    let workspace = SourceGitInspector::open(&args.source)?.workspace()?;
    let session_id = string_field(&input, session_id_fields(args.host))
        .unwrap_or_else(|| "missing-session".to_owned());
    let journal = JournalStore::open(&workspace, &session_id)?;
    journal.append_batch_with(|next_sequence| {
        let mut sequenced_input = input;
        sequenced_input.insert("sequence".into(), Value::from(next_sequence));
        normalize_hook_input(
            args.host,
            &args.event,
            Value::Object(sequenced_input),
            &workspace,
        )
    })?;

    // Native hooks must receive only their documented response schema. Both hosts accept an
    // empty object for non-blocking lifecycle notifications.
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
    let context = NormalizedContext::from_input(host, &input, workspace)?;
    let Some(event_types) = event_types(host, event, &input) else {
        return context.gap("unsupported_host_event", metadata_for(host, &input));
    };
    if context.missing_session {
        return context.gap("missing_mandatory_context", metadata_for(host, &input));
    }

    let metadata = metadata_for(host, &input);
    event_types
        .into_iter()
        .enumerate()
        .map(|(offset, event_type)| {
            context.envelope(
                event_type.clone(),
                offset as u64,
                event_payload(
                    &event_type,
                    &input,
                    metadata.clone(),
                    &context,
                    &workspace.head,
                ),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct NormalizedContext {
    event_id: String,
    sequence: u64,
    occurred_at: String,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
    missing_session: bool,
}

impl NormalizedContext {
    fn from_input(
        host: AdapterHost,
        input: &Map<String, Value>,
        workspace: &SourceWorkspace,
    ) -> Result<Self, DevMapError> {
        let session_id = string_field(input, session_id_fields(host));
        let missing_session = session_id.is_none();
        let session_id = session_id.unwrap_or_else(|| "missing-session".to_owned());
        let actor_id = string_field(input, agent_id_fields(host))
            .unwrap_or_else(|| format!("{}:{session_id}", host_name(host)));
        let occurred_at = match string_field(input, &["occurred_at", "timestamp", "time"]) {
            Some(value) => value,
            None => time::OffsetDateTime::now_utc().format(&Rfc3339)?,
        };
        let sequence = input.get("sequence").and_then(Value::as_u64).unwrap_or(1);
        let event_id = string_field(input, event_id_fields(host))
            .unwrap_or_else(|| format!("hook-{}-{session_id}-{sequence}", host_name(host)));
        let repository = workspace.root.to_string_lossy().to_string();
        let context = SessionContext::new(
            session_id,
            string_field(input, &["route_id", "routeId"]),
            repository.clone(),
            Some(repository),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )?;

        Ok(Self {
            event_id,
            sequence,
            occurred_at,
            host: HostIdentity::new(host_name(host), ADAPTER_VERSION)?,
            actor: ActorIdentity::new(actor_id, string_field(input, parent_agent_id_fields(host)))?,
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

    fn gap(&self, reason: &str, metadata: Value) -> Result<Vec<EventEnvelope>, DevMapError> {
        Ok(vec![self.envelope(
            EventType::CaptureGap,
            0,
            json!({
                "capture_grade": CaptureGrade::A,
                "reason": reason,
                "host_metadata": metadata,
            }),
        )?])
    }
}

fn event_types(
    host: AdapterHost,
    event: &str,
    input: &Map<String, Value>,
) -> Option<Vec<EventType>> {
    let event_types = match event.trim().to_ascii_lowercase().as_str() {
        "sessionstart" | "session_start" => vec![EventType::SessionStarted],
        "userpromptsubmit" | "user_prompt_submit" => vec![EventType::InstructionObserved],
        "pretooluse" | "pre_tool_use" => vec![EventType::ToolRequested],
        "posttooluse" | "post_tool_use" => {
            let mut events = vec![EventType::ToolCompleted];
            if write_capable(input) {
                events.extend([EventType::MutationObserved, EventType::CaptureGap]);
            }
            events
        }
        "precompact" | "pre_compact" => vec![EventType::ContextCompacting],
        "postcompact" | "post_compact" => vec![EventType::ContextCompacted],
        "subagentstart" | "subagent_start" => vec![EventType::AgentStarted],
        "subagentstop" | "subagent_stop" => vec![EventType::AgentStopped],
        "stop" | "sessionend" | "session_end" => vec![EventType::SessionStopped],
        _ => return None,
    };
    match host {
        AdapterHost::Codex | AdapterHost::Claude => Some(event_types),
        AdapterHost::GenericMcp => None,
    }
}

fn event_payload(
    event_type: &EventType,
    input: &Map<String, Value>,
    metadata: Value,
    context: &NormalizedContext,
    head: &str,
) -> Value {
    let mut payload = Map::new();
    payload.insert("capture_grade".into(), json!(CaptureGrade::A));
    payload.insert("host_metadata".into(), metadata);
    match event_type {
        EventType::InstructionObserved => {
            payload.insert(
                "requirement_trace".into(),
                json!({
                    "source": {
                        "kind": "host_prompt_reference",
                        "locator": format!("{}:session:{}:event:{}", context.host.name(), context.context.session_id(), context.event_id),
                    },
                    "content_digest": prompt_digest(input, context),
                    "content_stored": false,
                }),
            );
        }
        EventType::ToolRequested | EventType::ToolCompleted => {
            payload.insert(
                "tool".into(),
                json!({"name": string_field(input, &["tool_name", "toolName"]).unwrap_or_else(|| "unknown".to_owned())}),
            );
        }
        EventType::MutationObserved => {
            payload.insert(
                "mutation_target".into(),
                Value::String(format!("workspace:{head}")),
            );
            payload.insert(
                "observed_tool".into(),
                Value::String(
                    string_field(input, &["tool_name", "toolName"])
                        .unwrap_or_else(|| "unknown".to_owned()),
                ),
            );
        }
        EventType::CaptureGap => {
            payload.insert(
                "reason".into(),
                Value::String("unexplained_mutation".into()),
            );
            payload.insert(
                "mutation_target".into(),
                Value::String(format!("workspace:{head}")),
            );
        }
        _ => {}
    }
    Value::Object(payload)
}

fn prompt_digest(input: &Map<String, Value>, context: &NormalizedContext) -> String {
    let source = ["prompt", "user_prompt", "text"]
        .iter()
        .find_map(|name| input.get(*name).and_then(Value::as_str))
        .map(str::as_bytes)
        .unwrap_or_else(|| context.event_id.as_bytes());
    format!("sha256-{}", sha256_hex(source))
}

fn metadata_for(host: AdapterHost, input: &Map<String, Value>) -> Value {
    let known = known_fields(host);
    let mut metadata = BTreeMap::new();
    for (key, value) in input {
        if known.contains(&key.as_str())
            || sensitive_key(key)
            || metadata.len() == MAX_METADATA_FIELDS
        {
            continue;
        }
        metadata.insert(
            truncate_utf8(key, MAX_METADATA_STRING_BYTES),
            bounded_value(value, 0),
        );
    }
    Value::Object(metadata.into_iter().collect())
}

fn bounded_value(value: &Value, depth: usize) -> Value {
    if depth >= MAX_METADATA_DEPTH {
        return Value::String("[truncated]".into());
    }
    match value {
        Value::Number(number) if number.is_f64() => Value::String("[non-canonical-number]".into()),
        Value::String(value) => Value::String(truncate_utf8(value, MAX_METADATA_STRING_BYTES)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(MAX_METADATA_ITEMS)
                .map(|value| bounded_value(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .filter(|(key, _)| !sensitive_key(key))
                .take(MAX_METADATA_ITEMS)
                .map(|(key, value)| {
                    (
                        truncate_utf8(key, MAX_METADATA_STRING_BYTES),
                        bounded_value(value, depth + 1),
                    )
                })
                .collect(),
        ),
        value => value.clone(),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = 0;
    for character in value.chars() {
        let next = end + character.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    value[..end].to_owned()
}

fn known_fields(host: AdapterHost) -> Vec<&'static str> {
    let mut fields = vec![
        "event_id",
        "hook_event_id",
        "session_id",
        "sessionId",
        "agent_id",
        "agentId",
        "subagent_id",
        "parent_agent_id",
        "parentAgentId",
        "occurred_at",
        "timestamp",
        "time",
        "route_id",
        "routeId",
        "sequence",
        "tool_name",
        "toolName",
    ];
    if host == AdapterHost::Codex {
        fields.extend(["thread_id", "threadId"]);
    }
    fields
}

fn sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "prompt",
        "transcript",
        "message",
        "content",
        "toolresponse",
        "compactsummary",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn write_capable(input: &Map<String, Value>) -> bool {
    let Some(name) = string_field(input, &["tool_name", "toolName"]) else {
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

fn string_field(input: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        input
            .get(*name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
    })
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

fn session_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["session_id", "thread_id", "threadId"],
        AdapterHost::Claude => &["session_id", "sessionId"],
        AdapterHost::GenericMcp => &["session_id"],
    }
}

fn agent_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["agent_id", "subagent_id"],
        AdapterHost::Claude => &["agent_id", "agentId", "subagent_id"],
        AdapterHost::GenericMcp => &["agent_id"],
    }
}

fn parent_agent_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["parent_agent_id"],
        AdapterHost::Claude => &["parent_agent_id", "parentAgentId"],
        AdapterHost::GenericMcp => &["parent_agent_id"],
    }
}

fn event_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex | AdapterHost::Claude => &["event_id", "hook_event_id"],
        AdapterHost::GenericMcp => &["event_id"],
    }
}
