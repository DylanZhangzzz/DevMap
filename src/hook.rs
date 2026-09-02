use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{Map, Value, json};

use crate::CommandOutput;
use crate::capture::{CaptureKernel, RequirementTraceInput};
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
    let input: Value = serde_json::from_reader(stdin)?;
    let mut input = require_object(input)?;
    let workspace = SourceGitInspector::open(&args.source)?.workspace()?;
    let session_id = string_field(&input, session_id_fields(args.host))
        .unwrap_or_else(|| "missing-session".to_owned());
    let journal = JournalStore::open(&workspace, &session_id)?;
    let next_sequence = journal.replay()?.len() as u64 + 1;
    input.insert("sequence".into(), Value::from(next_sequence));

    let events = normalize_hook_input(args.host, &args.event, Value::Object(input), &workspace)?;
    let mut recorded_event_ids = Vec::with_capacity(events.len());
    for event in events {
        let record = if event.event_type() == &EventType::InstructionObserved {
            record_instruction(&journal, &event)?
        } else if event.event_type() == &EventType::CaptureGap {
            record_gap(&journal, &event, &workspace)?
        } else {
            journal.append(event)?
        };
        recorded_event_ids.push(record.event.event_id().to_owned());
    }

    let has_gap = events_have_gap(&recorded_event_ids, &journal)?;
    let status = json!({
        "continue": true,
        "captured": !recorded_event_ids.is_empty() && !has_gap,
        "capture_gap_recorded": has_gap,
        "event_ids": recorded_event_ids,
    });
    Ok(CommandOutput {
        stdout: format!("{}\n", serde_json::to_string(&status)?),
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
    let context = NormalizedContext::from_input(host, &input, workspace);
    if context.missing_mandatory_context() {
        return context.gap("missing_mandatory_context", metadata_for(host, &input));
    }
    if is_user_prompt_event(event) && approved_quotation(&input).is_none() {
        return context.gap("missing_mandatory_context", metadata_for(host, &input));
    }

    let metadata = metadata_for(host, &input);
    let event_types = event_types(host, event, &input);
    let Some(event_types) = event_types else {
        return context.gap("unsupported_host_event", metadata);
    };

    event_types
        .into_iter()
        .enumerate()
        .map(|(offset, event_type)| {
            let payload = event_payload(&event_type, &input, metadata.clone(), &workspace.head);
            context.envelope(event_type, offset as u64, payload)
        })
        .collect()
}

fn record_instruction(
    journal: &JournalStore,
    event: &EventEnvelope,
) -> Result<crate::journal::JournalRecord, DevMapError> {
    let trace = &event.payload()["requirement_trace"];
    let source_kind = trace["source"]["kind"]
        .as_str()
        .unwrap_or("human_instruction");
    let source_locator = trace["source"]["locator"].as_str().map(str::to_owned);
    let quotation = trace["approved_quotation"].as_str().unwrap_or_default();
    let kernel = CaptureKernel::new(
        journal.clone(),
        CaptureGrade::A,
        event.host().clone(),
        event.actor().clone(),
        event.context().clone(),
    );
    kernel.record_requirement(
        event.event_id(),
        event.occurred_at(),
        RequirementTraceInput {
            source_kind: source_kind.to_owned(),
            source_locator,
            quoted_text: quotation.to_owned(),
        },
        false,
    )
}

fn record_gap(
    journal: &JournalStore,
    event: &EventEnvelope,
    workspace: &SourceWorkspace,
) -> Result<crate::journal::JournalRecord, DevMapError> {
    let reason = event.payload()["reason"]
        .as_str()
        .unwrap_or("capture_unavailable");
    let kernel = CaptureKernel::new(
        journal.clone(),
        CaptureGrade::A,
        event.host().clone(),
        event.actor().clone(),
        event.context().clone(),
    );
    kernel.record_gap(
        event.event_id(),
        event.occurred_at(),
        reason,
        &format!("workspace:{}", workspace.head),
    )
}

fn events_have_gap(
    recorded_event_ids: &[String],
    journal: &JournalStore,
) -> Result<bool, DevMapError> {
    let records = journal.replay()?;
    Ok(records.iter().any(|record| {
        recorded_event_ids
            .iter()
            .any(|id| id == record.event.event_id())
            && record.event.event_type() == &EventType::CaptureGap
    }))
}

#[derive(Debug, Clone)]
struct NormalizedContext {
    event_id: String,
    sequence: u64,
    occurred_at: String,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
    missing: bool,
}

impl NormalizedContext {
    fn from_input(
        host: AdapterHost,
        input: &Map<String, Value>,
        workspace: &SourceWorkspace,
    ) -> Self {
        let session_id = string_field(input, session_id_fields(host));
        let agent_id = string_field(input, agent_id_fields(host));
        let occurred_at = string_field(input, &["occurred_at", "timestamp", "time"]);
        let missing = session_id.is_none() || agent_id.is_none() || occurred_at.is_none();
        let session_id = session_id.unwrap_or_else(|| "missing-session".to_owned());
        let agent_id = agent_id.unwrap_or_else(|| "missing-agent".to_owned());
        let occurred_at = occurred_at.unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());
        let parent_agent_id = string_field(input, parent_agent_id_fields(host));
        let sequence = input.get("sequence").and_then(Value::as_u64).unwrap_or(1);
        let event_id = string_field(input, event_id_fields(host))
            .unwrap_or_else(|| format!("hook-{session_id}-{sequence}"));
        let repository = workspace.root.to_string_lossy().to_string();
        let worktree = Some(repository.clone());
        let context = SessionContext::new(
            session_id,
            string_field(input, &["route_id", "routeId"]),
            repository,
            worktree,
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .expect("workspace-derived hook context must be valid");
        Self {
            event_id,
            sequence,
            occurred_at,
            host: HostIdentity::new(host_name(host), ADAPTER_VERSION)
                .expect("static hook identity must be valid"),
            actor: ActorIdentity::new(agent_id, parent_agent_id)
                .expect("normalized hook actor must be valid"),
            context,
            missing,
        }
    }

    fn missing_mandatory_context(&self) -> bool {
        self.missing
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
    let normalized = event.trim().to_ascii_lowercase();
    let event_types = match normalized.as_str() {
        "sessionstart" | "session_start" => vec![EventType::SessionStarted],
        "userpromptsubmit" | "user_prompt_submit" => {
            if approved_quotation(input).is_some() {
                vec![EventType::InstructionObserved]
            } else {
                return None;
            }
        }
        "pretooluse" | "pre_tool_use" => vec![EventType::ToolRequested],
        "posttooluse" | "post_tool_use" => {
            let mut events = vec![EventType::ToolCompleted];
            if write_capable(input) {
                events.push(EventType::MutationObserved);
                events.push(EventType::CaptureGap);
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
                        "kind": string_field(input, &["source_kind"]).unwrap_or_else(|| "human_instruction".to_owned()),
                        "locator": string_field(input, &["source_locator"]),
                    },
                    "approved_quotation": approved_quotation(input).unwrap_or_default(),
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
            truncate(key, MAX_METADATA_STRING_BYTES),
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
        Value::String(value) => Value::String(truncate(value, MAX_METADATA_STRING_BYTES)),
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
                .take(MAX_METADATA_ITEMS)
                .map(|(key, value)| {
                    (
                        truncate(key, MAX_METADATA_STRING_BYTES),
                        bounded_value(value, depth + 1),
                    )
                })
                .collect(),
        ),
        value => value.clone(),
    }
}

fn truncate(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    value
        .char_indices()
        .take_while(|(index, _)| *index < max_bytes)
        .map(|(_, character)| character)
        .collect()
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
        "approved_quotation",
        "source_kind",
        "source_locator",
        "tool_name",
        "toolName",
    ];
    if host == AdapterHost::Codex {
        fields.extend(["thread_id", "threadId"]);
    }
    fields
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["prompt", "transcript", "message", "content"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn approved_quotation(input: &Map<String, Value>) -> Option<String> {
    string_field(input, &["approved_quotation", "approvedQuotation"])
}

fn is_user_prompt_event(event: &str) -> bool {
    matches!(
        event.trim().to_ascii_lowercase().as_str(),
        "userpromptsubmit" | "user_prompt_submit"
    )
}

fn write_capable(input: &Map<String, Value>) -> bool {
    let Some(name) = string_field(input, &["tool_name", "toolName"]) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    [
        "write", "edit", "patch", "delete", "remove", "move", "rename", "mkdir", "apply",
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
        AdapterHost::Claude => &["sessionId", "session_id"],
        AdapterHost::GenericMcp => &["session_id"],
    }
}

fn agent_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["agent_id", "subagent_id", "thread_id"],
        AdapterHost::Claude => &["agentId", "agent_id", "subagent_id"],
        AdapterHost::GenericMcp => &["agent_id"],
    }
}

fn parent_agent_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["parent_agent_id"],
        AdapterHost::Claude => &["parentAgentId", "parent_agent_id"],
        AdapterHost::GenericMcp => &["parent_agent_id"],
    }
}

fn event_id_fields(host: AdapterHost) -> &'static [&'static str] {
    match host {
        AdapterHost::Codex => &["event_id", "hook_event_id"],
        AdapterHost::Claude => &["hook_event_id", "event_id"],
        AdapterHost::GenericMcp => &["event_id"],
    }
}
