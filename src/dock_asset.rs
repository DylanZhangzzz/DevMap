pub const DOCK_RESOURCE_URI: &str = "ui://devmap/dock/v1.html";
pub const DOCK_MIME_TYPE: &str = "text/html;profile=mcp-app";

pub fn dock_html() -> &'static str {
    include_str!("../assets/dock.html")
}
