use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::canonical::{canonical_json, sha256_hex};
use crate::cli::AgentsArgs;
use crate::error::DevMapError;
use crate::events::CaptureGrade;
use crate::git::{SourceGitInspector, SourceWorkspace};
use crate::journal::{JournalIntegrity, JournalSummary, summarize_existing_sessions};
use crate::presence::{
    Confidence, PresenceLoadReport, PresenceRecord, PresenceStatus, PresenceStore, StatusSource,
};
use crate::worktrees::{WorktreeDescriptor, WorktreeScanner, repository_id};

pub const DOCK_SCHEMA_VERSION: &str = "devmap/dock/1";
pub const MAX_DOCK_MODEL_BYTES: usize = 768 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockEntry {
    pub worktree_id: String,
    pub display_path: String,
    pub is_current: bool,
    pub branch: Option<String>,
    pub head: String,
    pub session_id: Option<String>,
    pub actor_id: Option<String>,
    pub host: Option<String>,
    pub route_id: Option<String>,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: Option<CaptureGrade>,
    pub last_event_at: Option<String>,
    pub blocker_count: u32,
    pub gap_count: u32,
    pub capture_incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockReadModel {
    pub schema_version: &'static str,
    pub repository_id: String,
    pub revision: u64,
    pub generated_at: String,
    pub current_worktree_id: String,
    pub current: Vec<DockEntry>,
    pub active: Vec<DockEntry>,
    pub stale_or_uninstrumented: Vec<DockEntry>,
    pub warnings: Vec<DockWarning>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockWarning {
    pub code: String,
    pub subject_id: Option<String>,
}

pub trait RouteProvider {
    fn route_for(&self, worktree_id: &str, session_id: Option<&str>) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoRoutes;

impl RouteProvider for NoRoutes {
    fn route_for(&self, _worktree_id: &str, _session_id: Option<&str>) -> Option<String> {
        None
    }
}

pub struct DockReducer<R: RouteProvider> {
    routes: R,
}

impl<R: RouteProvider> DockReducer<R> {
    pub fn new(routes: R) -> Self {
        Self { routes }
    }

    pub fn reduce(
        &self,
        workspace: &SourceWorkspace,
        worktrees: Vec<WorktreeDescriptor>,
        presence: PresenceLoadReport,
        journals: BTreeMap<String, JournalSummary>,
        now: OffsetDateTime,
    ) -> Result<DockReadModel, DevMapError> {
        let repository_id = repository_id(workspace);
        let current_worktree_id = worktrees
            .iter()
            .find(|row| row.is_current)
            .map(|row| row.worktree_id.clone())
            .ok_or_else(|| DevMapError::InvalidPresence("current worktree is missing".into()))?;
        let descriptors = worktrees
            .iter()
            .map(|row| (row.worktree_id.clone(), row))
            .collect::<HashMap<_, _>>();
        let mut represented = HashSet::new();
        let mut entries = Vec::new();
        let mut warnings = presence
            .warnings
            .into_iter()
            .map(|warning| DockWarning {
                code: warning.code.to_owned(),
                subject_id: warning.subject_id,
            })
            .collect::<Vec<_>>();

        for record in presence.records {
            if record.repository_id != repository_id {
                warnings.push(DockWarning {
                    code: "presence_repository_mismatch".into(),
                    subject_id: Some(record.session_id),
                });
                continue;
            }
            let Some(worktree) = descriptors.get(&record.worktree_id).copied() else {
                warnings.push(DockWarning {
                    code: "presence_worktree_missing".into(),
                    subject_id: Some(record.session_id),
                });
                continue;
            };
            represented.insert(worktree.worktree_id.clone());
            let journal_integrity = journals
                .get(&record.session_id)
                .map_or(JournalIntegrity::Missing, |summary| summary.integrity);
            if journal_integrity != JournalIntegrity::Verified {
                warnings.push(DockWarning {
                    code: match journal_integrity {
                        JournalIntegrity::Missing => "journal_missing",
                        JournalIntegrity::Corrupt => "journal_corrupt",
                        JournalIntegrity::Verified => unreachable!(),
                    }
                    .into(),
                    subject_id: Some(record.session_id.clone()),
                });
            }
            entries.push(self.entry_from_presence(
                worktree,
                record.effective_at(now),
                journal_integrity,
            ));
        }
        for worktree in &worktrees {
            if !represented.contains(&worktree.worktree_id) {
                entries.push(self.unknown_entry(worktree));
            }
        }
        entries.sort_by(compare_entries);
        warnings.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.subject_id.cmp(&right.subject_id))
        });
        warnings.dedup();

        let mut current = Vec::new();
        let mut active = Vec::new();
        let mut stale_or_uninstrumented = Vec::new();
        for entry in entries {
            if entry.is_current {
                current.push(entry);
            } else if matches!(
                entry.status,
                PresenceStatus::Waiting
                    | PresenceStatus::Working
                    | PresenceStatus::Starting
                    | PresenceStatus::Idle
            ) {
                active.push(entry);
            } else {
                stale_or_uninstrumented.push(entry);
            }
        }

        bound_model(DockReadModel {
            schema_version: DOCK_SCHEMA_VERSION,
            repository_id,
            revision: 0,
            generated_at: now.format(&Rfc3339)?,
            current_worktree_id,
            current,
            active,
            stale_or_uninstrumented,
            warnings,
            truncated: presence.truncated,
        })
    }

    fn entry_from_presence(
        &self,
        worktree: &WorktreeDescriptor,
        record: PresenceRecord,
        journal_integrity: JournalIntegrity,
    ) -> DockEntry {
        DockEntry {
            worktree_id: worktree.worktree_id.clone(),
            display_path: worktree.root.to_string_lossy().into_owned(),
            is_current: worktree.is_current,
            branch: worktree.branch.clone(),
            head: worktree.head.clone(),
            session_id: Some(record.session_id.clone()),
            actor_id: Some(record.actor_id),
            host: Some(record.host),
            route_id: record.route_id.or_else(|| {
                self.routes
                    .route_for(&worktree.worktree_id, Some(&record.session_id))
            }),
            status: record.status,
            status_source: record.status_source,
            confidence: record.confidence,
            capture_grade: Some(record.capture_grade),
            last_event_at: Some(record.last_event_at),
            blocker_count: record.blocker_count,
            gap_count: record.gap_count,
            capture_incomplete: record.gap_count > 0
                || journal_integrity != JournalIntegrity::Verified,
        }
    }

    fn unknown_entry(&self, worktree: &WorktreeDescriptor) -> DockEntry {
        DockEntry {
            worktree_id: worktree.worktree_id.clone(),
            display_path: worktree.root.to_string_lossy().into_owned(),
            is_current: worktree.is_current,
            branch: worktree.branch.clone(),
            head: worktree.head.clone(),
            session_id: None,
            actor_id: None,
            host: None,
            route_id: self.routes.route_for(&worktree.worktree_id, None),
            status: PresenceStatus::Unknown,
            status_source: StatusSource::GitOnly,
            confidence: Confidence::Unknown,
            capture_grade: None,
            last_event_at: None,
            blocker_count: 0,
            gap_count: 0,
            capture_incomplete: true,
        }
    }
}

impl DockReadModel {
    pub fn content_hash(&self) -> Result<String, DevMapError> {
        #[derive(Serialize)]
        struct Content<'a> {
            schema_version: &'a str,
            repository_id: &'a str,
            current_worktree_id: &'a str,
            current: &'a [DockEntry],
            active: &'a [DockEntry],
            stale_or_uninstrumented: &'a [DockEntry],
            warnings: &'a [DockWarning],
            truncated: bool,
        }
        let bytes = canonical_json(&Content {
            schema_version: self.schema_version,
            repository_id: &self.repository_id,
            current_worktree_id: &self.current_worktree_id,
            current: &self.current,
            active: &self.active,
            stale_or_uninstrumented: &self.stale_or_uninstrumented,
            warnings: &self.warnings,
            truncated: self.truncated,
        })?;
        Ok(format!("sha256-{}", sha256_hex(&bytes)))
    }
}

pub struct DockService {
    workspace: SourceWorkspace,
    reducer: DockReducer<NoRoutes>,
    revision: u64,
    content_hash: Option<String>,
    snapshot: Option<DockReadModel>,
}

impl DockService {
    pub fn open(source: &Path) -> Result<Self, DevMapError> {
        let workspace = SourceGitInspector::open(source)?.workspace()?;
        let mut service = Self {
            workspace,
            reducer: DockReducer::new(NoRoutes),
            revision: 0,
            content_hash: None,
            snapshot: None,
        };
        service.refresh(OffsetDateTime::now_utc())?;
        Ok(service)
    }

    pub fn refresh(&mut self, now: OffsetDateTime) -> Result<&DockReadModel, DevMapError> {
        let worktrees = WorktreeScanner::scan(&self.workspace)?;
        let presence = PresenceStore::open_existing(&self.workspace)?
            .map(|store| store.load_all())
            .unwrap_or(PresenceLoadReport {
                records: Vec::new(),
                warnings: Vec::new(),
                truncated: false,
            });
        let sessions = presence
            .records
            .iter()
            .map(|record| record.session_id.clone())
            .collect::<BTreeSet<_>>();
        let journals = summarize_existing_sessions(&self.workspace, &sessions);
        let mut next = self
            .reducer
            .reduce(&self.workspace, worktrees, presence, journals, now)?;
        let content_hash = next.content_hash()?;
        if self.content_hash.as_deref() != Some(&content_hash) {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(DevMapError::DockRevisionOverflow)?;
            self.content_hash = Some(content_hash);
        }
        next.revision = self.revision.max(1);
        self.snapshot = Some(next);
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> &DockReadModel {
        self.snapshot
            .as_ref()
            .expect("DockService::open creates the initial snapshot")
    }
}

pub fn agents(args: AgentsArgs) -> Result<CommandOutput, DevMapError> {
    let service = DockService::open(&args.source)?;
    let model = service.snapshot();
    let stdout = if args.json {
        format!(
            "{}\n",
            String::from_utf8(canonical_json(model)?)
                .map_err(|_| DevMapError::NonUtf8GitOutput("canonical Dock model".into()))?
        )
    } else {
        render_text(model)
    };
    Ok(CommandOutput {
        stdout,
        exit_code: 0,
    })
}

fn compare_entries(left: &DockEntry, right: &DockEntry) -> std::cmp::Ordering {
    severity(left.status)
        .cmp(&severity(right.status))
        .then_with(|| event_instant(right).cmp(&event_instant(left)))
        .then_with(|| left.worktree_id.cmp(&right.worktree_id))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

fn event_instant(entry: &DockEntry) -> Option<OffsetDateTime> {
    entry
        .last_event_at
        .as_deref()
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
}

fn severity(status: PresenceStatus) -> u8 {
    match status {
        PresenceStatus::Waiting => 0,
        PresenceStatus::Stale => 1,
        PresenceStatus::Working => 2,
        PresenceStatus::Starting => 3,
        PresenceStatus::Idle => 4,
        PresenceStatus::Completed => 5,
        PresenceStatus::Unknown => 6,
    }
}

fn render_text(model: &DockReadModel) -> String {
    let mut output = format!(
        "DevMap Agents · revision {} · warnings {}{}\n",
        model.revision,
        model.warnings.len(),
        if model.truncated { " · TRUNCATED" } else { "" }
    );
    for (title, entries) in [
        ("CURRENT", &model.current),
        ("ACTIVE", &model.active),
        ("STALE OR UNINSTRUMENTED", &model.stale_or_uninstrumented),
    ] {
        output.push_str(&format!("\n{title}\n"));
        for entry in entries {
            output.push_str(&format!(
                "{:<10} {:<18} {:<20} {}{}\n",
                format!("{:?}", entry.status).to_lowercase(),
                bounded(entry.actor_id.as_deref().unwrap_or("-"), 18),
                bounded(entry.branch.as_deref().unwrap_or("detached"), 20),
                bounded(&entry.display_path, 80),
                if entry.capture_incomplete {
                    "  CAPTURE INCOMPLETE"
                } else {
                    ""
                }
            ));
        }
    }
    output
}

fn bounded(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!(
            "{}…",
            prefix
                .chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        prefix
    }
}

fn bound_model(mut model: DockReadModel) -> Result<DockReadModel, DevMapError> {
    const STRUCTURAL_RESERVE: usize = 32 * 1024;
    let mut remaining = MAX_DOCK_MODEL_BYTES.saturating_sub(STRUCTURAL_RESERVE);
    let mut output_truncated = false;
    output_truncated |= retain_within_budget(&mut model.current, &mut remaining)?;
    output_truncated |= retain_within_budget(&mut model.active, &mut remaining)?;
    output_truncated |= retain_within_budget(&mut model.warnings, &mut remaining)?;
    output_truncated |= retain_within_budget(&mut model.stale_or_uninstrumented, &mut remaining)?;
    if output_truncated {
        model.truncated = true;
        model.warnings.push(DockWarning {
            code: "dock_output_truncated".into(),
            subject_id: None,
        });
        model.warnings.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.subject_id.cmp(&right.subject_id))
        });
        model.warnings.dedup();
    }
    if canonical_json(&model)?.len() > MAX_DOCK_MODEL_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "Dock read model",
            limit: MAX_DOCK_MODEL_BYTES,
        });
    }
    Ok(model)
}

fn retain_within_budget<T: Serialize>(
    values: &mut Vec<T>,
    remaining: &mut usize,
) -> Result<bool, DevMapError> {
    let original = values.len();
    let mut kept = Vec::with_capacity(original);
    for value in values.drain(..) {
        let size = canonical_json(&value)?.len().saturating_add(1);
        if size <= *remaining {
            *remaining -= size;
            kept.push(value);
        }
    }
    let truncated = kept.len() != original;
    *values = kept;
    Ok(truncated)
}
