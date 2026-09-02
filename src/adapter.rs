use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::cli::AdapterHost;
use crate::error::DevMapError;
use crate::events::{CaptureCapabilities, CaptureGrade, EventType};
use crate::git::SourceGitInspector;

const EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "SessionEnd",
];
const OWNED_BINDING_PREFIX: &str = "devmap/v1/";
const KERNEL_COMMAND_PATH: &str = "devmap hook handle";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterPlan {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub bindings: Vec<HookBinding>,
    pub capabilities: CaptureCapabilities,
    pub capture_grade: CaptureGrade,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookBinding {
    pub binding_id: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub present: Vec<String>,
    pub missing: Vec<String>,
    pub modified: Vec<String>,
    pub kernel_command_path: String,
    pub capabilities: CaptureCapabilities,
    pub capture_grade: CaptureGrade,
    pub drift_reasons: Vec<String>,
}

pub fn plan_adapter(source: &Path, host: AdapterHost) -> Result<AdapterPlan, DevMapError> {
    let source_root = SourceGitInspector::open(source)?.root().to_path_buf();
    let (host_name, relative_config) = host_details(host)?;
    let bindings = EVENTS
        .iter()
        .map(|event| {
            let binding_id = format!("devmap/v1/{host_name}/{event}");
            HookBinding {
                binding_id: binding_id.clone(),
                event: (*event).to_owned(),
                matcher: None,
                command: format!(
                    "devmap hook handle --host {host_name} --event {event} --binding-id {binding_id}"
                ),
            }
        })
        .collect();
    let capabilities = native_capabilities(host);
    let capture_grade = capabilities.grade();

    Ok(AdapterPlan {
        host,
        config_path: source_root.join(relative_config),
        bindings,
        capabilities,
        capture_grade,
    })
}

pub fn install_adapter(plan: AdapterPlan) -> Result<InstallReport, DevMapError> {
    ensure_plan_target(&plan)?;
    let mut document = read_config(&plan.config_path)?.unwrap_or_else(|| json!({}));
    validate_document(&document, &plan.config_path, plan.host)?;
    let existing = binding_occurrences(&document)?
        .into_iter()
        .map(|occurrence| occurrence.binding_id.to_owned())
        .collect::<BTreeSet<_>>();
    let mut added = Vec::new();
    let mut unchanged = Vec::new();

    for binding in &plan.bindings {
        if existing.contains(&binding.binding_id) {
            unchanged.push(binding.binding_id.clone());
            continue;
        }
        append_binding(&mut document, binding)?;
        added.push(binding.binding_id.clone());
    }

    let changed = !added.is_empty();
    if changed {
        write_config(&plan.config_path, &document)?;
    }
    Ok(InstallReport {
        host: plan.host,
        config_path: plan.config_path,
        added,
        removed: Vec::new(),
        unchanged,
        changed,
    })
}

pub fn verify_adapter(source: &Path, host: AdapterHost) -> Result<VerifyReport, DevMapError> {
    let plan = plan_adapter(source, host)?;
    let document = read_config(&plan.config_path)?.unwrap_or_else(|| json!({}));
    validate_document(&document, &plan.config_path, plan.host)?;
    let occurrences = binding_occurrences(&document)?;
    let mut present = Vec::new();
    let mut missing = Vec::new();
    let mut modified = Vec::new();

    for binding in &plan.bindings {
        let matching = occurrences
            .iter()
            .filter(|occurrence| occurrence.binding_id == binding.binding_id)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            missing.push(binding.binding_id.clone());
        } else if matching.len() == 1 && occurrence_matches(matching[0], binding) {
            present.push(binding.binding_id.clone());
        } else {
            modified.push(binding.binding_id.clone());
        }
    }

    let mut drift_reasons = Vec::new();
    if !missing.is_empty() {
        drift_reasons.push(format!("missing bindings: {}", missing.join(", ")));
    }
    if !modified.is_empty() {
        drift_reasons.push(format!("modified bindings: {}", modified.join(", ")));
    }
    let capture_grade = if drift_reasons.is_empty() {
        plan.capture_grade
    } else {
        CaptureGrade::D
    };

    Ok(VerifyReport {
        host: plan.host,
        config_path: plan.config_path,
        present,
        missing,
        modified,
        kernel_command_path: KERNEL_COMMAND_PATH.to_owned(),
        capabilities: plan.capabilities,
        capture_grade,
        drift_reasons,
    })
}

pub fn uninstall_adapter(source: &Path, host: AdapterHost) -> Result<InstallReport, DevMapError> {
    let plan = plan_adapter(source, host)?;
    ensure_plan_target(&plan)?;
    let Some(mut document) = read_config(&plan.config_path)? else {
        return Ok(InstallReport {
            host,
            config_path: plan.config_path,
            added: Vec::new(),
            removed: Vec::new(),
            unchanged: Vec::new(),
            changed: false,
        });
    };
    validate_document(&document, &plan.config_path, plan.host)?;
    let removed = remove_owned_bindings(&mut document, &plan.bindings)?;
    let changed = !removed.is_empty();
    if changed {
        write_config(&plan.config_path, &document)?;
    }

    Ok(InstallReport {
        host,
        config_path: plan.config_path,
        added: Vec::new(),
        removed,
        unchanged: Vec::new(),
        changed,
    })
}

fn host_details(host: AdapterHost) -> Result<(&'static str, &'static str), DevMapError> {
    match host {
        AdapterHost::Codex => Ok(("codex", ".codex/hooks.json")),
        AdapterHost::Claude => Ok(("claude", ".claude/settings.json")),
        AdapterHost::GenericMcp => Err(DevMapError::UnsupportedAdapterHost("generic-mcp".into())),
    }
}

fn native_capabilities(host: AdapterHost) -> CaptureCapabilities {
    CaptureCapabilities {
        lifecycle_events: vec![
            EventType::SessionStarted,
            EventType::SessionStopped,
            EventType::InstructionObserved,
            EventType::AgentStarted,
            EventType::AgentStopped,
            EventType::ToolRequested,
            EventType::ToolCompleted,
            EventType::MutationObserved,
            EventType::EvidenceRecorded,
            EventType::ContextCompacting,
            EventType::ContextCompacted,
        ],
        pre_mutation_blocking: true,
        subagent_lifecycle: true,
        workspace_rebind: host == AdapterHost::Codex,
        tool_results: true,
        commit_mapping: true,
        raw_transcript: false,
    }
}

fn ensure_plan_target(plan: &AdapterPlan) -> Result<(), DevMapError> {
    let (_, expected_relative) = host_details(plan.host)?;
    if !plan.config_path.ends_with(expected_relative) {
        return Err(DevMapError::UnsafeInstallerOverwrite(
            plan.config_path.clone(),
        ));
    }
    let source = plan
        .config_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(plan.config_path.clone()))?;
    let expected = plan_adapter(source, plan.host)?;
    if plan != &expected {
        return Err(DevMapError::UnsafeInstallerOverwrite(
            plan.config_path.clone(),
        ));
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<Option<Value>, DevMapError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            DevMapError::MalformedAdapterConfig(format!("{}: {error}", path.display()))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_document(document: &Value, path: &Path, host: AdapterHost) -> Result<(), DevMapError> {
    let root = document
        .as_object()
        .ok_or_else(|| malformed(path, "top level must be an object"))?;
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object()
        .ok_or_else(|| malformed(path, "hooks must be an object"))?;
    for (event, groups) in hooks {
        validate_event(path, host, event)?;
        let groups = groups
            .as_array()
            .ok_or_else(|| malformed(path, format!("hooks.{event} must be an array")))?;
        for (group_index, group) in groups.iter().enumerate() {
            let group = group.as_object().ok_or_else(|| {
                malformed(
                    path,
                    format!("hooks.{event}[{group_index}] must be an object"),
                )
            })?;
            if group
                .get("matcher")
                .is_some_and(|matcher| !matcher.is_string())
            {
                return Err(malformed(
                    path,
                    format!("hooks.{event}[{group_index}].matcher must be a string"),
                ));
            }
            let handlers = group
                .get("hooks")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    malformed(
                        path,
                        format!("hooks.{event}[{group_index}].hooks must be an array"),
                    )
                })?;
            for (handler_index, handler) in handlers.iter().enumerate() {
                validate_handler(path, host, event, group_index, handler_index, handler)?;
            }
        }
    }
    Ok(())
}

fn validate_handler(
    path: &Path,
    host: AdapterHost,
    event: &str,
    group_index: usize,
    handler_index: usize,
    handler: &Value,
) -> Result<(), DevMapError> {
    let location = format!("hooks.{event}[{group_index}].hooks[{handler_index}]");
    let handler = handler
        .as_object()
        .ok_or_else(|| malformed(path, format!("{location} must be an object")))?;
    let handler_type = handler
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(path, format!("{location}.type must be a string")))?;
    if !handler_type_supported(host, event, handler_type) {
        return Err(malformed(
            path,
            format!("{location}.type is not supported for {host:?} {event}: {handler_type}"),
        ));
    }
    let required_strings: &[&str] = match handler_type {
        "command" => &["command"],
        "http" => &["url"],
        "mcp_tool" => &["server", "tool"],
        "prompt" | "agent" => &["prompt"],
        _ => unreachable!("unsupported handler types are rejected above"),
    };
    for field in required_strings {
        if !handler.get(*field).is_some_and(Value::is_string) {
            return Err(malformed(
                path,
                format!("{location}.{field} must be a string"),
            ));
        }
    }
    validate_optional_string(path, handler, &location, "statusMessage")?;
    validate_optional_number(path, handler, &location, "timeout")?;
    match host {
        AdapterHost::Codex => validate_codex_handler(path, handler, &location, handler_type),
        AdapterHost::Claude => validate_claude_handler(path, handler, &location, handler_type),
        AdapterHost::GenericMcp => unreachable!("generic MCP has no native adapter config"),
    }
}

fn validate_event(path: &Path, host: AdapterHost, event: &str) -> Result<(), DevMapError> {
    let supported = match host {
        AdapterHost::Codex => matches!(
            event,
            "SessionStart"
                | "UserPromptSubmit"
                | "PreToolUse"
                | "PermissionRequest"
                | "PostToolUse"
                | "PreCompact"
                | "PostCompact"
                | "SubagentStart"
                | "SubagentStop"
                | "Stop"
                | "SessionEnd"
        ),
        AdapterHost::Claude => matches!(
            event,
            "Setup"
                | "SessionStart"
                | "UserPromptSubmit"
                | "UserPromptExpansion"
                | "PreToolUse"
                | "PermissionRequest"
                | "PermissionDenied"
                | "PostToolUse"
                | "PostToolUseFailure"
                | "PostToolBatch"
                | "SubagentStart"
                | "SubagentStop"
                | "TaskCreated"
                | "TaskCompleted"
                | "Stop"
                | "StopFailure"
                | "TeammateIdle"
                | "PreCompact"
                | "PostCompact"
                | "SessionEnd"
                | "Elicitation"
                | "ElicitationResult"
                | "WorktreeCreate"
                | "WorktreeRemove"
                | "Notification"
                | "ConfigChange"
                | "InstructionsLoaded"
                | "CwdChanged"
                | "FileChanged"
                | "DirectoryAdded"
                | "PreModelSwitch"
                | "PostModelSwitch"
                | "MessageDisplay"
        ),
        AdapterHost::GenericMcp => false,
    };
    if supported {
        Ok(())
    } else {
        Err(malformed(
            path,
            format!("hooks.{event} is not supported for {host:?}"),
        ))
    }
}

fn handler_type_supported(host: AdapterHost, event: &str, handler_type: &str) -> bool {
    match host {
        AdapterHost::Codex => {
            handler_type == "command" || (handler_type == "mcp_tool" && event != "SessionEnd")
        }
        AdapterHost::Claude => match event {
            "SessionStart" | "Setup" => matches!(handler_type, "command" | "mcp_tool"),
            "PermissionDenied"
            | "PermissionRequest"
            | "PostToolBatch"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "PreToolUse"
            | "Stop"
            | "SubagentStop"
            | "TaskCompleted"
            | "TaskCreated"
            | "TeammateIdle"
            | "UserPromptExpansion"
            | "UserPromptSubmit" => matches!(
                handler_type,
                "command" | "http" | "mcp_tool" | "prompt" | "agent"
            ),
            _ => matches!(handler_type, "command" | "http" | "mcp_tool"),
        },
        AdapterHost::GenericMcp => false,
    }
}

fn validate_codex_handler(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
) -> Result<(), DevMapError> {
    const KNOWN_FIELDS: &[&str] = &[
        "command",
        "commandWindows",
        "additionalContextLimit",
        "async",
        "server",
        "tool",
        "input",
    ];
    match handler_type {
        "command" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &[
                    "command",
                    "commandWindows",
                    "additionalContextLimit",
                    "async",
                ],
            )?;
            validate_optional_string(path, handler, location, "commandWindows")?;
            validate_optional_bool(path, handler, location, "async")?;
            validate_optional_unsigned_integer(path, handler, location, "additionalContextLimit")
        }
        "mcp_tool" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["server", "tool", "input"],
            )?;
            validate_optional_object(path, handler, location, "input")
        }
        _ => unreachable!("Codex handler type was validated"),
    }
}

fn validate_claude_handler(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
) -> Result<(), DevMapError> {
    const KNOWN_FIELDS: &[&str] = &[
        "command",
        "args",
        "async",
        "asyncRewake",
        "shell",
        "url",
        "headers",
        "allowedEnvVars",
        "server",
        "tool",
        "input",
        "prompt",
        "model",
        "continueOnBlock",
    ];
    validate_optional_string(path, handler, location, "if")?;
    validate_optional_bool(path, handler, location, "once")?;
    match handler_type {
        "command" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["command", "args", "async", "asyncRewake", "shell"],
            )?;
            validate_optional_string_array(path, handler, location, "args")?;
            validate_optional_bool(path, handler, location, "async")?;
            validate_optional_bool(path, handler, location, "asyncRewake")?;
            validate_optional_shell(path, handler, location)
        }
        "http" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["url", "headers", "allowedEnvVars"],
            )?;
            validate_optional_string_map(path, handler, location, "headers")?;
            validate_optional_string_array(path, handler, location, "allowedEnvVars")
        }
        "mcp_tool" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["server", "tool", "input"],
            )?;
            validate_optional_object(path, handler, location, "input")
        }
        "prompt" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["prompt", "model", "continueOnBlock"],
            )?;
            validate_optional_string(path, handler, location, "model")?;
            validate_optional_bool(path, handler, location, "continueOnBlock")
        }
        "agent" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["prompt", "model"],
            )?;
            validate_optional_string(path, handler, location, "model")
        }
        _ => unreachable!("Claude handler type was validated"),
    }
}

fn validate_allowed_type_fields(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
    known_fields: &[&str],
    allowed_fields: &[&str],
) -> Result<(), DevMapError> {
    if let Some(field) = known_fields
        .iter()
        .find(|field| handler.contains_key(**field) && !allowed_fields.contains(field))
    {
        Err(malformed(
            path,
            format!("{location}.{field} is not supported for type {handler_type}"),
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_shell(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        "shell",
        "bash or powershell",
        |value| matches!(value.as_str(), Some("bash" | "powershell")),
    )
}

fn validate_optional(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
    expected: &str,
    predicate: impl FnOnce(&Value) -> bool,
) -> Result<(), DevMapError> {
    if handler.get(field).is_none_or(predicate) {
        Ok(())
    } else {
        Err(malformed(
            path,
            format!("{location}.{field} must be {expected}"),
        ))
    }
}

fn validate_optional_string(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(path, handler, location, field, "a string", Value::is_string)
}

fn validate_optional_number(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(path, handler, location, field, "a number", Value::is_number)
}

fn validate_optional_bool(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "a boolean",
        Value::is_boolean,
    )
}

fn validate_optional_object(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an object",
        Value::is_object,
    )
}

fn validate_optional_unsigned_integer(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "a non-negative integer",
        |value| value.as_u64().is_some(),
    )
}

fn validate_optional_string_array(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an array of strings",
        |value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
        },
    )
}

fn validate_optional_string_map(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an object of strings",
        |value| {
            value
                .as_object()
                .is_some_and(|entries| entries.values().all(Value::is_string))
        },
    )
}

fn malformed(path: &Path, reason: impl AsRef<str>) -> DevMapError {
    DevMapError::MalformedAdapterConfig(format!("{}: {}", path.display(), reason.as_ref()))
}

fn append_binding(document: &mut Value, binding: &HookBinding) -> Result<(), DevMapError> {
    let root = document
        .as_object_mut()
        .expect("validated adapter document is an object");
    if !root.contains_key("hooks") {
        root.insert("hooks".into(), Value::Object(Map::new()));
    }
    let hooks = root["hooks"]
        .as_object_mut()
        .expect("validated hooks value is an object");
    if !hooks.contains_key(&binding.event) {
        hooks.insert(binding.event.clone(), Value::Array(Vec::new()));
    }
    let groups = hooks[&binding.event]
        .as_array_mut()
        .expect("validated hook event is an array");
    groups.push(expected_group(binding));
    Ok(())
}

fn expected_group(binding: &HookBinding) -> Value {
    let mut group = Map::new();
    if let Some(matcher) = &binding.matcher {
        group.insert("matcher".into(), Value::String(matcher.clone()));
    }
    group.insert(
        "hooks".into(),
        Value::Array(vec![expected_handler(binding)]),
    );
    Value::Object(group)
}

fn expected_handler(binding: &HookBinding) -> Value {
    json!({
        "type": "command",
        "command": binding.command,
        "statusMessage": "Recording DevMap lifecycle",
    })
}

#[derive(Debug)]
struct BindingOccurrence<'a> {
    binding_id: &'a str,
    event: &'a str,
    matcher: Option<&'a str>,
    handler: &'a Value,
}

fn binding_occurrences(document: &Value) -> Result<Vec<BindingOccurrence<'_>>, DevMapError> {
    let Some(hooks) = document.get("hooks").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut occurrences = Vec::new();
    for (event, groups) in hooks {
        for group in groups.as_array().expect("validated hook event is an array") {
            let matcher = group.get("matcher").and_then(Value::as_str);
            for handler in group["hooks"]
                .as_array()
                .expect("validated handler list is an array")
            {
                if let Some(binding_id) = handler_binding_id(handler) {
                    occurrences.push(BindingOccurrence {
                        binding_id,
                        event,
                        matcher,
                        handler,
                    });
                }
            }
        }
    }
    Ok(occurrences)
}

fn occurrence_matches(occurrence: &BindingOccurrence<'_>, binding: &HookBinding) -> bool {
    occurrence.event == binding.event
        && occurrence.matcher == binding.matcher.as_deref()
        && occurrence.handler == &expected_handler(binding)
}

fn handler_binding_id(handler: &Value) -> Option<&str> {
    let mut words = handler.get("command")?.as_str()?.split_ascii_whitespace();
    if words.next()? != "devmap" || words.next()? != "hook" || words.next()? != "handle" {
        return None;
    }
    while let Some(word) = words.next() {
        if word == "--binding-id" {
            return words.next();
        }
        if let Some(binding_id) = word.strip_prefix("--binding-id=") {
            return Some(binding_id);
        }
    }
    None
}

fn remove_owned_bindings(
    document: &mut Value,
    bindings: &[HookBinding],
) -> Result<Vec<String>, DevMapError> {
    let Some(hooks) = document.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(Vec::new());
    };
    let mut removed = Vec::new();
    let events = hooks.keys().cloned().collect::<Vec<_>>();
    for event in events {
        let groups = hooks[&event]
            .as_array_mut()
            .expect("validated hook event is an array");
        let mut removed_from_event = false;
        groups.retain_mut(|group| {
            let devmap_generated = bindings
                .iter()
                .filter(|binding| binding.event == event)
                .any(|binding| group == &expected_group(binding));
            let handlers = group["hooks"]
                .as_array_mut()
                .expect("validated handler list is an array");
            let mut removed_from_group = false;
            handlers.retain(|handler| {
                let Some(binding_id) = handler_binding_id(handler) else {
                    return true;
                };
                if !binding_id.starts_with(OWNED_BINDING_PREFIX) {
                    return true;
                }
                removed.push(binding_id.to_owned());
                removed_from_group = true;
                removed_from_event = true;
                false
            });
            !(devmap_generated && removed_from_group && handlers.is_empty())
        });
        if removed_from_event && groups.is_empty() {
            hooks.remove(&event);
        }
    }
    removed.sort();
    Ok(removed)
}

fn write_config(path: &Path, document: &Value) -> Result<(), DevMapError> {
    ensure_local_target(path)?;
    let mut bytes = serde_json::to_vec_pretty(document)?;
    bytes.push(b'\n');
    let temporary = suffixed_path(path, ".devmap-tmp")?;
    let backup = suffixed_path(path, ".devmap-backup")?;
    let stale_artifact = if path_entry_exists(&temporary)? {
        Some(temporary.clone())
    } else if path_entry_exists(&backup)? {
        Some(backup.clone())
    } else {
        None
    };
    if let Some(stale_artifact) = stale_artifact {
        return Err(DevMapError::UnsafeInstallerOverwrite(stale_artifact));
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        return Err(transaction_error(path, error, &temporary));
    }
    drop(file);

    replace_config(path, &temporary, &backup, &bytes)
}

fn path_entry_exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_local_target(path: &Path) -> Result<(), DevMapError> {
    let parent = path
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let root = parent
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?
        .canonicalize()?;
    let resolved_parent = parent.canonicalize()?;
    if !resolved_parent.starts_with(root) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(())
}

fn suffixed_path(path: &Path, suffix: &str) -> Result<PathBuf, DevMapError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}

#[cfg(not(windows))]
fn replace_config(
    path: &Path,
    temporary: &Path,
    _backup: &Path,
    _expected: &[u8],
) -> Result<(), DevMapError> {
    commit_temporary_with(path, temporary, fs::rename)
}

#[cfg(windows)]
fn replace_config(
    path: &Path,
    temporary: &Path,
    backup: &Path,
    expected: &[u8],
) -> Result<(), DevMapError> {
    commit_temporary_with(path, temporary, |temporary, path| {
        windows_atomic_replace(temporary, path, backup, expected)
    })
}

fn commit_temporary_with(
    path: &Path,
    temporary: &Path,
    replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), DevMapError> {
    if let Err(replace_error) = replace(temporary, path) {
        return Err(transaction_error(path, replace_error, temporary));
    }
    Ok(())
}

fn transaction_error(
    path: &Path,
    operation_error: std::io::Error,
    temporary: &Path,
) -> DevMapError {
    let cleanup = match remove_temporary_file(temporary) {
        Ok(()) => "succeeded".to_owned(),
        Err(cleanup_error) => format!("failed: {cleanup_error}"),
    };
    DevMapError::AdapterConfigTransaction {
        path: path.to_path_buf(),
        operation_error: operation_error.to_string(),
        cleanup,
    }
}

fn remove_temporary_file(temporary: &Path) -> std::io::Result<()> {
    if !temporary
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".devmap-tmp"))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to clean a non-DevMap temporary path",
        ));
    }
    match fs::symlink_metadata(temporary) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(temporary)
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "DevMap temporary path is not a file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(any(windows, test))]
fn finalize_verified_replacement(
    path: &Path,
    expected: &[u8],
    restore: impl FnOnce() -> std::io::Result<()>,
    cleanup_backup: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let verification_error = match fs::read(path) {
        Ok(actual) if actual == expected => None,
        Ok(_) => Some(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "named adapter config did not match the serialized bytes",
        )),
        Err(error) => Some(std::io::Error::other(format!(
            "reading back the named adapter config failed: {error}"
        ))),
    };
    if let Some(verification_error) = verification_error {
        return match restore() {
            Ok(()) => Err(verification_error),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "{verification_error}; restoring the original failed: {restore_error}"
            ))),
        };
    }

    if let Err(cleanup_error) = cleanup_backup() {
        return match restore() {
            Ok(()) => Err(std::io::Error::other(format!(
                "cleaning replacement backup failed and the replacement was rolled back: {cleanup_error}"
            ))),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "cleaning replacement backup failed: {cleanup_error}; restoring the original failed: {restore_error}"
            ))),
        };
    }
    Ok(())
}

#[cfg(windows)]
fn windows_atomic_replace(
    temporary: &Path,
    path: &Path,
    backup: &Path,
    expected: &[u8],
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut core::ffi::c_void,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn move_file(source: &Path, destination: &Path, replace: bool) -> std::io::Result<()> {
        let source = wide(source);
        let destination = wide(destination);
        let flags = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        // SAFETY: both path pointers refer to owned NUL-terminated buffers.
        let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
        if moved == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    if !path.try_exists()? {
        move_file(temporary, path, false)?;
        return finalize_verified_replacement(
            path,
            expected,
            || move_file(path, temporary, false),
            || Ok(()),
        );
    }

    let temporary_wide = wide(temporary);
    let path_wide = wide(path);
    let backup_wide = wide(backup);
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain
    // alive for the call; the remaining optional pointers are documented null values.
    let replaced = unsafe {
        ReplaceFileW(
            path_wide.as_ptr(),
            temporary_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        let replace_error = std::io::Error::last_os_error();
        if backup.try_exists()?
            && let Err(restore_error) = move_file(backup, path, true)
        {
            return Err(std::io::Error::other(format!(
                "{replace_error}; restoring the original from {} failed: {restore_error}",
                backup.display()
            )));
        }
        return Err(replace_error);
    }

    finalize_verified_replacement(
        path,
        expected,
        || move_file(backup, path, true),
        || fs::remove_file(backup),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn replacement_failure_preserves_original_and_cleans_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        let temporary = root.path().join("hooks.json.devmap-tmp");
        fs::write(&path, b"original").unwrap();
        fs::write(&temporary, b"replacement").unwrap();

        let error = commit_temporary_with(&path, &temporary, |_, _| {
            Err(std::io::Error::other("injected replacement failure"))
        })
        .expect_err("the injected replacement failure must propagate");

        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(!temporary.exists());
        assert!(error.to_string().contains("injected replacement failure"));
    }

    #[test]
    fn replacement_failure_reports_a_temporary_cleanup_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        let temporary = root.path().join("hooks.json.devmap-tmp");
        fs::write(&path, b"original").unwrap();
        fs::write(&temporary, b"replacement").unwrap();

        let error = commit_temporary_with(&path, &temporary, |temporary, _| {
            fs::remove_file(temporary).unwrap();
            fs::create_dir(temporary).unwrap();
            Err(std::io::Error::other("injected replacement failure"))
        })
        .expect_err("both replacement and cleanup failures must propagate");

        assert_eq!(fs::read(&path).unwrap(), b"original");
        let rendered = error.to_string();
        assert!(rendered.contains("injected replacement failure"));
        assert!(rendered.contains("cleanup"));
    }

    #[test]
    fn replacement_readback_mismatch_restores_before_backup_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::write(&path, b"unexpected replacement").unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                fs::write(&path, b"original")
            },
            || {
                cleanup_called.set(true);
                Ok(())
            },
        )
        .expect_err("a byte mismatch must fail the transaction");

        assert!(restored.get());
        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(error.to_string().contains("did not match"));
    }

    #[test]
    fn replacement_readback_failure_restores_before_backup_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::create_dir(&path).unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                fs::remove_dir(&path)?;
                fs::write(&path, b"original")
            },
            || {
                cleanup_called.set(true);
                Ok(())
            },
        )
        .expect_err("a read-back failure must fail the transaction");

        assert!(restored.get());
        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(error.to_string().contains("reading back"));
    }

    #[test]
    fn replacement_backup_cleanup_runs_only_after_exact_readback() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::write(&path, b"expected replacement").unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                Ok(())
            },
            || {
                assert_eq!(fs::read(&path).unwrap(), b"expected replacement");
                cleanup_called.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!restored.get());
        assert!(cleanup_called.get());
    }

    #[test]
    fn replacement_readback_restore_failure_preserves_the_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        let backup = root.path().join("hooks.json.devmap-backup");
        fs::write(&path, b"unexpected replacement").unwrap();
        fs::write(&backup, b"original").unwrap();
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || Err(std::io::Error::other("injected restore failure")),
            || {
                cleanup_called.set(true);
                fs::remove_file(&backup)
            },
        )
        .expect_err("a restore failure must propagate");

        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert!(error.to_string().contains("injected restore failure"));
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_replacement_failure_keeps_the_named_original() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::write(&path, b"original").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();

        let error = write_config(&path, &json!({"hooks": {}}))
            .expect_err("an exclusive Windows handle must prevent replacement");
        drop(lock);

        assert!(matches!(
            error,
            DevMapError::AdapterConfigTransaction { .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(!root.path().join("hooks.json.devmap-tmp").exists());
        assert!(!root.path().join("hooks.json.devmap-backup").exists());
    }
}
