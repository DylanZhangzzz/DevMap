use std::io::{BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::capture::{AgentDecisionInput, CaptureKernel, EvidenceInput, RequirementTraceInput};
use crate::dock::{DockReadModel, DockService, ObservedTask};
use crate::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI, dock_html};
use crate::error::DevMapError;
use crate::events::{ActorIdentity, HostIdentity, SessionContext, host_capabilities};
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::JournalStore;
use crate::presence::{PresenceSignal, PresenceStatus, PresenceStore};
use crate::viewer::{
    ViewerHandle, ViewerRuntime, start_live_viewer, start_live_viewer_with_task_inventory,
};

pub const DOCK_DATA_TOOL: &str = "devmap_dock_snapshot";
pub const DOCK_RENDER_TOOL: &str = "devmap_open_dock";
pub const DOCK_BROWSER_TOOL: &str = "devmap_start_browser_dock";

pub const MAP_OPEN_TOOL: &str = "devmap_open_map";
pub const MAP_READ_TOOL: &str = "devmap_read_map";
pub const MAP_PLAN_TOOL: &str = "devmap_set_route_plan";

pub const MCP_TOOLS: [&str; 6] = [
    MAP_OPEN_TOOL,
    MAP_READ_TOOL,
    MAP_PLAN_TOOL,
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

#[derive(Debug, Default)]
pub struct TransportAudit {
    pub stdio_messages: u64,
    pub tcp_listeners_opened: u64,
}

pub struct McpRuntime {
    workspace: SourceWorkspace,
    dock: Option<DockService>,
    browser_dock: Option<BrowserDock>,
    audit: TransportAudit,
    legacy_initialized: bool,
}

struct BrowserDock {
    handle: ViewerHandle,
    runtime: ViewerRuntime,
}

impl McpRuntime {
    pub fn open(source: &Path) -> Result<Self, DevMapError> {
        Ok(Self {
            workspace: SourceGitInspector::open(source)?.workspace_allow_unborn()?,
            dock: None,
            browser_dock: None,
            audit: TransportAudit::default(),
            legacy_initialized: false,
        })
    }

    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        self.audit.stdio_messages = self.audit.stdio_messages.saturating_add(1);
        handle_message(self, message)
    }

    pub fn audit(&self) -> &TransportAudit {
        &self.audit
    }
}

pub fn serve_mcp(
    source: &Path,
    mut reader: impl BufRead,
    mut writer: impl Write,
) -> Result<(), DevMapError> {
    // Workspace identity is stable for the lifetime of this stdio process. Resolving it once
    // avoids spawning multiple Git commands for every semantic capture.
    let mut runtime = McpRuntime::open(source)?;
    loop {
        let response = match read_bounded_line(&mut reader)? {
            None => break,
            Some(Ok(line)) => match serde_json::from_slice::<Value>(&line) {
                Ok(message) => runtime.handle(&message),
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

fn handle_message(runtime: &mut McpRuntime, message: &Value) -> Option<Value> {
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
            runtime.legacy_initialized = true;
        }
        return Some(response);
    }

    let era = match request_era(method, params, runtime.legacy_initialized) {
        Ok(era) => era,
        Err(error) => return Some(error.with_id(id)),
    };
    let response = match method {
        "server/discover" if era == RequestEra::Modern => json_rpc_result(id, discovery_result()),
        "server/discover" => invalid_params(id, "server/discover requires modern request metadata"),
        "tools/list" => list_tools_response(id, params),
        "tools/call" => call_tool_response(runtime, id, params),
        "resources/list" => list_resources_response(id, params),
        "resources/read" => read_resource_response(id, params),
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
            "capabilities": {"resources": {}, "tools": {}},
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
        "capabilities": {"resources": {}, "tools": {}}
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
        if matches!(method, "server/discover" | "tools/list" | "resources/list") {
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

fn list_resources_response(id: Value, params: Option<&Value>) -> Value {
    if params.is_some_and(|params| !params.is_object()) {
        return invalid_params(id, "resources/list params must be an object");
    }
    if params.and_then(Value::as_object).is_some_and(|params| {
        params
            .keys()
            .any(|field| !matches!(field.as_str(), "cursor" | "_meta"))
    }) {
        return invalid_params(id, "resources/list contains an unexpected field");
    }
    if params
        .and_then(Value::as_object)
        .and_then(|params| params.get("cursor"))
        .is_some_and(|cursor| !cursor.is_string())
    {
        return invalid_params(id, "cursor must be a string");
    }
    json_rpc_result(
        id,
        json!({
            "resources": [{
                "uri": DOCK_RESOURCE_URI,
                "name": "DevMap Live Worktree Dock",
                "description": "A read-only live view of local worktrees and instrumented Agents.",
                "mimeType": DOCK_MIME_TYPE
            }]
        }),
    )
}

fn read_resource_response(id: Value, params: Option<&Value>) -> Value {
    let Some(params) = params.and_then(Value::as_object) else {
        return invalid_params(id, "resources/read params must be an object");
    };
    if params
        .keys()
        .any(|field| !matches!(field.as_str(), "uri" | "_meta"))
    {
        return invalid_params(id, "resources/read contains an unexpected field");
    }
    if params.get("uri").and_then(Value::as_str) != Some(DOCK_RESOURCE_URI) {
        return invalid_params(id, "unknown resource URI");
    }
    if dock_html().len() > MAX_MCP_LINE_BYTES / 2 {
        return json_rpc_error(
            id,
            -32000,
            "Dock resource exceeds the configured byte limit",
            Some(json!({"resource": "Dock HTML", "limit": MAX_MCP_LINE_BYTES / 2})),
        );
    }
    json_rpc_result(
        id,
        json!({
            "contents": [{
                "uri": DOCK_RESOURCE_URI,
                "mimeType": DOCK_MIME_TYPE,
                "text": dock_html(),
                "_meta": {
                    "ui": {
                        "prefersBorder": false,
                        "csp": {
                            "connectDomains": [],
                            "resourceDomains": [],
                            "frameDomains": []
                        }
                    }
                }
            }]
        }),
    )
}

fn call_tool_response(runtime: &mut McpRuntime, id: Value, params: Option<&Value>) -> Value {
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
    if matches!(name, MAP_OPEN_TOOL | MAP_READ_TOOL | MAP_PLAN_TOOL) {
        return map_tool_response(runtime, id, name, arguments);
    }
    if !MCP_TOOLS.contains(&name)
        && !matches!(
            name,
            "devmap_context" | DOCK_DATA_TOOL | DOCK_RENDER_TOOL | DOCK_BROWSER_TOOL
        )
    {
        return invalid_params(id, format!("Unknown tool: {name}"));
    }

    if name == DOCK_BROWSER_TOOL {
        if let Err(error) = ensure_fields(&arguments, &["codex_tasks", "codex_tasks_complete"]) {
            return json_rpc_result(id, tool_error(error));
        }
        let observed_tasks = match parse_codex_tasks(&arguments) {
            Ok(tasks) => tasks,
            Err(error) => return json_rpc_result(id, tool_error(error)),
        };
        return match start_or_reuse_browser_dock(runtime, observed_tasks) {
            Ok((url, revision, reused)) => json_rpc_result(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": "DevMap Browser Dock is ready."
                    }],
                    "structuredContent": {
                        "url": url,
                        "revision": revision,
                        "reused": reused
                    },
                    "isError": false
                }),
            ),
            Err(error) => json_rpc_result(id, tool_error(error)),
        };
    }

    if matches!(name, DOCK_DATA_TOOL | DOCK_RENDER_TOOL) {
        if let Err(error) = ensure_fields(&arguments, &["codex_tasks", "codex_tasks_complete"]) {
            return json_rpc_result(id, tool_error(error));
        }
        let observed_tasks = match parse_codex_tasks(&arguments) {
            Ok(tasks) => tasks,
            Err(error) => return json_rpc_result(id, tool_error(error)),
        };
        if runtime.dock.is_none() {
            match DockService::open(&runtime.workspace.root) {
                Ok(service) => runtime.dock = Some(service),
                Err(error) => return json_rpc_result(id, tool_error(error)),
            }
        }
        if let Some(inventory) = observed_tasks
            && let Err(error) = replace_dock_inventory(runtime, inventory)
        {
            return json_rpc_result(id, tool_error(error));
        }
        let dock = runtime.dock.as_mut().expect("Dock was initialized above");
        return match dock.refresh(OffsetDateTime::now_utc()) {
            Ok(model) => json_rpc_result(id, dock_tool_result(model, name == DOCK_RENDER_TOOL)),
            Err(error) => json_rpc_result(id, tool_error(error)),
        };
    }

    match call_tool(&runtime.workspace, name, &arguments) {
        Ok(structured) => json_rpc_result(id, tool_result(structured)),
        Err(error) => json_rpc_result(id, tool_error(error)),
    }
}

fn map_tool_response(
    runtime: &mut McpRuntime,
    id: Value,
    name: &str,
    mut arguments: Map<String, Value>,
) -> Value {
    if name == MAP_PLAN_TOOL {
        let result =
            serde_json::from_value::<crate::route_plan::PlanInput>(Value::Object(arguments))
                .map_err(|e| DevMapError::RoutePlan(e.to_string()))
                .and_then(|input| {
                    crate::route_plan::RoutePlanStore::open(&runtime.workspace)?.set(input)
                })
                .and_then(|plan| Ok(tool_result(serde_json::to_value(plan)?)));
        return json_rpc_result(id, result.unwrap_or_else(tool_error));
    }
    let allowed = if name == MAP_OPEN_TOOL {
        &["codex_tasks", "codex_tasks_complete", "surface"][..]
    } else {
        &["codex_tasks", "codex_tasks_complete", "entity_id", "view"][..]
    };
    if let Err(error) = ensure_fields(&arguments, allowed) {
        return json_rpc_result(id, tool_error(error));
    }
    let surface = arguments.remove("surface").unwrap_or(json!("app"));
    if surface != "app" && surface != "browser" {
        return json_rpc_result(id, tool_error(DevMapError::InvalidDomain("surface")));
    }
    let entity = arguments.remove("entity_id");
    if entity
        .as_ref()
        .is_some_and(|v| v.as_str().is_none_or(|s| s.is_empty() || s.len() > 128))
    {
        return json_rpc_result(id, tool_error(DevMapError::InvalidDomain("entity_id")));
    }
    let view = arguments.remove("view").unwrap_or(json!("map"));
    if view != "map" && view != "context" && view != "agent" {
        return json_rpc_result(id, tool_error(DevMapError::InvalidDomain("view")));
    }
    if view == "context" {
        if entity.is_some() || !arguments.is_empty() {
            return json_rpc_result(
                id,
                tool_error(DevMapError::InvalidDomain("context view arguments")),
            );
        }
        return json_rpc_result(
            id,
            call_tool(&runtime.workspace, "devmap_context", &Map::new())
                .map(tool_result)
                .unwrap_or_else(tool_error),
        );
    }
    let alias = if name == MAP_OPEN_TOOL {
        if surface == "browser" {
            DOCK_BROWSER_TOOL
        } else {
            DOCK_RENDER_TOOL
        }
    } else {
        DOCK_DATA_TOOL
    };
    let params = json!({"name": alias, "arguments": arguments});
    let response = call_tool_response(runtime, id.clone(), Some(&params));
    if view == "agent" {
        let Some(model) = response
            .get("result")
            .and_then(|r| r.get("structuredContent"))
        else {
            return response;
        };
        let workspace_id = entity.as_ref().unwrap_or(&model["current_worktree_id"]);
        let workspace = model["lanes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|lane| &lane["worktree_id"] == workspace_id);
        let Some(workspace) = workspace else {
            return json_rpc_result(
                id,
                tool_error(DevMapError::RoutePlan(
                    "agent workspace not present in current bounded map".into(),
                )),
            );
        };
        let plans: Vec<_> = model["route_plans"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|plan| &plan["worktree_id"] == workspace_id)
            .collect();
        return json_rpc_result(
            id,
            tool_result(json!({
                "repository_id":model["repository_id"], "revision":model["revision"],
            "generated_at":model["generated_at"], "workspace":workspace,
            "task_observation":model["task_observation"],
            "workspace_facts":model["workspace_facts"].as_array().into_iter().flatten().find(|facts| &facts["worktree_id"] == workspace_id),
                "route_plans":plans, "warnings":model["warnings"], "truncated":model["truncated"],
                "execution":{"checks_status":"unverified", "merge_ready":false,
                    "authorization_verified":false,
                    "guidance":"Delivery is recorded intent. Verify authorization against the user's instructions, select one active route, check its completion conditions and fresh source/target Git state before execution. Human changes prevail. DevMap does not execute merges or certify readiness."}
            })),
        );
    }
    let Some(entity_id) = entity.as_ref().and_then(Value::as_str) else {
        return response;
    };
    let Some(model) = response
        .get("result")
        .and_then(|r| r.get("structuredContent"))
    else {
        return response;
    };
    let found = model["route_plans"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["route_id"] == entity_id)
        .or_else(|| {
            model["lanes"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["worktree_id"] == entity_id)
        })
        .or_else(|| {
            model["topology"]["commits"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["oid"] == entity_id)
        });
    match found {
        Some(value) => json_rpc_result(
            id,
            tool_result(json!({
                "repository_id": model["repository_id"], "revision": model["revision"],
                "generated_at": model["generated_at"], "entity": value,
                "warnings": model["warnings"], "truncated": model["truncated"]
            })),
        ),
        None => json_rpc_result(
            id,
            tool_error(DevMapError::RoutePlan(
                "entity not present in current bounded map".into(),
            )),
        ),
    }
}

fn replace_dock_inventory(
    runtime: &mut McpRuntime,
    inventory: ObservedTaskInventory,
) -> Result<u64, DevMapError> {
    let now = OffsetDateTime::now_utc();
    let dock = runtime.dock.as_mut().expect("Dock was initialized above");
    let revision = dock
        .replace_observed_tasks_with_completeness(inventory.tasks.clone(), inventory.complete, now)?
        .revision;
    if let Some(browser) = runtime
        .browser_dock
        .as_ref()
        .filter(|dock| dock.runtime.is_running())
    {
        browser.runtime.replace_observed_tasks_with_completeness(
            inventory.tasks,
            inventory.complete,
            now,
        )?;
    }
    Ok(revision)
}

fn start_or_reuse_browser_dock(
    runtime: &mut McpRuntime,
    observed_tasks: Option<ObservedTaskInventory>,
) -> Result<(String, u64, bool), DevMapError> {
    if runtime
        .browser_dock
        .as_ref()
        .is_some_and(|dock| dock.runtime.is_running())
    {
        let revision = if let Some(inventory) = observed_tasks {
            replace_dock_inventory(runtime, inventory)?
        } else {
            runtime
                .dock
                .as_ref()
                .map(|service| service.snapshot().revision)
                .unwrap_or(1)
        };
        return Ok((
            runtime
                .browser_dock
                .as_ref()
                .expect("healthy Viewer exists")
                .handle
                .url(),
            revision,
            true,
        ));
    }
    runtime.browser_dock = None;

    if runtime.dock.is_none() {
        runtime.dock = Some(DockService::open(&runtime.workspace.root)?);
    }
    let dock = runtime
        .dock
        .as_mut()
        .expect("Dock was initialized or returned an error above");
    let revision = match observed_tasks {
        Some(inventory) => {
            dock.replace_observed_tasks_with_completeness(
                inventory.tasks,
                inventory.complete,
                OffsetDateTime::now_utc(),
            )?
            .revision
        }
        None => dock.refresh(OffsetDateTime::now_utc())?.revision,
    };
    let tasks = dock.observed_tasks().to_vec();
    let inventory_complete = dock.task_inventory_complete();
    let inventory_observed_at = dock
        .task_inventory_observed_at()
        .map(|value| {
            OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
                DevMapError::Viewer(format!(
                    "invalid retained task observation timestamp: {error}"
                ))
            })
        })
        .transpose()?;
    let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let (handle, viewer_runtime) = if let Some(observed_at) = inventory_observed_at {
        start_live_viewer_with_task_inventory(
            &runtime.workspace.root,
            bind,
            tasks,
            inventory_complete,
            observed_at,
        )?
    } else {
        start_live_viewer(&runtime.workspace.root, bind)?
    };
    let url = handle.url();
    runtime.browser_dock = Some(BrowserDock {
        handle,
        runtime: viewer_runtime,
    });
    runtime.audit.tcp_listeners_opened = runtime.audit.tcp_listeners_opened.saturating_add(1);
    Ok((url, revision, false))
}

struct ObservedTaskInventory {
    tasks: Vec<ObservedTask>,
    complete: bool,
}

fn is_codex_thread_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn parse_codex_tasks(
    arguments: &Map<String, Value>,
) -> Result<Option<ObservedTaskInventory>, DevMapError> {
    let Some(value) = arguments.get("codex_tasks") else {
        if arguments.contains_key("codex_tasks_complete") {
            return Err(DevMapError::InvalidDomain("codex_tasks_complete"));
        }
        return Ok(None);
    };
    let complete = match arguments.get("codex_tasks_complete") {
        None => false,
        Some(Value::Bool(complete)) => *complete,
        Some(_) => return Err(DevMapError::InvalidDomain("codex_tasks_complete")),
    };
    let rows = value
        .as_array()
        .ok_or(DevMapError::InvalidDomain("codex_tasks"))?;
    if rows.len() > MAX_SEMANTIC_ARRAY_ITEMS {
        return Err(DevMapError::ResourceLimit {
            resource: "codex_tasks",
            limit: MAX_SEMANTIC_ARRAY_ITEMS,
        });
    }
    let mut tasks = Vec::with_capacity(rows.len());
    for row in rows {
        let row = row
            .as_object()
            .ok_or(DevMapError::InvalidDomain("codex_tasks"))?;
        ensure_fields(
            row,
            &[
                "id",
                "title",
                "status",
                "lifecycle",
                "cwd",
                "updatedAt",
                "hostId",
                "kind",
                "subagents",
            ],
        )?;
        if required_string(row, "kind")? != "codex" {
            return Err(DevMapError::InvalidDomain("codex_tasks.kind"));
        }
        let host = required_string(row, "hostId")?;
        if host != "local" {
            continue;
        }
        let host_status = required_string(row, "status")?;
        let lifecycle = match row.get("lifecycle").and_then(Value::as_str) {
            Some("present") => crate::dock::TaskLifecycle::Present,
            Some("archived") => crate::dock::TaskLifecycle::Archived,
            Some("deleted") => crate::dock::TaskLifecycle::Deleted,
            Some("unknown") => crate::dock::TaskLifecycle::Unknown,
            None if !row.contains_key("lifecycle") => crate::dock::TaskLifecycle::Unknown,
            _ => return Err(DevMapError::InvalidDomain("codex_tasks.lifecycle")),
        };
        let status = match host_status.as_str() {
            "active" => PresenceStatus::Working,
            "idle" => PresenceStatus::Idle,
            "waiting" => PresenceStatus::Waiting,
            "completed" => PresenceStatus::Completed,
            "notLoaded" => PresenceStatus::Stale,
            _ => return Err(DevMapError::InvalidDomain("codex_tasks.status")),
        };
        let session_id = required_string(row, "id")?;
        if session_id.len() > MAX_IDENTIFIER_BYTES || !is_codex_thread_uuid(&session_id) {
            return Err(DevMapError::InvalidDomain("codex_tasks.id"));
        }
        let updated_at = row
            .get("updatedAt")
            .and_then(Value::as_u64)
            .ok_or(DevMapError::InvalidDomain("codex_tasks.updatedAt"))?;
        let unix_seconds = if updated_at > 10_000_000_000 {
            updated_at / 1_000
        } else {
            updated_at
        };
        let updated_at = OffsetDateTime::from_unix_timestamp(
            i64::try_from(unix_seconds)
                .map_err(|_| DevMapError::InvalidDomain("codex_tasks.updatedAt"))?,
        )
        .map_err(|_| DevMapError::InvalidDomain("codex_tasks.updatedAt"))?
        .format(&Rfc3339)?;
        tasks.push(ObservedTask {
            subagents: parse_subagents(row.get("subagents"))?,
            lifecycle,
            session_id,
            display_title: required_string(row, "title")?,
            host,
            host_status,
            workspace_path: required_string(row, "cwd")?,
            status,
            updated_at,
        });
    }
    Ok(Some(ObservedTaskInventory { tasks, complete }))
}

// Nested membership is an explicit host observation of this chat's direct
// collaborators. It does not create another chat identity or passenger.
fn parse_subagents(
    value: Option<&Value>,
) -> Result<Option<Vec<crate::dock::DockSubagent>>, DevMapError> {
    let Some(value) = value else { return Ok(None) };
    let rows = value
        .as_array()
        .filter(|rows| rows.len() <= 32)
        .ok_or(DevMapError::InvalidDomain("codex_tasks.subagents"))?;
    let mut ids = std::collections::HashSet::new();
    let mut agents = Vec::with_capacity(rows.len());
    for row in rows {
        let row = row
            .as_object()
            .ok_or(DevMapError::InvalidDomain("codex_tasks.subagents"))?;
        ensure_fields(row, &["id", "name", "status", "observedAt"])?;
        let id = required_string(row, "id")?;
        let display_name = required_string(row, "name")?;
        if id.len() > 256 || display_name.len() > 256 || !ids.insert(id.clone()) {
            return Err(DevMapError::InvalidDomain("codex_tasks.subagents.identity"));
        }
        let status = match required_string(row, "status")?.as_str() {
            "working" => PresenceStatus::Working,
            "waiting" => PresenceStatus::Waiting,
            "idle" => PresenceStatus::Idle,
            "completed" => PresenceStatus::Completed,
            "unknown" => PresenceStatus::Unknown,
            _ => return Err(DevMapError::InvalidDomain("codex_tasks.subagents.status")),
        };
        let seconds = row
            .get("observedAt")
            .and_then(Value::as_i64)
            .filter(|seconds| *seconds >= 0)
            .ok_or(DevMapError::InvalidDomain(
                "codex_tasks.subagents.observedAt",
            ))?;
        let observed_at = OffsetDateTime::from_unix_timestamp(seconds)
            .map_err(|_| DevMapError::InvalidDomain("codex_tasks.subagents.observedAt"))?
            .format(&Rfc3339)?;
        agents.push(crate::dock::DockSubagent {
            id,
            display_name,
            status,
            observed_at,
        });
    }
    Ok(Some(agents))
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
    if let Err(error) = PresenceStore::open(workspace).and_then(|store| {
        store
            .observe(
                PresenceSignal::AcceptedRecords(std::slice::from_ref(&record)),
                OffsetDateTime::now_utc(),
            )
            .map(|_| ())
    }) {
        eprintln!("devmap: presence update skipped: {error}");
    }
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
    let mut open = dock_tool_descriptor(MAP_OPEN_TOOL, true);
    open["description"] = json!(
        "Open the DevMap map. Use surface browser only for an explicit browser/right-panel request; default app uses host placement. Never changes source Git."
    );
    open["inputSchema"]["properties"]["surface"] =
        json!({"type":"string", "enum":["app","browser"]});
    let mut read = dock_tool_descriptor(MAP_READ_TOOL, false);
    read["description"] = json!(
        "Read the bounded map. View agent returns delivery intent and observed workspace facts for the current worktree, or an exact worktree entity_id. It never certifies merge readiness. View context reads capture context."
    );
    read["inputSchema"]["properties"]["entity_id"] =
        json!({"type":"string","minLength":1,"maxLength":128});
    read["inputSchema"]["properties"]["view"] =
        json!({"type":"string","enum":["map","context","agent"]});
    let mut plan = tool_descriptor(
        MAP_PLAN_TOOL,
        "Record or revise explicit route intent as local DevMap metadata. Never creates a Git branch, commit or merge. Use a stable request_id for retries and expected_revision for concurrent edits. Read worktree_id from the map.",
        &[
            "request_id",
            "expected_revision",
            "worktree_id",
            "goal",
            "source",
        ],
        json!({
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "route_id":{"type":["string","null"],"minLength":1,"maxLength":128},
            "expected_revision":{"type":"integer","minimum":0},
            "worktree_id":{"type":"string","minLength":1,"maxLength":128},
            "goal":{"type":"string","minLength":1,"maxLength":2048},
            "source":{"type":"string","minLength":1,"maxLength":2048,"description":"Explicit instruction or plan source; not proof of Git execution or authenticated authorship."},
            "target_ref":{"type":["string","null"],"minLength":1,"maxLength":256,"description":"Local repository target refs/heads/name, or null when unknown."},
            "milestones":{"type":"array","maxItems":12,"items":{"type":"string","minLength":1,"maxLength":256}},
            "delivery":{"type":"object","additionalProperties":false,"required":["mode"],"description":"Full replacement; omission resets to manual. Auto merge requires target_ref, conditions and explicit authorization_source. Recorded intent, not authenticated permission.","properties":{
                "mode":{"type":"string","enum":["manual","auto_merge"]},
                "conditions":{"type":"array","maxItems":12,"items":{"type":"string","minLength":1,"maxLength":256}},
                "authorization_source":{"type":["string","null"],"minLength":1,"maxLength":2048}
            }},
            "abandoned":{"type":"boolean"}
        }),
    );
    plan["annotations"] =
        json!({"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false});
    let mut descriptors = vec![open, read, plan];
    descriptors.extend(legacy_tool_descriptors().into_iter().filter(|d| {
        d["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("devmap_record_"))
    }));
    descriptors
}

fn legacy_tool_descriptors() -> Vec<Value> {
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
        dock_tool_descriptor(DOCK_DATA_TOOL, false),
        dock_tool_descriptor(DOCK_RENDER_TOOL, true),
        dock_tool_descriptor(DOCK_BROWSER_TOOL, false),
    ]
}

fn dock_tool_descriptor(name: &str, renders_ui: bool) -> Value {
    let mut descriptor = json!({
        "name": name,
        "description": if name == DOCK_BROWSER_TOOL {
            "Start or reuse the read-only DevMap Viewer for a right-side Browser panel."
        } else if renders_ui {
            "Open the read-only DevMap worktree Dock."
        } else {
            "Read the current local DevMap worktree and Agent state."
        },
        "inputSchema": {
            "type": "object",
            "properties": {
                "codex_tasks": {
                    "type": "array",
                    "maxItems": MAX_SEMANTIC_ARRAY_ITEMS,
                    "description": "Optional active, idle, or notLoaded Codex task metadata. DevMap associates each task only when cwd exactly matches a local worktree.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "minLength": 36,
                                "maxLength": 36,
                                "pattern": "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
                            },
                            "title": {"type": "string", "minLength": 1, "maxLength": MAX_SEMANTIC_STRING_BYTES},
                            "status": {"type": "string", "enum": ["active", "idle", "waiting", "completed", "notLoaded"]},
                            "lifecycle": {"type":"string", "enum":["present","archived","deleted","unknown"], "description":"Chat existence independent of execution status. Present requires an observed unarchived chat. Omission means unknown; absence from a list never proves deletion."},
                            "cwd": {"type": "string", "minLength": 1, "maxLength": MAX_SEMANTIC_STRING_BYTES},
                            "updatedAt": {"type": "integer", "minimum": 0},
                            "hostId": {"type": "string", "minLength": 1, "maxLength": MAX_SEMANTIC_STRING_BYTES},
                            "kind": {"type": "string", "const": "codex"},
                            "subagents": {
                                "type":"array", "maxItems":32,
                                "description":"Optional explicitly observed direct collaborators of this exact parent chat. Never infer membership from names, cwd, or proximity. Omit when unavailable; these are not additional passengers. Independent chats must still appear in codex_tasks.",
                                "items":{"type":"object","properties":{
                                    "id":{"type":"string","minLength":1,"maxLength":256},
                                    "name":{"type":"string","minLength":1,"maxLength":256},
                                    "status":{"type":"string","enum":["working","waiting","idle","completed","unknown"]},
                                    "observedAt":{"type":"integer","minimum":0,"description":"Unix seconds when this relationship and status were actually observed."}
                                },"required":["id","name","status","observedAt"],"additionalProperties":false}
                            }
                        },
                        "required": ["id", "title", "status", "cwd", "updatedAt", "hostId", "kind"],
                        "additionalProperties": false
                    }
                },
                "codex_tasks_complete": {
                    "type": "boolean",
                    "description": "Whether codex_tasks covers every local unarchived chat, including idle, completed and unloaded chats. Judge coverage before filtering. Omit both fields to retain the previous observation."
                }
            },
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": true,
            "openWorldHint": false,
            "destructiveHint": false
        }
    });
    if renders_ui {
        descriptor["_meta"] = json!({
            "ui": {"resourceUri": DOCK_RESOURCE_URI},
            "openai/outputTemplate": DOCK_RESOURCE_URI
        });
    }
    descriptor
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

fn dock_tool_result(model: &DockReadModel, renders_ui: bool) -> Value {
    let structured =
        serde_json::to_value(model).expect("DockReadModel serialization is infallible");
    let total = model.current.len() + model.active.len() + model.stale_or_uninstrumented.len();
    let mut result = json!({
        "content": [{
            "type": "text",
            "text": format!(
                "DevMap Dock revision {}: {} worktree/Agent rows, {} warning(s).",
                model.revision,
                total,
                model.warnings.len()
            )
        }],
        "structuredContent": structured
    });
    if renders_ui {
        result["_meta"] = json!({"ui": {"resourceUri": DOCK_RESOURCE_URI}});
    }
    result
}

fn tool_error(error: DevMapError) -> Value {
    if let DevMapError::RoutePlanConflict {
        revision,
        ref current_plan,
    } = error
    {
        return json!({"content":[{"type":"text","text":error.to_string()}],"isError":true,
            "structuredContent":{"error_code":"revision_conflict","current_revision":revision,"current_plan":current_plan}});
    }
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
