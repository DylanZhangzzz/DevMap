//! Private local runtime protocol. No filesystem, SQL, shell or arbitrary execution requests.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub const VERSION: u32 = 1;
// Diagnostic messages need no bulk payload. Sixteen admitted connections hold
// at most 256 KiB of frame buffers; no application queue exists in this slice.
pub const MAX_FRAME: usize = 16 * 1024;
pub const MAX_CONNECTIONS: usize = 16;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: u32,
    pub repository: String,
    pub build: String,
    pub source: PathBuf,
    pub git_dir: PathBuf,
    pub client_instance: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    pub protocol: u32,
    pub repository: String,
    pub build: String,
    pub owner_instance: String,
    pub owner_pid: u32,
    pub client_instance: String,
}
/// Extension seam: add reviewed typed application operations here when their service exists.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum Request {
    Ping {
        protocol: u32,
        repository: String,
        client_instance: String,
        request_id: u64,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub request_id: u64,
    pub owner_instance: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
pub enum HelloReply {
    Accepted { welcome: Welcome },
    Rejected { reason: String },
}
