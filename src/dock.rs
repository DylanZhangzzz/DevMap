use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

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
    DevelopmentTarget, ForkPoint, GitRelationship, GitRelationshipReport, GitRelationshipResolver,
    IntegrationBranch,
};
use crate::git_topology::{GitTopologyCollector, TopologyBoundary, TopologyGraph};
use crate::journal::{JournalIntegrity, JournalSummary, summarize_existing_sessions};
use crate::presence::{
    Confidence, PresenceLoadReport, PresenceRecord, PresenceStatus, PresenceStore, StatusSource,
};
use crate::worktrees::{WorktreeDescriptor, WorktreeScanner, repository_id};

pub const DOCK_SCHEMA_VERSION: &str = "devmap/dock/4";
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
pub struct WriterEvidence {
    pub task_id: String,
    pub observed_at: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceFacts {
    pub worktree_id: String,
    pub head_oid: String,
    pub detached: bool,
    pub head_ref_coverage: String,
    pub integration: String,
    pub target_ref: Option<String>,
    pub merge_commit_oid: Option<String>,
    pub working_state: String,
    pub upstream: String,
    pub task_observed_at: Option<String>,
    pub git_observed_at: Option<String>,
    pub writer_evidence: Vec<WriterEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskObservation {
    pub observed_at: Option<String>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockCounts {
    pub workspaces: usize,
    pub tasks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockReadModel {
    pub route_plans: Vec<crate::route_plan::RoutePlan>,
    pub schema_version: &'static str,
    pub repository_id: String,
    pub revision: u64,
    pub observation_revision: u64,
    pub generated_at: String,
    pub current_worktree_id: String,
    pub development_target: Option<DevelopmentTarget>,
    pub integration_branches: Vec<IntegrationBranch>,
    pub branch_groups: Vec<BranchGroup>,
    pub topology: TopologyGraph,
    pub workspace_facts: Vec<WorkspaceFacts>,
    pub task_observation: TaskObservation,
    pub counts: DockCounts,
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
        // These independent read-only scans share the observed worktree inventory.
        if !worktrees.iter().any(|worktree| worktree.is_current) {
            return Err(DevMapError::InvalidPresence(
                "current worktree is missing".into(),
            ));
        }
        // Overlap Git process latency rather than serializing both scan pipelines.
        let (topology, relationships) = std::thread::scope(|scope| {
            let topology = scope.spawn(|| GitTopologyCollector::scan(workspace, &worktrees));
            let relationships = GitRelationshipResolver::resolve(workspace, &worktrees);
            Ok::<_, DevMapError>((
                topology.join().expect("topology worker panicked")?,
                relationships?,
            ))
        })?;
        self.reduce_with_inputs(
            workspace,
            worktrees,
            presence,
            journals,
            now,
            observed_tasks,
            TaskObservation {
                observed_at: None,
                complete: false,
            },
            topology,
            Some(relationships),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn reduce_with_inputs(
        &self,
        workspace: &SourceWorkspace,
        worktrees: Vec<WorktreeDescriptor>,
        presence: PresenceLoadReport,
        journals: BTreeMap<String, JournalSummary>,
        now: OffsetDateTime,
        observed_tasks: &[ObservedTask],
        task_observation: TaskObservation,
        topology: TopologyGraph,
        relationships: Option<GitRelationshipReport>,
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
        let relationship_report = match relationships {
            Some(report) => report,
            None => GitRelationshipResolver::resolve(workspace, &worktrees)?,
        };
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
                        chat.codex_thread_id = Some(task.session_id.clone());
                        chat.association_source = "codex_task_cwd";
                        chat.host = task.host.clone();
                        chat.status = task.status;
                        chat.status_source = StatusSource::HostExplicit;
                        chat.confidence = Confidence::Observed;
                        chat.last_event_at = task.updated_at.clone();
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
                            status_observed: false,
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
        let git_observed_at = now.format(&Rfc3339)?;
        let workspace_facts = workspace_facts(
            &lanes,
            &topology,
            &relationship_report.integration_branches,
            task_observation.observed_at.as_deref(),
            &git_observed_at,
        );
        let counts = DockCounts {
            workspaces: worktrees
                .iter()
                .map(|worktree| worktree.worktree_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            tasks: observed_tasks
                .iter()
                .filter(|task| {
                    worktrees
                        .iter()
                        .any(|worktree| same_workspace_path(&task.workspace_path, &worktree.root))
                })
                .map(|task| task.session_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
        };
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
            route_plans: Vec::new(),
            schema_version: DOCK_SCHEMA_VERSION,
            repository_id,
            revision: 0,
            observation_revision: 0,
            generated_at: now.format(&Rfc3339)?,
            current_worktree_id,
            development_target: relationship_report.target,
            integration_branches: relationship_report.integration_branches,
            branch_groups,
            topology,
            workspace_facts,
            task_observation,
            counts,
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
        let mut workspace_facts = self.workspace_facts.clone();
        for facts in &mut workspace_facts {
            facts.task_observed_at = None;
            facts.git_observed_at = None;
        }
        #[derive(Serialize)]
        struct Content<'a> {
            route_plans: &'a [crate::route_plan::RoutePlan],
            schema_version: &'a str,
            repository_id: &'a str,
            current_worktree_id: &'a str,
            development_target: &'a Option<DevelopmentTarget>,
            integration_branches: &'a [IntegrationBranch],
            branch_groups: &'a [BranchGroup],
            topology: &'a TopologyGraph,
            workspace_facts: &'a [WorkspaceFacts],
            task_observation_complete: bool,
            counts: &'a DockCounts,
            lanes: &'a [DockLane],
            current: &'a [DockEntry],
            active: &'a [DockEntry],
            stale_or_uninstrumented: &'a [DockEntry],
            warnings: &'a [DockWarning],
            truncated: bool,
        }
        let bytes = canonical_json(&Content {
            route_plans: &self.route_plans,
            schema_version: self.schema_version,
            repository_id: &self.repository_id,
            current_worktree_id: &self.current_worktree_id,
            development_target: &self.development_target,
            integration_branches: &self.integration_branches,
            branch_groups: &self.branch_groups,
            topology: &self.topology,
            workspace_facts: &workspace_facts,
            task_observation_complete: self.task_observation.complete,
            counts: &self.counts,
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
    observation_revision: u64,
    content_hash: Option<String>,
    snapshot: Option<DockReadModel>,
    observed_tasks: Vec<ObservedTask>,
    task_inventory_synced_at: Option<String>,
    task_inventory_complete: bool,
    topology_cache_key: Option<String>,
    topology_cache: Option<TopologyGraph>,
}

impl DockService {
    pub fn open(source: &Path) -> Result<Self, DevMapError> {
        let workspace = SourceGitInspector::open(source)?.workspace_allow_unborn()?;
        let mut service = Self {
            workspace,
            reducer: DockReducer::new(NoRoutes),
            revision: 0,
            observation_revision: 0,
            content_hash: None,
            snapshot: None,
            observed_tasks: Vec::new(),
            task_inventory_synced_at: None,
            task_inventory_complete: false,
            topology_cache_key: None,
            topology_cache: None,
        };
        service.refresh(OffsetDateTime::now_utc())?;
        Ok(service)
    }

    pub fn replace_observed_tasks(
        &mut self,
        tasks: Vec<ObservedTask>,
        now: OffsetDateTime,
    ) -> Result<&DockReadModel, DevMapError> {
        self.replace_observed_tasks_with_completeness(tasks, true, now)
    }

    pub fn replace_observed_tasks_with_completeness(
        &mut self,
        tasks: Vec<ObservedTask>,
        complete: bool,
        now: OffsetDateTime,
    ) -> Result<&DockReadModel, DevMapError> {
        self.replace_observed_tasks_preserving_timestamp(tasks, complete, now, now)
    }

    pub(crate) fn replace_observed_tasks_preserving_timestamp(
        &mut self,
        mut tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
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
        self.observed_tasks = tasks;
        self.task_inventory_synced_at = Some(observed_at.format(&Rfc3339)?);
        self.task_inventory_complete = complete;
        self.refresh(now)
    }

    pub fn refresh(&mut self, now: OffsetDateTime) -> Result<&DockReadModel, DevMapError> {
        let worktrees = WorktreeScanner::scan(&self.workspace)?;
        let topology_key = topology_cache_key(&self.workspace, &worktrees)?;
        let topology = match topology_key {
            Some(key) if self.topology_cache_key.as_deref() == Some(&key) => self
                .topology_cache
                .clone()
                .expect("topology cache key is only stored with a graph"),
            Some(key) => {
                let topology = GitTopologyCollector::scan(&self.workspace, &worktrees)?;
                self.topology_cache_key = Some(key);
                self.topology_cache = Some(topology.clone());
                topology
            }
            None => {
                self.topology_cache_key = None;
                self.topology_cache = None;
                GitTopologyCollector::scan(&self.workspace, &worktrees)?
            }
        };
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
        let task_observation = TaskObservation {
            observed_at: self.task_inventory_synced_at.clone(),
            complete: self.task_inventory_complete,
        };
        let mut next = self.reducer.reduce_with_inputs(
            &self.workspace,
            worktrees,
            presence,
            journals,
            now,
            &self.observed_tasks,
            task_observation,
            topology,
            None,
        )?;
        next.task_inventory_synced_at = self.task_inventory_synced_at.clone();
        match crate::route_plan::RoutePlanStore::open(&self.workspace)
            .and_then(|store| store.list())
        {
            Ok(plans) => next.route_plans = plans,
            Err(_) => next.warnings.push(DockWarning {
                code: "route_plans_unavailable".into(),
                subject_id: None,
            }),
        }
        let mut target_cache = BTreeMap::new();
        for plan in &next.route_plans {
            if !next
                .lanes
                .iter()
                .any(|lane| lane.worktree_id == plan.worktree_id)
            {
                next.warnings.push(DockWarning {
                    code: "planned_workspace_unavailable".into(),
                    subject_id: Some(plan.route_id.clone()),
                });
            }
            if let Some(target) = &plan.target_ref {
                let exists = if let Some(exists) = target_cache.get(target) {
                    *exists
                } else {
                    let exists = std::process::Command::new("git")
                        .arg("-C")
                        .arg(&self.workspace.root)
                        .args(["show-ref", "--verify", "--quiet", target])
                        .output()?
                        .status
                        .success();
                    target_cache.insert(target.clone(), exists);
                    exists
                };
                if !exists {
                    next.warnings.push(DockWarning {
                        code: "planned_target_unavailable".into(),
                        subject_id: Some(plan.route_id.clone()),
                    });
                }
            }
        }
        if let Some(previous) = &self.snapshot {
            for lane in &next.lanes {
                let Some(old) = previous
                    .lanes
                    .iter()
                    .find(|old| old.worktree_id == lane.worktree_id)
                else {
                    continue;
                };
                if old.head == lane.head {
                    next.warnings.extend(
                        previous
                            .warnings
                            .iter()
                            .filter(|w| {
                                w.subject_id.as_deref() == Some(lane.worktree_id.as_str())
                                    && matches!(
                                        w.code.as_str(),
                                        "workspace_history_changed"
                                            | "workspace_history_unverified"
                                    )
                            })
                            .cloned(),
                    );
                } else if !old.head.is_empty() && !lane.head.is_empty() {
                    let status = std::process::Command::new("git")
                        .arg("-C")
                        .arg(&self.workspace.root)
                        .args(["merge-base", "--is-ancestor", &old.head, &lane.head])
                        .output()?
                        .status;
                    if !status.success() {
                        next.warnings.push(DockWarning {
                            code: if status.code() == Some(1) {
                                "workspace_history_changed"
                            } else {
                                "workspace_history_unverified"
                            }
                            .into(),
                            subject_id: Some(lane.worktree_id.clone()),
                        });
                    }
                }
            }
        }
        next = bound_model(next)?;
        let content_hash = next.content_hash()?;
        if self.content_hash.as_deref() != Some(&content_hash) {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(DevMapError::DockRevisionOverflow)?;
            self.content_hash = Some(content_hash);
        }
        self.observation_revision = self
            .observation_revision
            .checked_add(1)
            .ok_or(DevMapError::DockRevisionOverflow)?;
        next.revision = self.revision.max(1);
        next.observation_revision = self.observation_revision;
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

    pub fn task_inventory_complete(&self) -> bool {
        self.task_inventory_complete
    }

    pub fn task_inventory_observed_at(&self) -> Option<&str> {
        self.task_inventory_synced_at.as_deref()
    }
}

fn topology_cache_key(
    workspace: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
) -> Result<Option<String>, DevMapError> {
    let mut bytes = Vec::new();
    for worktree in worktrees {
        bytes.extend_from_slice(worktree.worktree_id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(worktree.head.as_bytes());
        bytes.push(0);
        if let Some(branch) = &worktree.branch {
            bytes.extend_from_slice(branch.as_bytes());
        }
        bytes.push(0xff);
    }
    let inputs = [
        vec![
            OsString::from("for-each-ref"),
            OsString::from("--count=257"),
            OsString::from("--sort=refname"),
            OsString::from("--format=%(refname)%00%(objectname)%00%(*objectname)"),
            OsString::from("refs/heads"),
            OsString::from("refs/remotes"),
            OsString::from("refs/tags"),
        ],
        vec![
            OsString::from("rev-parse"),
            OsString::from("--is-shallow-repository"),
        ],
    ];
    for (index, args) in inputs.into_iter().enumerate() {
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .args(&args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .output()?;
        if !output.status.success() {
            return Err(DevMapError::GitCommand {
                command: format!(
                    "git {}",
                    args.iter()
                        .map(|value| value.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        if index == 1 && output.stdout.starts_with(b"true") {
            return Ok(None);
        }
        bytes.extend_from_slice(&output.stdout);
        bytes.push(0xfe);
    }
    Ok(Some(format!("sha256-{}", sha256_hex(&bytes))))
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
        status_source: StatusSource::HostExplicit,
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

fn workspace_facts(
    lanes: &[DockLane],
    topology: &TopologyGraph,
    integration_branches: &[IntegrationBranch],
    task_observed_at: Option<&str>,
    git_observed_at: &str,
) -> Vec<WorkspaceFacts> {
    lanes
        .iter()
        .map(|lane| {
            let relationship = &lane.relationship;
            let integration = match (relationship.merge_target.as_ref(), relationship.ahead) {
                (None, _) => "terminal",
                (Some(_), Some(0)) => "included",
                (Some(_), Some(_)) => "ahead",
                (Some(_), None) => "unknown",
            };
            let target_ref = relationship.merge_target.as_ref().and_then(|target| {
                integration_branches
                    .iter()
                    .find(|branch| branch.name == *target)
                    .map(|branch| branch.ref_name.clone())
            });
            let protected = topology
                .refs
                .iter()
                .any(|reference| ref_reaches_head(topology, &reference.oid, &lane.head));
            let head_ref_coverage =
                if lane.head.is_empty() || lane.head.bytes().all(|byte| byte == b'0') {
                    "unknown"
                } else if protected {
                    "protected"
                } else if topology.complete {
                    "unprotected"
                } else {
                    "unknown"
                };
            let published = topology.refs.iter().any(|reference| {
                reference.kind == "remote" && ref_reaches_head(topology, &reference.oid, &lane.head)
            });
            WorkspaceFacts {
                worktree_id: lane.worktree_id.clone(),
                head_oid: lane.head.clone(),
                detached: lane.branch.is_none(),
                head_ref_coverage: head_ref_coverage.into(),
                integration: integration.into(),
                target_ref,
                merge_commit_oid: None,
                working_state: if !relationship.status_observed {
                    "unknown"
                } else if relationship.dirty {
                    "dirty"
                } else {
                    "clean"
                }
                .into(),
                upstream: if published { "published" } else { "unknown" }.into(),
                task_observed_at: task_observed_at.map(str::to_owned),
                git_observed_at: Some(git_observed_at.to_owned()),
                writer_evidence: Vec::new(),
            }
        })
        .collect()
}

fn ref_reaches_head(topology: &TopologyGraph, ref_oid: &str, head_oid: &str) -> bool {
    if head_oid.is_empty() || head_oid.bytes().all(|byte| byte == b'0') {
        return false;
    }
    let commits = topology
        .commits
        .iter()
        .map(|commit| (commit.oid.as_str(), commit.parents.as_slice()))
        .collect::<HashMap<_, _>>();
    let mut pending = vec![ref_oid];
    let mut visited = HashSet::new();
    while let Some(oid) = pending.pop() {
        if oid == head_oid {
            return true;
        }
        if !visited.insert(oid) {
            continue;
        }
        if let Some(parents) = commits.get(oid) {
            pending.extend(parents.iter().map(String::as_str));
        }
    }
    false
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
    // Leave room for the service's final revision and observation envelope.
    const ENVELOPE_RESERVE: usize = 2048;
    let ceiling = MAX_DOCK_MODEL_BYTES - ENVELOPE_RESERVE;
    while !model.route_plans.is_empty() && canonical_json(&model)?.len() > ceiling {
        model.route_plans.pop();
        model.truncated = true;
        if !model
            .warnings
            .iter()
            .any(|w| w.code == "route_plans_truncated")
        {
            model.warnings.push(DockWarning {
                code: "route_plans_truncated".into(),
                subject_id: None,
            });
        }
    }
    let attached_heads = model
        .workspace_facts
        .iter()
        .map(|facts| facts.head_oid.clone())
        .filter(|oid| !oid.is_empty() && !oid.bytes().all(|byte| byte == b'0'))
        .collect::<BTreeSet<_>>();
    let mut topology_budget = ceiling;
    let mut output_truncated =
        retain_topology_within_budget(&mut model.topology, &attached_heads, &mut topology_budget)?;
    if canonical_json(&model)?.len() > ceiling {
        // Lanes are the authoritative named/canonical workspace inventory. Facts
        // and exact HEADs are mandatory; compatibility duplicates are expendable.
        model.branch_groups.clear();
        model.current.clear();
        model.active.clear();
        model.stale_or_uninstrumented.clear();
        let tasks = model
            .lanes
            .iter_mut()
            .map(|lane| (lane.worktree_id.clone(), std::mem::take(&mut lane.chats)))
            .collect::<Vec<_>>();
        for (id, chats) in &tasks {
            if !chats.is_empty() {
                model.warnings.push(DockWarning {
                    code: "workspace_detail_partial".into(),
                    subject_id: Some(id.clone()),
                });
            }
        }
        let required_bytes = canonical_json(&model)?.len() - canonical_json(&model.topology)?.len();
        let Some(mut remaining) = ceiling.checked_sub(required_bytes) else {
            return Err(DevMapError::ResourceLimit {
                resource: "Dock workspace coverage",
                limit: MAX_DOCK_MODEL_BYTES,
            });
        };
        // Spend only the actual remaining budget on history, preserving every
        // workspace attachment through an explicit boundary when needed.
        retain_topology_within_budget(&mut model.topology, &attached_heads, &mut remaining)?;
        for (lane, (id, chats)) in model.lanes.iter_mut().zip(tasks) {
            let original_count = chats.len();
            for chat in chats {
                let size = canonical_json(&chat)?.len() + 1;
                if size <= remaining {
                    remaining -= size;
                    lane.chats.push(chat);
                }
            }
            if lane.chats.len() != original_count {
                model.task_observation.complete = false;
            } else {
                model.warnings.retain(|warning| {
                    warning.code != "workspace_detail_partial"
                        || warning.subject_id.as_ref() != Some(&id)
                });
            }
        }
        output_truncated = true;
    }
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
fn retain_topology_within_budget(
    topology: &mut TopologyGraph,
    attached_heads: &BTreeSet<String>,
    remaining: &mut usize,
) -> Result<bool, DevMapError> {
    let mut truncated = false;
    for commit in &mut topology.commits {
        if commit
            .subject
            .as_ref()
            .is_some_and(|subject| subject.len() > 512)
        {
            commit.subject = commit.subject.as_deref().map(|subject| {
                let mut end = 512;
                while !subject.is_char_boundary(end) {
                    end -= 1;
                }
                subject[..end].to_owned()
            });
            truncated = true;
        }
    }
    if canonical_json(topology)?.len() > *remaining {
        for commit in &mut topology.commits {
            commit.subject = None;
        }
        truncated = true;
    }
    if canonical_json(topology)?.len() > *remaining {
        for commit in &mut topology.commits {
            commit.authored_at = None;
        }
        truncated = true;
    }
    while canonical_json(topology)?.len() > *remaining && !topology.commits.is_empty() {
        let remove = (topology.commits.len() / 8).max(1);
        topology
            .commits
            .truncate(topology.commits.len().saturating_sub(remove));
        topology.complete = false;
        truncated = true;
        rebuild_budget_boundaries(topology, attached_heads);
    }
    let size = canonical_json(topology)?.len();
    if size > *remaining {
        return Err(DevMapError::ResourceLimit {
            resource: "Dock topology",
            limit: *remaining,
        });
    }
    *remaining -= size;
    Ok(truncated)
}

fn rebuild_budget_boundaries(topology: &mut TopologyGraph, attached_heads: &BTreeSet<String>) {
    let retained = topology
        .commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    topology
        .edges
        .retain(|edge| retained.contains(edge.to_oid.as_str()));
    let mut boundaries = topology
        .boundaries
        .iter()
        .map(|boundary| {
            (
                (boundary.reason.clone(), boundary.oid.clone()),
                boundary.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for edge in &topology.edges {
        if !retained.contains(edge.from_oid.as_str()) {
            let key = ("history_limit".to_owned(), edge.from_oid.clone());
            boundaries.entry(key).or_insert_with(|| TopologyBoundary {
                id: format!("boundary:history_limit:{}", edge.from_oid),
                oid: edge.from_oid.clone(),
                reason: "history_limit".into(),
            });
        }
    }
    for reference in &topology.refs {
        if !retained.contains(reference.oid.as_str()) {
            let key = ("history_limit".to_owned(), reference.oid.clone());
            boundaries.entry(key).or_insert_with(|| TopologyBoundary {
                id: format!("boundary:history_limit:{}", reference.oid),
                oid: reference.oid.clone(),
                reason: "history_limit".into(),
            });
        }
    }
    for head_oid in attached_heads {
        if !retained.contains(head_oid.as_str()) {
            let key = ("history_limit".to_owned(), head_oid.clone());
            boundaries.entry(key).or_insert_with(|| TopologyBoundary {
                id: format!("boundary:history_limit:{head_oid}"),
                oid: head_oid.clone(),
                reason: "history_limit".into(),
            });
        }
    }
    topology.boundaries = boundaries.into_values().collect();
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn coverage_model(path_repetitions: usize) -> DockReadModel {
        // Synthetic valid-length canonical identities isolate the hard ceiling;
        // no operating-system path-length assumption or Git fixture is needed.
        let head = "a".repeat(40);
        let lanes = (0..256)
            .map(|index| DockLane {
                worktree_id: format!("wt-{index:064x}"),
                workspace_path: format!(
                    "C:/workspaces/{index}/{}",
                    "long/".repeat(path_repetitions)
                ),
                is_current: index == 0,
                branch: None,
                head: head.clone(),
                chats: vec![],
                relationship: GitRelationship {
                    base_target: None,
                    merge_target: None,
                    merged: None,
                    ahead: None,
                    behind: None,
                    dirty: true,
                    changed_file_count: 1,
                    status_observed: true,
                    fork_point: None,
                },
            })
            .collect::<Vec<_>>();
        let topology = TopologyGraph {
            commits: vec![crate::git_topology::TopologyCommit {
                oid: head.clone(),
                parents: vec![],
                authored_at: None,
                subject: None,
            }],
            refs: vec![],
            edges: vec![],
            boundaries: vec![],
            complete: true,
        };
        let facts = workspace_facts(
            &lanes,
            &topology,
            &[],
            Some("2026-09-05T00:00:00Z"),
            "2026-09-05T00:00:00Z",
        );
        DockReadModel {
            route_plans: Vec::new(),
            schema_version: DOCK_SCHEMA_VERSION,
            repository_id: format!("sha256-{}", "b".repeat(64)),
            revision: 1,
            observation_revision: 1,
            generated_at: "2026-09-05T00:00:00Z".into(),
            current_worktree_id: lanes[0].worktree_id.clone(),
            development_target: None,
            integration_branches: vec![],
            branch_groups: vec![],
            topology,
            workspace_facts: facts,
            task_observation: TaskObservation {
                observed_at: Some("2026-09-05T00:00:00Z".into()),
                complete: true,
            },
            counts: DockCounts {
                workspaces: 256,
                tasks: 0,
            },
            task_inventory_synced_at: None,
            lanes,
            current: vec![],
            active: vec![],
            stale_or_uninstrumented: vec![],
            warnings: vec![],
            truncated: false,
        }
    }

    #[test]
    fn irreducible_workspace_coverage_exceeding_ceiling_fails_explicitly() {
        let model = coverage_model(800);
        assert!(matches!(
            bound_model(model),
            Err(DevMapError::ResourceLimit {
                resource: "Dock workspace coverage",
                limit: MAX_DOCK_MODEL_BYTES
            })
        ));
    }

    #[test]
    fn history_pruning_reserves_an_explicit_boundary_for_every_checkout_head() {
        let mut model = coverage_model(60);
        let head = "a".repeat(40);
        for index in 1..2048 {
            let parent = model.topology.commits.last().unwrap().oid.clone();
            let oid = format!("{index:040x}");
            model
                .topology
                .edges
                .push(crate::git_topology::TopologyEdge {
                    id: format!("edge:{parent}:{oid}"),
                    from_oid: parent.clone(),
                    to_oid: oid.clone(),
                });
            model
                .topology
                .commits
                .push(crate::git_topology::TopologyCommit {
                    oid,
                    parents: vec![parent],
                    authored_at: None,
                    subject: None,
                });
        }
        model.topology.commits.reverse();
        let bounded = bound_model(model).unwrap();
        assert!(bounded.truncated);
        assert_eq!(bounded.lanes.len(), 256);
        assert_eq!(bounded.workspace_facts.len(), 256);
        assert!(
            !bounded
                .topology
                .commits
                .iter()
                .any(|commit| commit.oid == head)
        );
        assert!(
            bounded
                .topology
                .boundaries
                .iter()
                .any(|boundary| boundary.oid == head && boundary.reason == "history_limit")
        );
        assert!(canonical_json(&bounded).unwrap().len() <= MAX_DOCK_MODEL_BYTES);
    }
}
