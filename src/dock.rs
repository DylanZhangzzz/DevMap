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
use crate::git_relationship::{
    DevelopmentTarget, ForkPoint, GitRelationship, GitRelationshipResolver, IntegrationBranch,
};
use crate::journal::{JournalIntegrity, JournalSummary, summarize_existing_sessions};
use crate::presence::{
    Confidence, PresenceLoadReport, PresenceRecord, PresenceStatus, PresenceStore, StatusSource,
};
use crate::worktrees::{WorktreeDescriptor, WorktreeScanner, repository_id};

pub const DOCK_SCHEMA_VERSION: &str = "devmap/dock/3";
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
pub struct DockChat {
    pub session_id: String,
    pub codex_thread_id: Option<String>,
    pub display_title: String,
    pub actor_id: String,
    pub host: String,
    pub host_status: Option<String>,
    pub route_id: Option<String>,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: CaptureGrade,
    pub last_event_at: String,
    pub blocker_count: u32,
    pub gap_count: u32,
    pub capture_incomplete: bool,
    pub association_source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedTask {
    pub session_id: String,
    pub display_title: String,
    pub host: String,
    pub host_status: String,
    pub workspace_path: String,
    pub status: PresenceStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockLane {
    pub worktree_id: String,
    pub workspace_path: String,
    pub is_current: bool,
    pub branch: Option<String>,
    pub head: String,
    pub relationship: GitRelationship,
    pub chats: Vec<DockChat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchGroup {
    pub target_branch: String,
    pub terminal: bool,
    pub fork_point: Option<ForkPoint>,
    pub lanes: Vec<DockLane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockReadModel {
    pub schema_version: &'static str,
    pub repository_id: String,
    pub revision: u64,
    pub generated_at: String,
    pub current_worktree_id: String,
    pub development_target: Option<DevelopmentTarget>,
    pub integration_branches: Vec<IntegrationBranch>,
    pub branch_groups: Vec<BranchGroup>,
    pub task_inventory_synced_at: Option<String>,
    pub lanes: Vec<DockLane>,
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
        self.reduce_with_tasks(workspace, worktrees, presence, journals, now, &[])
    }

    pub fn reduce_with_tasks(
        &self,
        workspace: &SourceWorkspace,
        worktrees: Vec<WorktreeDescriptor>,
        presence: PresenceLoadReport,
        journals: BTreeMap<String, JournalSummary>,
        now: OffsetDateTime,
        observed_tasks: &[ObservedTask],
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
        let relationship_report = GitRelationshipResolver::resolve(workspace, &worktrees)?;
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
        warnings.extend(
            relationship_report
                .warnings
                .iter()
                .map(|warning| DockWarning {
                    code: warning.code.into(),
                    subject_id: warning.worktree_id.clone(),
                }),
        );

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
        let mut lanes = worktrees
            .iter()
            .map(|worktree| {
                let mut chats = entries
                    .iter()
                    .filter(|entry| entry.worktree_id == worktree.worktree_id)
                    .filter_map(chat_from_entry)
                    .collect::<Vec<_>>();
                for task in observed_tasks
                    .iter()
                    .filter(|task| same_workspace_path(&task.workspace_path, &worktree.root))
                {
                    if let Some(chat) = chats
                        .iter_mut()
                        .find(|chat| chat.session_id == task.session_id)
                    {
                        chat.display_title = task.display_title.clone();
                        chat.host_status = Some(task.host_status.clone());
                    } else {
                        chats.push(chat_from_observed_task(task));
                    }
                }
                chats.sort_by(|left, right| {
                    event_instant_from_text(&right.last_event_at)
                        .cmp(&event_instant_from_text(&left.last_event_at))
                        .then_with(|| left.session_id.cmp(&right.session_id))
                });
                DockLane {
                    worktree_id: worktree.worktree_id.clone(),
                    workspace_path: worktree.root.to_string_lossy().into_owned(),
                    is_current: worktree.is_current,
                    branch: worktree.branch.clone(),
                    head: worktree.head.clone(),
                    relationship: relationship_report
                        .by_worktree_id
                        .get(&worktree.worktree_id)
                        .cloned()
                        .unwrap_or(GitRelationship {
                            base_target: relationship_report
                                .target
                                .as_ref()
                                .map(|target| target.name.clone()),
                            merge_target: relationship_report
                                .target
                                .as_ref()
                                .map(|target| target.name.clone()),
                            merged: None,
                            ahead: None,
                            behind: None,
                            dirty: false,
                            changed_file_count: 0,
                            fork_point: None,
                        }),
                    chats,
                }
            })
            .collect::<Vec<_>>();
        lanes.sort_by(|left, right| {
            right
                .is_current
                .cmp(&left.is_current)
                .then_with(|| left.branch.cmp(&right.branch))
                .then_with(|| left.workspace_path.cmp(&right.workspace_path))
                .then_with(|| left.worktree_id.cmp(&right.worktree_id))
        });
        let branch_groups =
            branch_groups_from_lanes(&lanes, &relationship_report.integration_branches);
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
            development_target: relationship_report.target,
            integration_branches: relationship_report.integration_branches,
            branch_groups,
            task_inventory_synced_at: None,
            lanes,
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
            development_target: &'a Option<DevelopmentTarget>,
            integration_branches: &'a [IntegrationBranch],
            branch_groups: &'a [BranchGroup],
            task_inventory_synced_at: &'a Option<String>,
            lanes: &'a [DockLane],
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
            development_target: &self.development_target,
            integration_branches: &self.integration_branches,
            branch_groups: &self.branch_groups,
            task_inventory_synced_at: &self.task_inventory_synced_at,
            lanes: &self.lanes,
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
    observed_tasks: Vec<ObservedTask>,
    task_inventory_synced_at: Option<String>,
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
            observed_tasks: Vec::new(),
            task_inventory_synced_at: None,
        };
        service.refresh(OffsetDateTime::now_utc())?;
        Ok(service)
    }

    pub fn replace_observed_tasks(
        &mut self,
        mut tasks: Vec<ObservedTask>,
        now: OffsetDateTime,
    ) -> Result<&DockReadModel, DevMapError> {
        tasks.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.workspace_path.cmp(&right.workspace_path))
                .then_with(|| left.display_title.cmp(&right.display_title))
                .then_with(|| left.host_status.cmp(&right.host_status))
                .then_with(|| left.updated_at.cmp(&right.updated_at))
        });
        if tasks
            .windows(2)
            .any(|pair| pair[0].session_id == pair[1].session_id)
        {
            return Err(DevMapError::InvalidDomain("codex_tasks.id"));
        }
        if self.observed_tasks != tasks {
            self.observed_tasks = tasks;
            self.task_inventory_synced_at = Some(now.format(&Rfc3339)?);
        }
        self.refresh(now)
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
        let mut next = self.reducer.reduce_with_tasks(
            &self.workspace,
            worktrees,
            presence,
            journals,
            now,
            &self.observed_tasks,
        )?;
        next.task_inventory_synced_at = self.task_inventory_synced_at.clone();
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

    pub fn observed_tasks(&self) -> &[ObservedTask] {
        &self.observed_tasks
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

fn chat_from_entry(entry: &DockEntry) -> Option<DockChat> {
    let actor_id = entry.actor_id.clone()?;
    Some(DockChat {
        session_id: entry.session_id.clone()?,
        codex_thread_id: None,
        display_title: actor_id.clone(),
        actor_id,
        host: entry.host.clone()?,
        host_status: None,
        route_id: entry.route_id.clone(),
        status: entry.status,
        status_source: entry.status_source,
        confidence: entry.confidence,
        capture_grade: entry.capture_grade?,
        last_event_at: entry.last_event_at.clone()?,
        blocker_count: entry.blocker_count,
        gap_count: entry.gap_count,
        capture_incomplete: entry.capture_incomplete,
        association_source: "presence_worktree_id",
    })
}

fn chat_from_observed_task(task: &ObservedTask) -> DockChat {
    DockChat {
        session_id: task.session_id.clone(),
        codex_thread_id: Some(task.session_id.clone()),
        display_title: task.display_title.clone(),
        actor_id: "codex".into(),
        host: task.host.clone(),
        host_status: Some(task.host_status.clone()),
        route_id: None,
        status: task.status,
        status_source: StatusSource::GitOnly,
        confidence: Confidence::Observed,
        capture_grade: CaptureGrade::D,
        last_event_at: task.updated_at.clone(),
        blocker_count: 0,
        gap_count: 0,
        capture_incomplete: true,
        association_source: "codex_task_cwd",
    }
}

fn same_workspace_path(observed: &str, workspace: &Path) -> bool {
    let observed = std::fs::canonicalize(observed).unwrap_or_else(|_| observed.into());
    let workspace = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    if cfg!(windows) {
        observed
            .to_string_lossy()
            .eq_ignore_ascii_case(&workspace.to_string_lossy())
    } else {
        observed == workspace
    }
}

fn branch_groups_from_lanes(
    lanes: &[DockLane],
    integration_branches: &[IntegrationBranch],
) -> Vec<BranchGroup> {
    let mut grouped = BTreeMap::<(String, Option<String>, bool), Vec<DockLane>>::new();
    for lane in lanes {
        let terminal = lane.relationship.merge_target.is_none();
        let target_branch = lane
            .relationship
            .merge_target
            .clone()
            .or_else(|| lane.branch.clone())
            .unwrap_or_else(|| "unknown".into());
        let commit = lane
            .relationship
            .fork_point
            .as_ref()
            .map(|fork| fork.commit.clone());
        grouped
            .entry((target_branch, commit, terminal))
            .or_default()
            .push(lane.clone());
    }
    let rail_order = integration_branches
        .iter()
        .enumerate()
        .map(|(index, branch)| (branch.name.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut groups = grouped
        .into_iter()
        .map(|((target_branch, _, terminal), lanes)| BranchGroup {
            target_branch,
            terminal,
            fork_point: lanes
                .first()
                .and_then(|lane| lane.relationship.fork_point.clone()),
            lanes,
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        rail_order
            .get(left.target_branch.as_str())
            .unwrap_or(&usize::MAX)
            .cmp(
                rail_order
                    .get(right.target_branch.as_str())
                    .unwrap_or(&usize::MAX),
            )
            .then_with(|| {
                right
                    .fork_point
                    .as_ref()
                    .and_then(|fork| fork.distance_to_target)
                    .cmp(
                        &left
                            .fork_point
                            .as_ref()
                            .and_then(|fork| fork.distance_to_target),
                    )
            })
            .then_with(|| {
                left.fork_point
                    .as_ref()
                    .map(|fork| fork.commit.as_str())
                    .cmp(&right.fork_point.as_ref().map(|fork| fork.commit.as_str()))
            })
    });
    groups
}

fn event_instant_from_text(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
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
    let available = MAX_DOCK_MODEL_BYTES.saturating_sub(STRUCTURAL_RESERVE);
    let mut group_remaining = available * 45 / 100;
    let mut lane_remaining = available * 25 / 100;
    let mut compatibility_remaining = available - group_remaining - lane_remaining;
    let mut output_truncated = false;
    output_truncated |=
        retain_branch_groups_within_budget(&mut model.branch_groups, &mut group_remaining)?;
    output_truncated |= retain_lanes_within_budget(&mut model.lanes, &mut lane_remaining)?;
    output_truncated |= retain_within_budget(&mut model.current, &mut compatibility_remaining)?;
    output_truncated |= retain_within_budget(&mut model.active, &mut compatibility_remaining)?;
    output_truncated |= retain_within_budget(&mut model.warnings, &mut compatibility_remaining)?;
    output_truncated |= retain_within_budget(
        &mut model.stale_or_uninstrumented,
        &mut compatibility_remaining,
    )?;
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

fn retain_branch_groups_within_budget(
    groups: &mut Vec<BranchGroup>,
    remaining: &mut usize,
) -> Result<bool, DevMapError> {
    let original_groups = groups.len();
    let original_lanes = groups.iter().map(|group| group.lanes.len()).sum::<usize>();
    let original_chats = groups
        .iter()
        .flat_map(|group| &group.lanes)
        .map(|lane| lane.chats.len())
        .sum::<usize>();
    let mut kept_groups = Vec::with_capacity(original_groups);
    let mut deferred_chats = Vec::new();
    for mut group in groups.drain(..) {
        let lanes = std::mem::take(&mut group.lanes);
        let group_size = canonical_json(&group)?.len().saturating_add(1);
        if group_size > *remaining {
            continue;
        }
        *remaining -= group_size;
        for mut lane in lanes {
            let chats = std::mem::take(&mut lane.chats);
            let lane_size = canonical_json(&lane)?.len().saturating_add(1);
            if lane_size <= *remaining {
                *remaining -= lane_size;
                let group_index = kept_groups.len();
                let lane_index = group.lanes.len();
                group.lanes.push(lane);
                deferred_chats.push((group_index, lane_index, chats));
            }
        }
        if !group.lanes.is_empty() {
            kept_groups.push(group);
        }
    }
    for (group_index, lane_index, chats) in deferred_chats {
        let Some(lane) = kept_groups
            .get_mut(group_index)
            .and_then(|group| group.lanes.get_mut(lane_index))
        else {
            continue;
        };
        for chat in chats {
            let size = canonical_json(&chat)?.len().saturating_add(1);
            if size <= *remaining {
                *remaining -= size;
                lane.chats.push(chat);
            }
        }
    }
    let kept_lanes = kept_groups
        .iter()
        .map(|group| group.lanes.len())
        .sum::<usize>();
    let kept_chats = kept_groups
        .iter()
        .flat_map(|group| &group.lanes)
        .map(|lane| lane.chats.len())
        .sum::<usize>();
    let truncated = kept_groups.len() != original_groups
        || kept_lanes != original_lanes
        || kept_chats != original_chats;
    *groups = kept_groups;
    Ok(truncated)
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

fn retain_lanes_within_budget(
    lanes: &mut Vec<DockLane>,
    remaining: &mut usize,
) -> Result<bool, DevMapError> {
    let original_lanes = lanes.len();
    let original_chats = lanes.iter().map(|lane| lane.chats.len()).sum::<usize>();
    let mut kept = Vec::with_capacity(original_lanes);
    for mut lane in lanes.drain(..) {
        let chats = std::mem::take(&mut lane.chats);
        let base_size = canonical_json(&lane)?.len().saturating_add(1);
        if base_size > *remaining {
            continue;
        }
        *remaining -= base_size;
        for chat in chats {
            let size = canonical_json(&chat)?.len().saturating_add(1);
            if size <= *remaining {
                *remaining -= size;
                lane.chats.push(chat);
            }
        }
        kept.push(lane);
    }
    let kept_chats = kept.iter().map(|lane| lane.chats.len()).sum::<usize>();
    let truncated = kept.len() != original_lanes || kept_chats != original_chats;
    *lanes = kept;
    Ok(truncated)
}
