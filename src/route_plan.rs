//! Local intent only: this store never executes a source Git mutation.
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::error::DevMapError;
use crate::fs_security::{
    checked_canonical_directory, checked_file, checked_metadata, ensure_directory_chain,
};
use crate::git::SourceWorkspace;
use crate::worktrees::{WorktreeScanner, repository_id};

const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PLANS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    #[default]
    Manual,
    AutoMerge,
}

/// Recorded instructions, not authenticated permission or proof that checks passed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub mode: DeliveryMode,
    #[serde(default)]
    pub conditions: Vec<String>,
    pub authorization_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    #[serde(default)]
    pub delivery: Delivery,
    pub request_id: String,
    pub route_id: Option<String>,
    pub expected_revision: u64,
    pub worktree_id: String,
    pub goal: String,
    pub target_ref: Option<String>,
    #[serde(default)]
    pub milestones: Vec<String>,
    pub source: String,
    #[serde(default)]
    pub abandoned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePlan {
    #[serde(default)]
    pub delivery: Delivery,
    pub route_id: String,
    pub repository_id: String,
    pub revision: u64,
    pub worktree_id: String,
    pub start_commit: String,
    pub goal: String,
    pub target_ref: Option<String>,
    pub milestones: Vec<String>,
    pub source: String,
    pub abandoned: bool,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    input: PlanInput,
    plan: RoutePlan,
}

pub struct RoutePlanStore {
    workspace: SourceWorkspace,
    root: PathBuf,
}

fn invalid(message: impl Into<String>) -> DevMapError {
    DevMapError::RoutePlan(message.into())
}

impl RoutePlanStore {
    /// Resolves storage without creating any state during reads.
    pub fn open(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        let root = checked_canonical_directory(&workspace.git_common_dir)?;
        Ok(Self {
            workspace: workspace.clone(),
            root,
        })
    }

    fn existing_path(&self) -> Result<Option<PathBuf>, DevMapError> {
        let directory = self.root.join("devmap");
        let Some(metadata) = checked_metadata(&directory)? else {
            return Ok(None);
        };
        if !metadata.is_dir() {
            return Err(invalid("plan directory is not a directory"));
        }
        let path = directory.join("route-plans.jsonl");
        Ok(checked_metadata(&path)?.map(|_| path))
    }

    pub fn list(&self) -> Result<Vec<RoutePlan>, DevMapError> {
        let Some(path) = self.existing_path()? else {
            return Ok(Vec::new());
        };
        let mut file = checked_file(&path, false, false)?;
        FileExt::lock_shared(&file)?;
        let records = read_records(&mut file, &repository_id(&self.workspace))?;
        let mut latest = BTreeMap::new();
        for record in records {
            latest.insert(record.plan.route_id.clone(), record.plan);
        }
        Ok(latest.into_values().collect())
    }

    pub fn set(&self, input: PlanInput) -> Result<RoutePlan, DevMapError> {
        validate(&input)?;
        if let Some(target) = &input.target_ref {
            let status = Command::new("git")
                .args(["check-ref-format", target])
                .output()?;
            if !status.status.success() {
                return Err(invalid("invalid target_ref"));
            }
        }
        let directory = ensure_directory_chain(&self.root, &["devmap"])?;
        let mut file = checked_file(&directory.join("route-plans.jsonl"), true, true)?;
        FileExt::lock_exclusive(&file)?;
        let records = read_records(&mut file, &repository_id(&self.workspace))?;
        if let Some(existing) = records
            .iter()
            .find(|record| record.input.request_id == input.request_id)
        {
            return if existing.input == input {
                Ok(existing.plan.clone())
            } else {
                Err(invalid("request_id already used for different content"))
            };
        }
        let previous = input
            .route_id
            .as_ref()
            .and_then(|id| records.iter().rev().find(|r| &r.plan.route_id == id));
        if input.route_id.is_some() && previous.is_none() {
            return Err(invalid("route_id not found"));
        }
        let actual_revision = previous.map_or(0, |r| r.plan.revision);
        if actual_revision != input.expected_revision {
            return Err(DevMapError::RoutePlanConflict {
                revision: actual_revision,
                current_plan: previous.map(|record| Box::new(record.plan.clone())),
            });
        }
        if previous.is_none()
            && records
                .iter()
                .map(|r| &r.plan.route_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                >= MAX_PLANS
        {
            return Err(invalid("route plan limit reached"));
        }
        let worktree = WorktreeScanner::scan(&self.workspace)?
            .into_iter()
            .find(|w| w.worktree_id == input.worktree_id && !w.is_bare && !w.is_prunable);
        let retains_workspace = previous.is_some_and(|r| r.plan.worktree_id == input.worktree_id);
        if worktree.is_none() && !retains_workspace {
            return Err(invalid("worktree_id not found in this repository"));
        }
        let start_commit = if let Some(record) = previous {
            record.plan.start_commit.clone()
        } else {
            let head = &worktree
                .as_ref()
                .expect("new plans require a live worktree")
                .head;
            if head.is_empty() || head.bytes().all(|b| b == b'0') {
                return Err(invalid("route start requires an existing commit"));
            }
            head.clone()
        };
        let route_id = match previous {
            Some(record) => record.plan.route_id.clone(),
            None => {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|e| invalid(e.to_string()))?;
                format!(
                    "route-{}",
                    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
                )
            }
        };
        let plan = RoutePlan {
            delivery: input.delivery.clone(),
            route_id,
            repository_id: repository_id(&self.workspace),
            revision: actual_revision
                .checked_add(1)
                .ok_or_else(|| invalid("revision overflow"))?,
            worktree_id: input.worktree_id.clone(),
            start_commit,
            goal: input.goal.clone(),
            target_ref: input.target_ref.clone(),
            milestones: input.milestones.clone(),
            source: input.source.clone(),
            abandoned: input.abandoned,
            updated_at: OffsetDateTime::now_utc().format(&Rfc3339)?,
        };
        let mut bytes = serde_json::to_vec(&Record {
            input,
            plan: plan.clone(),
        })?;
        bytes.push(b'\n');
        if file.metadata()?.len().saturating_add(bytes.len() as u64) > MAX_JOURNAL_BYTES {
            return Err(invalid("route plan journal limit reached"));
        }
        file.seek(SeekFrom::End(0))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(plan)
    }
}

fn read_records(file: &mut std::fs::File, repository: &str) -> Result<Vec<Record>, DevMapError> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(invalid("route plan journal limit reached"));
    }
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        return Err(invalid("incomplete plan journal; reconciliation required"));
    }
    let mut records = Vec::new();
    let mut revisions = BTreeMap::new();
    let mut requests = std::collections::BTreeSet::new();
    let mut starts = BTreeMap::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let record: Record =
            serde_json::from_slice(line).map_err(|_| invalid("invalid plan journal"))?;
        validate(&record.input)?;
        let p = &record.plan;
        let i = &record.input;
        let route_valid = p
            .route_id
            .strip_prefix("route-")
            .is_some_and(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()));
        let oid_valid = matches!(p.start_commit.len(), 40 | 64)
            && p.start_commit.bytes().all(|c| c.is_ascii_hexdigit())
            && !p.start_commit.bytes().all(|c| c == b'0');
        if !route_valid
            || !oid_valid
            || p.repository_id != repository
            || p.worktree_id != i.worktree_id
            || p.goal != i.goal
            || p.target_ref != i.target_ref
            || p.milestones != i.milestones
            || p.delivery != i.delivery
            || p.source != i.source
            || p.abandoned != i.abandoned
            || i.expected_revision.checked_add(1) != Some(p.revision)
            || (p.revision == 1 && i.route_id.is_some())
            || (p.revision > 1 && i.route_id.as_deref() != Some(p.route_id.as_str()))
            || OffsetDateTime::parse(&p.updated_at, &Rfc3339).is_err()
            || starts
                .get(&p.route_id)
                .is_some_and(|start| start != &p.start_commit)
        {
            return Err(invalid("inconsistent plan journal"));
        }
        starts.insert(p.route_id.clone(), p.start_commit.clone());
        let revision = revisions
            .entry(record.plan.route_id.clone())
            .or_insert(0u64);
        if Some(record.plan.revision) != revision.checked_add(1)
            || !requests.insert(record.input.request_id.clone())
        {
            return Err(invalid("inconsistent plan journal"));
        }
        *revision = record.plan.revision;
        records.push(record);
    }
    if revisions.len() > MAX_PLANS {
        return Err(invalid("route plan limit reached"));
    }
    Ok(records)
}

fn validate(input: &PlanInput) -> Result<(), DevMapError> {
    fn text(value: &str, limit: usize) -> bool {
        !value.trim().is_empty() && value.len() <= limit && !value.chars().any(|c| c.is_control())
    }
    if !text(&input.request_id, 128)
        || input.delivery.conditions.len() > 12
        || input.delivery.conditions.iter().any(|v| !text(v, 256))
        || input
            .delivery
            .authorization_source
            .as_ref()
            .is_some_and(|v| !text(v, 2048))
        || (input.delivery.mode == DeliveryMode::AutoMerge
            && (input.target_ref.is_none()
                || input.delivery.conditions.is_empty()
                || input.delivery.authorization_source.is_none()))
        || !text(&input.worktree_id, 128)
        || !text(&input.goal, 2048)
        || !text(&input.source, 2048)
        || input.milestones.len() > 12
        || input.milestones.iter().any(|v| !text(v, 256))
        || input.route_id.as_ref().is_some_and(|v| !text(v, 128))
        || input
            .target_ref
            .as_ref()
            .is_some_and(|v| !text(v, 256) || !v.starts_with("refs/heads/"))
    {
        return Err(invalid("invalid or oversized route plan input"));
    }
    Ok(())
}
