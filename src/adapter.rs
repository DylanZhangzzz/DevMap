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
    validate_document(&document, &plan.config_path)?;
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
    validate_document(&document, &plan.config_path)?;
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
    validate_document(&document, &plan.config_path)?;
    let removed = remove_owned_bindings(&mut document)?;
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

fn validate_document(document: &Value, path: &Path) -> Result<(), DevMapError> {
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
                validate_handler(path, event, group_index, handler_index, handler)?;
            }
        }
    }
    Ok(())
}

fn validate_handler(
    path: &Path,
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
    let required_strings: &[&str] = match handler_type {
        "command" => &["command"],
        "http" => &["url"],
        "mcp_tool" => &["server", "tool"],
        "prompt" | "agent" => &["prompt"],
        _ => {
            return Err(malformed(
                path,
                format!("{location}.type is not recognized: {handler_type}"),
            ));
        }
    };
    for field in required_strings {
        if !handler.get(*field).is_some_and(Value::is_string) {
            return Err(malformed(
                path,
                format!("{location}.{field} must be a string"),
            ));
        }
    }
    Ok(())
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
    let mut group = Map::new();
    if let Some(matcher) = &binding.matcher {
        group.insert("matcher".into(), Value::String(matcher.clone()));
    }
    group.insert(
        "hooks".into(),
        Value::Array(vec![expected_handler(binding)]),
    );
    groups.push(Value::Object(group));
    Ok(())
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

fn remove_owned_bindings(document: &mut Value) -> Result<Vec<String>, DevMapError> {
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
            !(removed_from_group && handlers.is_empty())
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
    if temporary.exists() || backup.exists() {
        return Err(DevMapError::UnsafeInstallerOverwrite(
            if temporary.exists() {
                temporary
            } else {
                backup
            },
        ));
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);

    replace_config(path, &temporary, &backup, &bytes)?;
    if fs::read(path)? != bytes {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(())
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
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_config(
    path: &Path,
    temporary: &Path,
    backup: &Path,
    expected: &[u8],
) -> Result<(), DevMapError> {
    let backup_created = path.exists();
    if backup_created {
        fs::rename(path, backup)?;
    }
    if let Err(error) = fs::rename(temporary, path) {
        if backup_created {
            let _ = fs::rename(backup, path);
        }
        return Err(error.into());
    }
    if backup_created {
        let verified = matches!(fs::read(path), Ok(bytes) if bytes == expected);
        if !verified {
            let _ = fs::remove_file(path);
            let _ = fs::rename(backup, path);
            return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
        }
        fs::remove_file(backup)?;
    }
    Ok(())
}
