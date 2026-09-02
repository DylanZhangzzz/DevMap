use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::capture::{AgentDecisionInput, CaptureKernel, EvidenceInput, RequirementTraceInput};
use crate::error::DevMapError;
use crate::events::{ActorIdentity, CaptureGrade, HostIdentity, SessionContext};
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::JournalStore;

pub const MCP_TOOLS: [&str; 4] = [
    "devmap_context",
    "devmap_record_requirement",
    "devmap_record_decision",
    "devmap_record_evidence",
];

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MCP_ADAPTER_VERSION: &str = "devmap-mcp/1";
const GENERIC_DESCRIPTOR_PATH: &str = ".devmap/mcp.json";

pub fn serve_mcp(
    source: &Path,
    reader: impl BufRead,
    mut writer: impl Write,
) -> Result<(), DevMapError> {
    for line in reader.lines() {
        let line = line?;
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(source, &message),
            Err(error) => Some(json_rpc_error(
                Value::Null,
                -32700,
                "Parse error",
                Some(json!({"detail": error.to_string()})),
            )),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn handle_message(source: &Path, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(json_rpc_error(Value::Null, -32600, "Invalid Request", None));
    };
    let id = object.get("id").cloned();
    let valid_version = object.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    let valid_method = object.get("method").is_some_and(Value::is_string);
    let is_notification = id.is_none() && valid_version && valid_method;
    if !valid_version || !valid_method || id.as_ref().is_some_and(|id| !valid_request_id(id)) {
        return (!is_notification).then(|| {
            json_rpc_error(
                id.filter(valid_request_id).unwrap_or(Value::Null),
                -32600,
                "Invalid Request",
                None,
            )
        });
    }
    if is_notification {
        return None;
    }

    let id = id.expect("requests have an ID");
    let method = object["method"].as_str().expect("method was validated");
    let params = object.get("params");
    Some(match method {
        "initialize" => initialize_response(id, params),
        "tools/list" => list_tools_response(id, params),
        "tools/call" => call_tool_response(source, id, params),
        _ => json_rpc_error(id, -32601, "Method not found", None),
    })
}

fn valid_request_id(id: &Value) -> bool {
    id.is_string() || id.is_number()
}

fn initialize_response(id: Value, params: Option<&Value>) -> Value {
    let Some(params) = params.and_then(Value::as_object) else {
        return invalid_params(id, "initialize params must be an object");
    };
    let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
        return invalid_params(id, "protocolVersion must be a string");
    };
    if requested != MCP_PROTOCOL_VERSION {
        return json_rpc_error(
            id,
            -32602,
            "Unsupported protocol version",
            Some(json!({
                "supported": [MCP_PROTOCOL_VERSION],
                "requested": requested,
            })),
        );
    }
    if !params.get("capabilities").is_some_and(Value::is_object)
        || !params.get("clientInfo").is_some_and(Value::is_object)
    {
        return invalid_params(id, "capabilities and clientInfo must be objects");
    }
    json_rpc_result(
        id,
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "devmap",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn list_tools_response(id: Value, params: Option<&Value>) -> Value {
    if params.is_some_and(|params| !params.is_object()) {
        return invalid_params(id, "tools/list params must be an object");
    }
    if params
        .and_then(Value::as_object)
        .and_then(|params| params.get("cursor"))
        .is_some_and(|cursor| !cursor.is_string())
    {
        return invalid_params(id, "cursor must be a string");
    }
    json_rpc_result(id, json!({"tools": tool_descriptors()}))
}

fn call_tool_response(source: &Path, id: Value, params: Option<&Value>) -> Value {
    let Some(params) = params.and_then(Value::as_object) else {
        return invalid_params(id, "tools/call params must be an object");
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return invalid_params(id, "tools/call name must be a string");
    };
    let arguments = match params.get("arguments") {
        None => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => return invalid_params(id, "tools/call arguments must be an object"),
    };
    if !MCP_TOOLS.contains(&name) {
        return invalid_params(id, format!("Unknown tool: {name}"));
    }

    match call_tool(source, name, &arguments) {
        Ok(structured) => json_rpc_result(id, tool_result(structured)),
        Err(error) => json_rpc_result(id, tool_error(error)),
    }
}

fn call_tool(
    source: &Path,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<Value, DevMapError> {
    let workspace = SourceGitInspector::open(source)?.workspace()?;
    if name == "devmap_context" {
        ensure_fields(arguments, &[])?;
        return Ok(json!({
            "workspace": workspace.root.to_string_lossy(),
            "branch": workspace.branch,
            "head": workspace.head,
            "journal_location": workspace.git_dir.join("devmap/sessions").to_string_lossy(),
            "capture_grade": "C",
        }));
    }

    let common = CommonCaptureArgs::parse(arguments, name, &workspace)?;
    let journal = JournalStore::open(&workspace, &common.session_id)?;
    let kernel = CaptureKernel::new(
        journal,
        CaptureGrade::C,
        HostIdentity::new("generic_mcp", MCP_ADAPTER_VERSION)?,
        ActorIdentity::new(common.agent_id, common.parent_agent_id)?,
        SessionContext::new(
            common.session_id,
            common.route_id,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch,
            Some(workspace.head),
        )?,
    );

    let record = match name {
        "devmap_record_requirement" => kernel.record_requirement(
            &common.event_id,
            &common.occurred_at,
            RequirementTraceInput {
                source_kind: required_string(arguments, "source_kind")?,
                source_locator: optional_string(arguments, "source_locator")?,
                quoted_text: required_string(arguments, "quoted_text")?,
            },
            optional_bool(arguments, "raw_transcript_opt_in")?.unwrap_or(false),
        )?,
        "devmap_record_decision" => kernel.record_decision(
            &common.event_id,
            &common.occurred_at,
            AgentDecisionInput {
                decision: required_string(arguments, "decision")?,
                basis: required_string_array(arguments, "basis")?,
                alternatives: required_string_array(arguments, "alternatives")?,
                rationale: required_string(arguments, "rationale")?,
                scope: required_string(arguments, "scope")?,
                authority: required_string(arguments, "authority")?,
                revisit_trigger: required_string(arguments, "revisit_trigger")?,
            },
        )?,
        "devmap_record_evidence" => kernel.record_evidence(
            &common.event_id,
            &common.occurred_at,
            EvidenceInput {
                kind: required_string(arguments, "kind")?,
                target: required_string(arguments, "target")?,
                command: optional_string(arguments, "command")?,
                outcome: required_string(arguments, "outcome")?,
            },
        )?,
        _ => unreachable!("tool names were checked before dispatch"),
    };
    Ok(json!({"sha256": record.sha256}))
}

struct CommonCaptureArgs {
    session_id: String,
    agent_id: String,
    parent_agent_id: Option<String>,
    route_id: Option<String>,
    event_id: String,
    occurred_at: String,
}

impl CommonCaptureArgs {
    fn parse(
        arguments: &Map<String, Value>,
        tool: &str,
        workspace: &SourceWorkspace,
    ) -> Result<Self, DevMapError> {
        ensure_fields(arguments, allowed_fields(tool))?;
        let session_id = required_string(arguments, "session_id")?;
        let agent_id = required_string(arguments, "agent_id")?;
        let parent_agent_id = optional_string(arguments, "parent_agent_id")?;
        let route_id = optional_string(arguments, "route_id")?;
        let sequence = JournalStore::open(workspace, &session_id)?.replay()?.len() as u64 + 1;
        let event_id = optional_string(arguments, "event_id")?
            .unwrap_or_else(|| format!("mcp-{tool}-{session_id}-{sequence}"));
        let occurred_at = optional_string(arguments, "occurred_at")?
            .map(Ok)
            .unwrap_or_else(|| OffsetDateTime::now_utc().format(&Rfc3339))?;
        Ok(Self {
            session_id,
            agent_id,
            parent_agent_id,
            route_id,
            event_id,
            occurred_at,
        })
    }
}

fn allowed_fields(tool: &str) -> &'static [&'static str] {
    const REQUIREMENT: &[&str] = &[
        "session_id",
        "agent_id",
        "parent_agent_id",
        "route_id",
        "event_id",
        "occurred_at",
        "source_kind",
        "source_locator",
        "quoted_text",
        "raw_transcript_opt_in",
    ];
    const DECISION: &[&str] = &[
        "session_id",
        "agent_id",
        "parent_agent_id",
        "route_id",
        "event_id",
        "occurred_at",
        "decision",
        "basis",
        "alternatives",
        "rationale",
        "scope",
        "authority",
        "revisit_trigger",
    ];
    const EVIDENCE: &[&str] = &[
        "session_id",
        "agent_id",
        "parent_agent_id",
        "route_id",
        "event_id",
        "occurred_at",
        "kind",
        "target",
        "command",
        "outcome",
    ];
    match tool {
        "devmap_record_requirement" => REQUIREMENT,
        "devmap_record_decision" => DECISION,
        "devmap_record_evidence" => EVIDENCE,
        _ => &[],
    }
}

fn ensure_fields(arguments: &Map<String, Value>, allowed: &[&str]) -> Result<(), DevMapError> {
    if arguments
        .keys()
        .any(|field| !allowed.contains(&field.as_str()))
    {
        return Err(DevMapError::InvalidDomain("unexpected tool argument"));
    }
    Ok(())
}

fn required_string(
    arguments: &Map<String, Value>,
    field: &'static str,
) -> Result<String, DevMapError> {
    match arguments.get(field).and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_owned()),
        _ => Err(DevMapError::InvalidDomain(field)),
    }
}

fn optional_string(
    arguments: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, DevMapError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        _ => Err(DevMapError::InvalidDomain(field)),
    }
}

fn optional_bool(
    arguments: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, DevMapError> {
    match arguments.get(field) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(DevMapError::InvalidDomain(field)),
    }
}

fn required_string_array(
    arguments: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, DevMapError> {
    arguments
        .get(field)
        .and_then(Value::as_array)
        .ok_or(DevMapError::InvalidDomain(field))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .ok_or(DevMapError::InvalidDomain(field))
        })
        .collect()
}

fn tool_descriptors() -> Vec<Value> {
    vec![
        tool_descriptor(
            "devmap_context",
            "Return the current source workspace and local capture context.",
            &[],
            json!({}),
        ),
        tool_descriptor(
            "devmap_record_requirement",
            "Record an explicitly approved requirement quotation.",
            &["session_id", "agent_id", "source_kind", "quoted_text"],
            json!({
                "session_id": string_schema("Capture session identifier."),
                "agent_id": string_schema("Calling agent identifier."),
                "parent_agent_id": string_schema("Optional parent agent identifier."),
                "route_id": string_schema("Optional route identifier."),
                "event_id": string_schema("Optional stable event identifier."),
                "occurred_at": string_schema("Optional RFC 3339 event timestamp."),
                "source_kind": string_schema("Kind of requirement source."),
                "source_locator": string_schema("Optional source locator."),
                "quoted_text": string_schema("Explicitly supplied approved quotation."),
                "raw_transcript_opt_in": {"type": "boolean"}
            }),
        ),
        tool_descriptor(
            "devmap_record_decision",
            "Record a fully justified agent decision.",
            &[
                "session_id",
                "agent_id",
                "decision",
                "basis",
                "alternatives",
                "rationale",
                "scope",
                "authority",
                "revisit_trigger",
            ],
            json!({
                "session_id": string_schema("Capture session identifier."),
                "agent_id": string_schema("Calling agent identifier."),
                "parent_agent_id": string_schema("Optional parent agent identifier."),
                "route_id": string_schema("Optional route identifier."),
                "event_id": string_schema("Optional stable event identifier."),
                "occurred_at": string_schema("Optional RFC 3339 event timestamp."),
                "decision": string_schema("Decision made."),
                "basis": string_array_schema(),
                "alternatives": string_array_schema(),
                "rationale": string_schema("Decision rationale."),
                "scope": string_schema("Scope of the decision."),
                "authority": string_schema("Authority for the decision."),
                "revisit_trigger": string_schema("Condition that requires reconsideration.")
            }),
        ),
        tool_descriptor(
            "devmap_record_evidence",
            "Record validated evidence without mutating source Git state.",
            &["session_id", "agent_id", "kind", "target", "outcome"],
            json!({
                "session_id": string_schema("Capture session identifier."),
                "agent_id": string_schema("Calling agent identifier."),
                "parent_agent_id": string_schema("Optional parent agent identifier."),
                "route_id": string_schema("Optional route identifier."),
                "event_id": string_schema("Optional stable event identifier."),
                "occurred_at": string_schema("Optional RFC 3339 event timestamp."),
                "kind": string_schema("Evidence kind."),
                "target": string_schema("commit:, artifact:, or workspace: digest target."),
                "command": string_schema("Optional command that produced the evidence."),
                "outcome": string_schema("Evidence outcome.")
            }),
        ),
    ]
}

fn tool_descriptor(name: &str, description: &str, required: &[&str], properties: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn string_schema(description: &str) -> Value {
    json!({"type": "string", "minLength": 1, "description": description})
}

fn string_array_schema() -> Value {
    json!({"type": "array", "items": {"type": "string", "minLength": 1}, "minItems": 1})
}

fn tool_result(structured: Value) -> Value {
    let text = structured.to_string();
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
    })
}

fn tool_error(error: DevMapError) -> Value {
    json!({
        "content": [{"type": "text", "text": error.to_string()}],
        "isError": true
    })
}

fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn invalid_params(id: Value, message: impl Into<String>) -> Value {
    json_rpc_error(id, -32602, message, None)
}

fn json_rpc_error(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message.into()});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

pub fn plan_generic_adapter(source: &Path) -> Result<CommandOutput, DevMapError> {
    let path = generic_config_path(source)?;
    Ok(CommandOutput {
        stdout: format!(
            "host=generic-mcp\nconfig_path={}\ndescriptor={}\ncapture_grade=C\n",
            path.display(),
            generic_descriptor()
        ),
        exit_code: 0,
    })
}

pub fn install_generic_adapter(source: &Path) -> Result<CommandOutput, DevMapError> {
    let path = generic_config_path(source)?;
    let changed = match read_generic_config(&path)? {
        Some(document) => {
            ensure_named_local_target(&path, false)?;
            validate_generic_descriptor(&path, &document)?;
            false
        }
        None => {
            write_new_generic_config(&path)?;
            true
        }
    };
    Ok(CommandOutput {
        stdout: format!(
            "host=generic-mcp\nconfig_path={}\nchanged={changed}\nadded={}\n",
            path.display(),
            if changed { "descriptor" } else { "" }
        ),
        exit_code: 0,
    })
}

pub fn verify_generic_adapter(source: &Path) -> Result<CommandOutput, DevMapError> {
    let path = generic_config_path(source)?;
    let present = match read_generic_config(&path)? {
        Some(document) => {
            validate_generic_descriptor(&path, &document)?;
            true
        }
        None => false,
    };
    Ok(CommandOutput {
        stdout: format!(
            "host=generic-mcp\nconfig_path={}\nkernel_command_path=devmap mcp\npresent={}\nmissing={}\nmodified=\ncapture_grade={}\ndrift_reason={}\n",
            path.display(),
            if present { "descriptor" } else { "" },
            if present { "" } else { "descriptor" },
            if present { "C" } else { "D" },
            if present { "" } else { "missing descriptor" }
        ),
        exit_code: if present { 0 } else { 1 },
    })
}

pub fn uninstall_generic_adapter(source: &Path) -> Result<CommandOutput, DevMapError> {
    let path = generic_config_path(source)?;
    let changed = match read_generic_config(&path)? {
        Some(document) => {
            validate_generic_descriptor(&path, &document)?;
            ensure_named_local_target(&path, false)?;
            fs::remove_file(&path)?;
            true
        }
        None => false,
    };
    Ok(CommandOutput {
        stdout: format!(
            "host=generic-mcp\nconfig_path={}\nchanged={changed}\nremoved={}\n",
            path.display(),
            if changed { "descriptor" } else { "" }
        ),
        exit_code: 0,
    })
}

fn generic_descriptor() -> Value {
    json!({
        "command": ["devmap", "mcp", "--source", "."],
        "transport": "stdio"
    })
}

fn generic_config_path(source: &Path) -> Result<PathBuf, DevMapError> {
    Ok(SourceGitInspector::open(source)?
        .root()
        .join(GENERIC_DESCRIPTOR_PATH))
}

fn read_generic_config(path: &Path) -> Result<Option<Value>, DevMapError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            DevMapError::MalformedAdapterConfig(format!("{}: {error}", path.display()))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_generic_descriptor(path: &Path, document: &Value) -> Result<(), DevMapError> {
    if document == &generic_descriptor() {
        Ok(())
    } else {
        Err(DevMapError::MalformedAdapterConfig(format!(
            "{}: existing Generic MCP descriptor is not DevMap's exact descriptor",
            path.display()
        )))
    }
}

fn write_new_generic_config(path: &Path) -> Result<(), DevMapError> {
    ensure_named_local_target(path, true)?;
    let temporary = path.with_file_name("mcp.json.devmap-tmp");
    let backup = path.with_file_name("mcp.json.devmap-backup");
    for artifact in [&temporary, &backup] {
        if fs::symlink_metadata(artifact).is_ok() {
            return Err(DevMapError::UnsafeInstallerOverwrite(artifact.clone()));
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&generic_descriptor())?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let cleanup = match fs::remove_file(&temporary) {
            Ok(()) => "succeeded".to_owned(),
            Err(cleanup) => format!("failed: {cleanup}"),
        };
        return Err(DevMapError::AdapterConfigTransaction {
            path: path.to_path_buf(),
            operation_error: error.to_string(),
            cleanup,
        });
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let cleanup = match fs::remove_file(&temporary) {
            Ok(()) => "succeeded".to_owned(),
            Err(cleanup) => format!("failed: {cleanup}"),
        };
        return Err(DevMapError::AdapterConfigTransaction {
            path: path.to_path_buf(),
            operation_error: error.to_string(),
            cleanup,
        });
    }
    if fs::read(path)? != bytes {
        return Err(DevMapError::AdapterConfigTransaction {
            path: path.to_path_buf(),
            operation_error: "named adapter config did not match serialized bytes".into(),
            cleanup: "not attempted after named write".into(),
        });
    }
    Ok(())
}

fn ensure_named_local_target(path: &Path, create_parent: bool) -> Result<(), DevMapError> {
    if path.file_name().and_then(|name| name.to_str()) != Some("mcp.json")
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some(".devmap")
    {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    let root = parent
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?
        .canonicalize()?;
    if create_parent {
        fs::create_dir_all(parent)?;
    }
    let resolved_parent = parent.canonicalize()?;
    if resolved_parent.parent() != Some(root.as_path()) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(())
}
