//! Private local runtime protocol. No filesystem, SQL, shell or arbitrary execution requests.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub const VERSION: u32 = 1;
// Every physical message stays bounded; application aggregates additionally require
// one of the four explicit exchange reservations before upload.
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
    Begin {
        protocol: u32,
        repository: String,
        client_instance: String,
        request_id: u64,
        owner_instance: String,
        total: usize,
        digest: String,
    },
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

// Every admitted transfer reserves space before any aggregate allocation.
pub const MAX_REQUEST: usize = 4 * 1024 * 1024 + 64 * 1024;
pub const MAX_RESULT: usize = 3 * 1024 * 1024;
pub const CHUNK_BYTES: usize = 2048;
pub const EXCHANGE_RESERVATION: usize = 24 * 1024 * 1024;
pub const MAX_EXCHANGES: usize = 4;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub request_id: u64,
    pub owner_instance: String,
    pub total: usize,
    pub digest: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub transfer: Transfer,
    pub offset: usize,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
pub enum ExchangeReply {
    Ready {
        request_id: u64,
        owner_instance: String,
    },
    Busy {
        request_id: u64,
        owner_instance: String,
    },
    Result {
        transfer: Transfer,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum ApplicationRequest {
    Mutate {
        command: Box<crate::mutation::MutationCommand>,
    },
    Query {
        query: crate::application::ClientQuery,
    },
    AcceptInventory {
        prior: crate::application::ClientQuery,
        tasks: Vec<crate::dock::ObservedTask>,
        complete: bool,
        observed_at: String,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "code", deny_unknown_fields)]
pub enum DomainError {
    #[serde(rename = "domain")]
    Domain { message: String },
    #[serde(rename = "response_limit")]
    ResponseLimit { message: String },
    #[serde(rename = "revision_conflict")]
    RevisionConflict {
        message: String,
        current_revision: u64,
        #[serde(deserialize_with = "required_current_plan")]
        current_plan: Option<Box<crate::route_plan::RoutePlan>>,
    },
}
fn required_current_plan<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<crate::route_plan::RoutePlan>>, D::Error> {
    // A present null means no current plan; omission is a malformed conflict.
    Option::deserialize(deserializer)
}
impl DomainError {
    pub fn message(&self) -> &str {
        match self {
            Self::Domain { message }
            | Self::ResponseLimit { message }
            | Self::RevisionConflict { message, .. } => message,
        }
    }
}
impl From<crate::error::DevMapError> for DomainError {
    fn from(error: crate::error::DevMapError) -> Self {
        let message = error.to_string();
        match error {
            crate::error::DevMapError::RoutePlanConflict {
                revision,
                current_plan,
            } => Self::RevisionConflict {
                message,
                current_revision: revision,
                current_plan,
            },
            _ => Self::Domain { message },
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", deny_unknown_fields)]
pub enum ApplicationResult {
    Mutation {
        mutation: crate::mutation::MutationResult,
    },
    Snapshot {
        snapshot: Box<crate::application::ApplicationSnapshot>,
    },
    Inventory {
        query: crate::application::ClientQuery,
    },
    Error {
        error: DomainError,
    },
}
