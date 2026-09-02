use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::capture::{AgentDecisionInput, CaptureKernel, EvidenceInput, RequirementTraceInput};
use crate::error::DevMapError;
use crate::events::{ActorIdentity, HostIdentity, SessionContext, host_capabilities};
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::JournalStore;

pub const MCP_TOOLS: [&str; 4] = [
    "devmap_context",
    "devmap_record_requirement",
    "devmap_record_decision",
    "devmap_record_evidence",
];

const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const MCP_ADAPTER_VERSION: &str = "devmap-mcp/1";
pub const MAX_MCP_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_MCP_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_MCP_ARGUMENT_BYTES: usize = 256 * 1024;
pub const MAX_SEMANTIC_STRING_BYTES: usize = 16 * 1024;
pub const MAX_SEMANTIC_ARRAY_ITEMS: usize = 64;
const MAX_IDENTIFIER_BYTES: usize = 512;

pub fn serve_mcp(
    source: &Path,
    mut reader: impl BufRead,
    mut writer: impl Write,
) -> Result<(), DevMapError> {
    // Workspace identity is stable for the lifetime of this stdio process. Resolving it once
    // avoids spawning multiple Git commands for every semantic capture.
    let workspace = SourceGitInspector::open(source)?.workspace()?;
    let mut legacy_initialized = false;
    loop {
        let response = match read_bounded_line(&mut reader)? {
            None => break,
            Some(Ok(line)) => match serde_json::from_slice::<Value>(&line) {
                Ok(message) => handle_message(&workspace, &message, &mut legacy_initialized),
                Err(error) => Some(json_rpc_error(
                    Value::Null,
                    -32700,
                    "Parse error",
                    Some(json!({"detail": error.to_string()})),
                )),
            },
            Some(Err(())) => Some(json_rpc_error(
                Value::Null,
                -32600,
                "MCP line exceeds the configured byte limit",
                Some(json!({"resource": "MCP line", "limit": MAX_MCP_LINE_BYTES})),
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

fn read_bounded_line(
    reader: &mut impl BufRead,
) -> Result<Option<Result<Vec<u8>, ()>>, DevMapError> {
    let mut line = Vec::with_capacity(4096);
    let mut exceeded = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() && !exceeded {
                return Ok(None);
            }
            if exceeded {
                return Ok(Some(Err(())));
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(Ok(line)));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content = newline.map_or(available, |index| &available[..index]);
        if !exceeded {
            if line.len().saturating_add(content.len()) > MAX_MCP_LINE_BYTES {
                exceeded = true;
                line.clear();
            } else {
                line.extend_from_slice(content);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            if exceeded {
                return Ok(Some(Err(())));
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(Ok(line)));
        }
    }
}

fn handle_message(
    workspace: &SourceWorkspace,
    message: &Value,
    legacy_initialized: &mut bool,
) -> Option<Value> {
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
    if method == "initialize" {
        let response = initialize_response(id, params);
        if response.get("result").is_some() {
            *legacy_initialized = true;
        }
        return Some(response);
    }

    let era = match request_era(method, params, *legacy_initialized) {
        Ok(era) => era,
        Err(error) => return Some(error.with_id(id)),
    };
    let response = match method {
        "server/discover" if era == RequestEra::Modern => json_rpc_result(id, discovery_result()),
        "server/discover" => invalid_params(id, "server/discover requires modern request metadata"),
        "tools/list" => list_tools_response(id, params),
        "tools/call" => call_tool_response(workspace, id, params),
        _ => json_rpc_error(id, -32601, "Method not found", None),
    };
    Some(if era == RequestEra::Modern {
        add_modern_result_fields(method, response)
    } else {
        response
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestEra {
    Legacy,
    Modern,
}

struct PendingError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl PendingError {
    fn with_id(self, id: Value) -> Value {
        json_rpc_error(id, self.code, self.message, self.data)
    }
}

fn request_era(
    method: &str,
    params: Option<&Value>,
    legacy_initialized: bool,
) -> Result<RequestEra, PendingError> {
    let metadata = params
        .and_then(Value::as_object)
        .and_then(|params| params.get("_meta"))
        .and_then(Value::as_object);
    let requested =
        metadata.and_then(|metadata| metadata.get("io.modelcontextprotocol/protocolVersion"));
    let Some(requested) = requested else {
        if legacy_initialized && method != "server/discover" {
            return Ok(RequestEra::Legacy);
        }
        return Err(PendingError {
            code: -32602,
            message: "modern request metadata is required before legacy initialization".into(),
            data: None,
        });
    };
    let Some(requested) = requested.as_str() else {
        return Err(PendingError {
            code: -32602,
            message: "modern protocolVersion metadata is required".into(),
            data: None,
        });
    };
    if requested != MODERN_PROTOCOL_VERSION {
        return Err(PendingError {
            code: -32022,
            message: "Unsupported protocol version".into(),
            data: Some(json!({
                "supported": [MODERN_PROTOCOL_VERSION],
                "requested": requested,
            })),
        });
    }
    let metadata = metadata.expect("modern protocol metadata came from an object");
    validate_modern_metadata(metadata)?;
    Ok(RequestEra::Modern)
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
    let Some(capabilities) = params.get("capabilities").and_then(Value::as_object) else {
        return invalid_params(id, "capabilities must be an object");
    };
    if let Err(message) = validate_capabilities(capabilities) {
        return invalid_params(id, message);
    }
    let Some(client_info) = params.get("clientInfo").and_then(Value::as_object) else {
        return invalid_params(id, "clientInfo must be an object");
    };
    if let Err(message) = validate_implementation(client_info) {
        return invalid_params(id, message);
    }
    // Legacy MCP requires a successful supported-version counteroffer when the requested
    // initialize version is unsupported. The client may accept it or disconnect.
    let _counteroffered = requested != LEGACY_PROTOCOL_VERSION;
    json_rpc_result(
        id,
        json!({
            "protocolVersion": LEGACY_PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "devmap",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn discovery_result() -> Value {
    json!({
        "supportedVersions": [MODERN_PROTOCOL_VERSION],
        "capabilities": {"tools": {}}
    })
}

fn validate_modern_metadata(metadata: &Map<String, Value>) -> Result<(), PendingError> {
    let encoded = serde_json::to_vec(metadata).map_err(|_| invalid_metadata("invalid metadata"))?;
    if encoded.len() > MAX_MCP_METADATA_BYTES {
        return Err(invalid_metadata(format!(
            "modern metadata exceeds {MAX_MCP_METADATA_BYTES} bytes"
        )));
    }
    if let Some(client_info) = metadata.get("io.modelcontextprotocol/clientInfo") {
        let client_info = client_info
            .as_object()
            .ok_or_else(|| invalid_metadata("modern clientInfo must be an object"))?;
        validate_implementation(client_info).map_err(invalid_metadata)?;
    }
    let capabilities = metadata
        .get("io.modelcontextprotocol/clientCapabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_metadata("modern clientCapabilities must be an object"))?;
    validate_capabilities(capabilities).map_err(invalid_metadata)?;
    if let Some(log_level) = metadata.get("io.modelcontextprotocol/logLevel")
        && !matches!(
            log_level.as_str(),
            Some(
                "debug"
                    | "info"
                    | "notice"
                    | "warning"
                    | "error"
                    | "critical"
                    | "alert"
                    | "emergency"
            )
        )
    {
        return Err(invalid_metadata("modern logLevel is invalid"));
    }
    if metadata
        .get("progressToken")
        .is_some_and(|value| !value.is_string() && !value.is_number())
    {
        return Err(invalid_metadata(
            "modern progressToken must be a string or number",
        ));
    }
    Ok(())
}

fn validate_implementation(value: &Map<String, Value>) -> Result<(), String> {
    for field in ["name", "version"] {
        let Some(text) = value.get(field).and_then(Value::as_str) else {
            return Err(format!("clientInfo.{field} must be a string"));
        };
        if text.trim().is_empty() || text.len() > MAX_IDENTIFIER_BYTES {
            return Err(format!("clientInfo.{field} is invalid"));
        }
    }
    for field in ["title", "description", "websiteUrl"] {
        if value.get(field).is_some_and(|value| {
            !value.as_str().is_some_and(|text| {
                !text.trim().is_empty() && text.len() <= MAX_SEMANTIC_STRING_BYTES
            })
        }) {
            return Err(format!(
                "clientInfo.{field} must be a bounded non-empty string"
            ));
        }
    }
    if let Some(icons) = value.get("icons") {
        let Some(icons) = icons.as_array() else {
            return Err("clientInfo.icons must be an array".into());
        };
        if icons.len() > MAX_SEMANTIC_ARRAY_ITEMS {
            return Err("clientInfo.icons has too many entries".into());
        }
        for icon in icons {
            validate_icon(icon)?;
        }
    }
    Ok(())
}

fn validate_icon(value: &Value) -> Result<(), String> {
    let Some(icon) = value.as_object() else {
        return Err("clientInfo.icons entries must be objects".into());
    };
    let Some(src) = icon.get("src").and_then(Value::as_str) else {
        return Err("clientInfo.icons.src must be a string".into());
    };
    if src.trim().is_empty() || src.len() > MAX_SEMANTIC_STRING_BYTES {
        return Err("clientInfo.icons.src is invalid".into());
    }
    if icon.get("mimeType").is_some_and(|value| {
        !value
            .as_str()
            .is_some_and(|text| !text.trim().is_empty() && text.len() <= MAX_IDENTIFIER_BYTES)
    }) {
        return Err("clientInfo.icons.mimeType must be a bounded non-empty string".into());
    }
    if icon.get("sizes").is_some_and(|value| {
        !value.as_array().is_some_and(|sizes| {
            sizes.len() <= MAX_SEMANTIC_ARRAY_ITEMS
                && sizes.iter().all(|size| {
                    size.as_str().is_some_and(|text| {
                        !text.trim().is_empty() && text.len() <= MAX_IDENTIFIER_BYTES
                    })
                })
        })
    }) {
        return Err("clientInfo.icons.sizes must be an array of bounded strings".into());
    }
    if icon
        .get("theme")
        .is_some_and(|value| !matches!(value.as_str(), Some("light" | "dark")))
    {
        return Err("clientInfo.icons.theme must be light or dark".into());
    }
    Ok(())
}

fn validate_capabilities(value: &Map<String, Value>) -> Result<(), String> {
    for (name, capability) in value {
        let Some(capability) = capability.as_object() else {
            return Err(format!("clientCapabilities.{name} must be an object"));
        };
        match name.as_str() {
            "roots" => validate_boolean_members(capability, &["listChanged"], name)?,
            "sampling" => validate_object_members(capability, &["context", "tools"], name)?,
            "elicitation" => validate_object_members(capability, &["form", "url"], name)?,
            "tasks" => validate_task_capability(capability)?,
            "experimental" | "extensions" => {
                if capability.values().any(|nested| !nested.is_object()) {
                    return Err(format!("clientCapabilities.{name} entries must be objects"));
                }
                if name == "extensions"
                    && capability
                        .keys()
                        .any(|extension| !is_prefixed_extension_name(extension))
                {
                    return Err("clientCapabilities.extensions keys require a valid prefix".into());
                }
            }
            // The capability model is intentionally open to future object-valued extensions.
            _ => {}
        }
    }
    Ok(())
}

fn validate_task_capability(value: &Map<String, Value>) -> Result<(), String> {
    validate_object_members(value, &["list", "cancel", "requests"], "tasks")?;
    let Some(requests) = value.get("requests") else {
        return Ok(());
    };
    let requests = requests
        .as_object()
        .expect("validate_object_members checked tasks.requests");
    validate_object_members(requests, &["sampling", "elicitation"], "tasks.requests")?;
    if let Some(sampling) = requests.get("sampling").and_then(Value::as_object) {
        validate_object_members(sampling, &["createMessage"], "tasks.requests.sampling")?;
    }
    if let Some(elicitation) = requests.get("elicitation").and_then(Value::as_object) {
        validate_object_members(elicitation, &["create"], "tasks.requests.elicitation")?;
    }
    Ok(())
}

fn is_prefixed_extension_name(value: &str) -> bool {
    let mut parts = value.split('/');
    let (Some(prefix), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [prefix, name].into_iter().all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    })
}

fn validate_object_members(
    value: &Map<String, Value>,
    known: &[&str],
    capability: &str,
) -> Result<(), String> {
    if let Some((name, _)) = value
        .iter()
        .find(|(name, nested)| known.contains(&name.as_str()) && !nested.is_object())
    {
        return Err(format!(
            "clientCapabilities.{capability}.{name} must be an object"
        ));
    }
    Ok(())
}

fn validate_boolean_members(
    value: &Map<String, Value>,
    known: &[&str],
    capability: &str,
) -> Result<(), String> {
    if let Some((name, _)) = value
        .iter()
        .find(|(name, nested)| known.contains(&name.as_str()) && !nested.is_boolean())
    {
        return Err(format!(
            "clientCapabilities.{capability}.{name} must be a boolean"
        ));
    }
    Ok(())
}

fn invalid_metadata(message: impl Into<String>) -> PendingError {
    PendingError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

fn add_modern_result_fields(method: &str, mut response: Value) -> Value {
    if let Some(result) = response.get_mut("result").and_then(Value::as_object_mut) {
        result.insert("resultType".into(), Value::String("complete".into()));
        result.insert(
            "_meta".into(),
            json!({"io.modelcontextprotocol/serverInfo": server_info()}),
        );
        if matches!(method, "server/discover" | "tools/list") {
            result.insert("ttlMs".into(), json!(0));
            result.insert("cacheScope".into(), Value::String("private".into()));
        }
    }
    response
}

fn server_info() -> Value {
    json!({"name": "devmap", "version": env!("CARGO_PKG_VERSION")})
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

fn call_tool_response(workspace: &SourceWorkspace, id: Value, params: Option<&Value>) -> Value {
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
    if serde_json::to_vec(&arguments).is_ok_and(|bytes| bytes.len() > MAX_MCP_ARGUMENT_BYTES) {
        return invalid_params(id, "tools/call arguments exceed the configured byte limit");
    }
    if !MCP_TOOLS.contains(&name) {
        return invalid_params(id, format!("Unknown tool: {name}"));
    }

    match call_tool(workspace, name, &arguments) {
        Ok(structured) => json_rpc_result(id, tool_result(structured)),
        Err(error) => json_rpc_result(id, tool_error(error)),
    }
}

fn call_tool(
    workspace: &SourceWorkspace,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<Value, DevMapError> {
    if name == "devmap_context" {
        ensure_fields(arguments, &[])?;
        return Ok(json!({
            "workspace": workspace.root.to_string_lossy(),
            "branch": workspace.branch.clone(),
            "head": workspace.head,
            "journal_location": workspace.git_dir.join("devmap/sessions").to_string_lossy(),
            "capture_grade": host_capabilities(crate::cli::AdapterHost::GenericMcp).grade(),
        }));
    }

    let common = CommonCaptureArgs::parse(arguments, name)?;
    let journal = JournalStore::open(workspace, &common.session_id)?;
    let kernel = CaptureKernel::new(
        journal,
        host_capabilities(crate::cli::AdapterHost::GenericMcp),
        HostIdentity::new("generic_mcp", MCP_ADAPTER_VERSION)?,
        ActorIdentity::new(common.agent_id, common.parent_agent_id)?,
        SessionContext::new(
            common.session_id,
            common.route_id,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )?,
    )?;

    let record = match name {
        "devmap_record_requirement" => kernel.record_requirement_with_id(
            common.event_id.as_deref(),
            &common.occurred_at,
            RequirementTraceInput {
                source_kind: required_string(arguments, "source_kind")?,
                source_locator: optional_string(arguments, "source_locator")?,
                quoted_text: required_string(arguments, "quoted_text")?,
            },
            optional_bool(arguments, "raw_transcript_opt_in")?.unwrap_or(false),
        )?,
        "devmap_record_decision" => kernel.record_decision_with_id(
            common.event_id.as_deref(),
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
        "devmap_record_evidence" => kernel.record_evidence_with_id(
            common.event_id.as_deref(),
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
    event_id: Option<String>,
    occurred_at: String,
}

impl CommonCaptureArgs {
    fn parse(arguments: &Map<String, Value>, tool: &str) -> Result<Self, DevMapError> {
        ensure_fields(arguments, allowed_fields(tool))?;
        let session_id = required_string(arguments, "session_id")?;
        let agent_id = required_string(arguments, "agent_id")?;
        let parent_agent_id = optional_string(arguments, "parent_agent_id")?;
        let route_id = optional_string(arguments, "route_id")?;
        let event_id = optional_string(arguments, "event_id")?;
        let occurred_at = optional_string(arguments, "occurred_at")?
            .map(Ok)
            .unwrap_or_else(|| OffsetDateTime::now_utc().format(&Rfc3339))?;
        OffsetDateTime::parse(&occurred_at, &Rfc3339)
            .map_err(|_| DevMapError::InvalidDomain("occurred_at"))?;
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
    let value = arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(DevMapError::InvalidDomain(field))?;
    let limit = semantic_string_limit(field);
    if value.len() > limit {
        return Err(DevMapError::ResourceLimit {
            resource: field,
            limit,
        });
    }
    Ok(value.to_owned())
}

fn optional_string(
    arguments: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, DevMapError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            let limit = semantic_string_limit(field);
            if value.len() > limit {
                return Err(DevMapError::ResourceLimit {
                    resource: field,
                    limit,
                });
            }
            Ok(Some(value.to_owned()))
        }
        _ => Err(DevMapError::InvalidDomain(field)),
    }
}

fn semantic_string_limit(field: &str) -> usize {
    if matches!(
        field,
        "session_id" | "agent_id" | "parent_agent_id" | "route_id" | "event_id"
    ) {
        MAX_IDENTIFIER_BYTES
    } else {
        MAX_SEMANTIC_STRING_BYTES
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
    let values = arguments
        .get(field)
        .and_then(Value::as_array)
        .ok_or(DevMapError::InvalidDomain(field))?;
    if values.is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    if values.len() > MAX_SEMANTIC_ARRAY_ITEMS {
        return Err(DevMapError::ResourceLimit {
            resource: field,
            limit: MAX_SEMANTIC_ARRAY_ITEMS,
        });
    }
    let mut parsed = Vec::with_capacity(values.len());
    let mut total_bytes = 0usize;
    for value in values {
        let value = value
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or(DevMapError::InvalidDomain(field))?;
        if value.len() > MAX_SEMANTIC_STRING_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: field,
                limit: MAX_SEMANTIC_STRING_BYTES,
            });
        }
        total_bytes = total_bytes.saturating_add(value.len());
        if total_bytes > MAX_MCP_ARGUMENT_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: field,
                limit: MAX_MCP_ARGUMENT_BYTES,
            });
        }
        parsed.push(value.to_owned());
    }
    Ok(parsed)
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
                "occurred_at": date_time_schema(),
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
                "occurred_at": date_time_schema(),
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
                "occurred_at": date_time_schema(),
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

fn date_time_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "format": "date-time",
        "description": "Optional RFC 3339 event timestamp."
    })
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
    crate::commands::adapter_plan(crate::cli::AdapterPlanArgs {
        source: source.to_path_buf(),
        host: crate::cli::AdapterHost::GenericMcp,
        action: crate::cli::AdapterPlanAction::Install,
    })
}

pub fn install_generic_adapter(
    source: &Path,
    approval_token: &str,
) -> Result<CommandOutput, DevMapError> {
    crate::commands::adapter_install(crate::cli::AdapterInstallArgs {
        source: source.to_path_buf(),
        host: crate::cli::AdapterHost::GenericMcp,
        plan_digest: approval_token.to_owned(),
    })
}

pub fn verify_generic_adapter(source: &Path) -> Result<CommandOutput, DevMapError> {
    crate::commands::adapter_verify(crate::cli::AdapterVerifyArgs {
        source: source.to_path_buf(),
        host: Some(crate::cli::AdapterHost::GenericMcp),
    })
}

pub fn uninstall_generic_adapter(
    source: &Path,
    approval_token: &str,
) -> Result<CommandOutput, DevMapError> {
    crate::commands::adapter_uninstall(crate::cli::AdapterUninstallArgs {
        source: source.to_path_buf(),
        host: crate::cli::AdapterHost::GenericMcp,
        plan_digest: approval_token.to_owned(),
    })
}
