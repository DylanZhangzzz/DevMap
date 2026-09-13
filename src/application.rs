//! Shared repository observations with client-owned presentation state.
use crate::{
    dock::{self, DockProjectionContext, DockReadModel, ObservedTask, TaskLifecycle},
    error::DevMapError,
    git::SourceWorkspace,
    store::snapshot::InputReader,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Survives owner replacement in the proxy. Inventory time is never a query time.
pub struct ClientView {
    workspace: SourceWorkspace,
    tasks: Vec<ObservedTask>,
    observed_at: Option<OffsetDateTime>,
    complete: bool,
    revision: u64,
    observation_revision: u64,
    content_hash: Option<String>,
    snapshot: Option<DockReadModel>,
}
impl ClientView {
    pub fn new(workspace: SourceWorkspace) -> Self {
        Self {
            workspace,
            tasks: vec![],
            observed_at: None,
            complete: false,
            revision: 0,
            observation_revision: 0,
            content_hash: None,
            snapshot: None,
        }
    }
    pub fn workspace(&self) -> &SourceWorkspace {
        &self.workspace
    }
    pub fn observed_tasks(&self) -> &[ObservedTask] {
        &self.tasks
    }
    pub fn inventory_observed_at(&self) -> Option<OffsetDateTime> {
        self.observed_at
    }
    pub fn inventory_complete(&self) -> bool {
        self.complete
    }
    pub fn snapshot(&self) -> Option<&DockReadModel> {
        self.snapshot.as_ref()
    }
    pub fn query_input(&self) -> Result<ClientQuery, DevMapError> {
        Ok(ClientQuery {
            tasks: self.tasks.clone(),
            inventory_observed_at: self.observed_at.map(|t| t.format(&Rfc3339)).transpose()?,
            complete: self.complete,
            previous_heads: dock::previous_heads(self.snapshot.as_ref()),
        })
    }
    /// Installs the accepted inventory response while keeping local presentation counters.
    pub fn apply_inventory(&mut self, input: ClientQuery) -> Result<(), DevMapError> {
        input.validate()?;
        let observed_at = input
            .inventory_observed_at
            .as_deref()
            .map(|t| {
                OffsetDateTime::parse(t, &Rfc3339)
                    .map_err(|_| DevMapError::InvalidDomain("inventory observation time"))
            })
            .transpose()?;
        if self.observed_at > observed_at {
            return Ok(());
        }
        self.tasks = input.tasks;
        self.observed_at = observed_at;
        self.complete = input.complete;
        Ok(())
    }
    /// Called in the surviving proxy, never in the repository owner after IPC.
    pub fn apply_projection(
        &mut self,
        mut result: ApplicationSnapshot,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        if result.model.repository_id != crate::worktrees::repository_id(&self.workspace) {
            return Err(DevMapError::InvalidDomain("projection repository mismatch"));
        }
        if !result.model.lanes.iter().any(|lane| {
            lane.is_current
                && result.model.current_worktree_id == lane.worktree_id
                && dock::same_workspace_path(&lane.workspace_path, &self.workspace.root)
        }) {
            return Err(DevMapError::InvalidDomain("projection workspace mismatch"));
        }
        result.model = dock::finalize_revisions(
            result.model,
            &mut self.revision,
            &mut self.observation_revision,
            &mut self.content_hash,
        )?;
        self.snapshot = Some(result.model.clone());
        Ok(result)
    }
}

/// Bounded transport input. Source workspace comes separately from authenticated Hello.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientQuery {
    pub tasks: Vec<ObservedTask>,
    pub inventory_observed_at: Option<String>,
    pub complete: bool,
    pub previous_heads: Vec<dock::PreviousHead>,
}
impl ClientQuery {
    fn validate(&self) -> Result<(), DevMapError> {
        if self.tasks.len() > 2048
            || self.previous_heads.len() > 256
            || serde_json::to_vec(self)?.len() > 2 * 1024 * 1024
        {
            return Err(DevMapError::InvalidDomain("query input limit"));
        }
        if self
            .inventory_observed_at
            .as_ref()
            .is_some_and(|t| timestamp(t).is_none())
        {
            return Err(DevMapError::InvalidDomain("inventory observation time"));
        }
        let mut tasks = std::collections::BTreeSet::new();
        for task in &self.tasks {
            if task.session_id.is_empty()
                || task.session_id.len() > 256
                || !tasks.insert(&task.session_id)
            {
                return Err(DevMapError::InvalidDomain("codex_tasks.id"));
            }
            if let Some(report) = &task.working_directory
                && (report.source != "agent_report"
                    || timestamp(&report.observed_at).is_none()
                    || !std::path::Path::new(&report.path).is_absolute())
            {
                return Err(DevMapError::InvalidDomain("codex_tasks.workingDirectory"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for h in &self.previous_heads {
            if !seen.insert(&h.worktree_id)
                || h.worktree_id.len() > 256
                || h.worktree_id.is_empty()
                || h.worktree_id.chars().any(char::is_control)
                || !(h.head.is_empty()
                    || (matches!(h.head.len(), 40 | 64)
                        && h.head.bytes().all(|b| b.is_ascii_hexdigit())))
            {
                return Err(DevMapError::InvalidDomain("previous workspace head"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplicationSnapshot {
    pub model: DockReadModel,
    pub store_generation: Option<u64>,
    pub store_inputs_observed_at: Option<String>,
    pub git_observed_at: String,
    pub git_cycle: u64,
}

pub struct RepositoryApplication {
    workspace: SourceWorkspace,
    common_dir: PathBuf,
    common_identity: crate::fs_security::FileIdentity,
    anchor: crate::store::migration::VerifiedCurrentOrigin,
    storage: InputReader,
    git: Option<DockProjectionContext>,
    git_at: Option<OffsetDateTime>,
    collected: Option<Instant>,
    git_max_age: Duration,
    cycle: u64,
    dirty: bool,
    targets: Vec<String>,
    ancestry: BTreeMap<(String, String), Option<dock::HistoryWarning>>,
    origin_fingerprint: Option<String>,
    #[cfg(test)]
    pub(crate) test_after_storage: Option<Box<dyn FnMut() -> Result<(), DevMapError> + Send>>,
}
pub(crate) enum QueryBoundaryAttempt {
    Ineligible,
    SourceStale,
    Ready(Box<ApplicationSnapshot>),
}
struct QueryAttemptGuard<'a> {
    application: &'a mut RepositoryApplication,
    committed: bool,
}
impl Drop for QueryAttemptGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.application.discard_query_caches();
        }
    }
}
impl RepositoryApplication {
    fn discard_query_caches(&mut self) {
        self.reconcile();
        self.git = None;
        self.git_at = None;
        self.collected = None;
        self.origin_fingerprint = None;
        self.targets.clear();
    }
    fn query_anchor_unchanged(&self, workspace: &SourceWorkspace) -> Result<bool, DevMapError> {
        Ok(self.workspace.root == workspace.root
            && self.workspace.git_dir == workspace.git_dir
            && self.workspace.git_common_dir == workspace.git_common_dir
            && self.common_dir == workspace.git_common_dir
            && crate::fs_security::checked_directory_identity(&self.common_dir)?
                == self.common_identity
            && self.anchor.application_location(&self.common_dir)?.as_ref() == Some(&self.anchor))
    }
    pub(crate) fn try_project_query_boundary(
        &mut self,
        source: &crate::runtime::query_validation::VerifiedQuerySource,
        query: &ClientQuery,
        now: OffsetDateTime,
        after_projection: impl FnOnce(),
    ) -> Result<QueryBoundaryAttempt, DevMapError> {
        // This seal is authorization, not the eligibility comparison below.
        let workspace = source.workspace()?.clone();
        if workspace.root != self.workspace.root
            || workspace.git_dir != self.workspace.git_dir
            || workspace.git_common_dir != self.workspace.git_common_dir
        {
            return Ok(QueryBoundaryAttempt::Ineligible);
        }
        let Some(epoch) = self
            .storage
            .take_query_epoch(&workspace, source.configuration())
        else {
            return Ok(QueryBoundaryAttempt::Ineligible);
        };
        let mut attempt = QueryAttemptGuard {
            application: self,
            committed: false,
        };
        let (source_valid, origin_valid) = epoch.boundary(&workspace, source.configuration())?;
        if !source_valid {
            return Ok(QueryBoundaryAttempt::SourceStale);
        }
        if !origin_valid || !attempt.application.query_anchor_unchanged(&workspace)? {
            return Ok(QueryBoundaryAttempt::Ineligible);
        }
        // Original read-only body, including all SQL/hash work, Git refreshes,
        // relationship/history misses, timestamps and source/anchor checks.
        let result = attempt.application.project_inner_in_epoch(
            &workspace,
            query,
            now,
            Some(source),
            Some(&epoch),
        )?;
        after_projection();
        // Closing origin drift retains precedence over a source-only stale
        // result. No closing capture runs after a failed or panicking body.
        let (source_valid, origin_valid) = epoch.boundary(&workspace, source.configuration())?;
        if !origin_valid {
            return Err(DevMapError::Store(
                "origin enumeration proof changed after observation".into(),
            ));
        }
        if !attempt.application.query_anchor_unchanged(&workspace)? {
            return Err(DevMapError::Store(
                "application anchor changed during query".into(),
            ));
        }
        source.workspace()?;
        if !source_valid {
            return Ok(QueryBoundaryAttempt::SourceStale);
        }
        attempt
            .application
            .storage
            .restore_query_epoch(&workspace, epoch)?;
        let result = Box::new(result);
        attempt.committed = true;
        Ok(QueryBoundaryAttempt::Ready(result))
    }
    pub fn open(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        Ok(Self {
            workspace: workspace.clone(),
            anchor: crate::store::migration::application_anchor(workspace)?,
            common_identity: crate::fs_security::checked_directory_identity(
                &workspace.git_common_dir,
            )?,
            common_dir: crate::fs_security::checked_canonical_directory(&workspace.git_common_dir)?,
            storage: InputReader::new(),
            git: None,
            git_at: None,
            collected: None,
            git_max_age: Duration::from_secs(2),
            cycle: 0,
            dirty: true,
            targets: vec![],
            ancestry: BTreeMap::new(),
            origin_fingerprint: None,
            #[cfg(test)]
            test_after_storage: None,
        })
    }
    #[cfg(test)]
    pub(crate) fn test_query_caches_discarded(&self) -> bool {
        self.storage.test_cache_discarded()
            && self.git.is_none()
            && self.git_at.is_none()
            && self.collected.is_none()
            && self.ancestry.is_empty()
            && self.targets.is_empty()
            && self.origin_fingerprint.is_none()
            && self.dirty
    }
    /// Explicit freshness bound; default two seconds, never above one minute.
    pub fn with_git_max_age(mut self, age: Duration) -> Result<Self, DevMapError> {
        if age.is_zero() || age > Duration::from_secs(60) {
            return Err(DevMapError::InvalidDomain("Git freshness bound"));
        }
        self.git_max_age = age;
        Ok(self)
    }
    pub fn change_hint(&mut self) {
        self.dirty = true;
    }
    /// Forces the next query to revalidate storage and reconcile Git. No threads.
    pub fn reconcile(&mut self) {
        self.dirty = true;
        self.storage.invalidate();
        self.ancestry.clear();
    }
    fn validate_client(&self, view: &ClientView) -> Result<SourceWorkspace, DevMapError> {
        if crate::fs_security::checked_canonical_directory(&view.workspace.git_common_dir)?
            != self.common_dir
        {
            return Err(DevMapError::InvalidDomain("client repository mismatch"));
        }
        let actual =
            crate::git::SourceGitInspector::open(&view.workspace.root)?.workspace_allow_unborn()?;
        if crate::fs_security::checked_canonical_directory(&actual.git_common_dir)?
            != self.common_dir
            || crate::fs_security::checked_canonical_directory(&actual.git_dir)?
                != crate::fs_security::checked_canonical_directory(&view.workspace.git_dir)?
        {
            return Err(DevMapError::InvalidDomain(
                "client workspace identity mismatch",
            ));
        }
        Ok(actual)
    }
    pub fn query(
        &mut self,
        view: &mut ClientView,
        now: OffsetDateTime,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        let result = self.project(&view.workspace, &view.query_input()?, now)?;
        view.apply_projection(result)
    }
    pub fn project(
        &mut self,
        workspace: &SourceWorkspace,
        query: &ClientQuery,
        now: OffsetDateTime,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        crate::git_process::with_operation(|| self.project_inner(workspace, query, now, None))
    }
    pub(crate) fn project_verified_query(
        &mut self,
        source: &crate::runtime::query_validation::VerifiedQuerySource,
        query: &ClientQuery,
        now: OffsetDateTime,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        crate::git_process::with_operation(|| {
            self.project_inner(source.workspace()?, query, now, Some(source))
        })
    }
    fn project_inner(
        &mut self,
        workspace: &SourceWorkspace,
        query: &ClientQuery,
        now: OffsetDateTime,
        verified: Option<&crate::runtime::query_validation::VerifiedQuerySource>,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        self.project_inner_in_epoch(workspace, query, now, verified, None)
    }
    fn project_inner_in_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        query: &ClientQuery,
        now: OffsetDateTime,
        verified: Option<&crate::runtime::query_validation::VerifiedQuerySource>,
        epoch: Option<&crate::store::snapshot::QueryReadEpoch>,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        query.validate()?;
        let actual = match verified {
            Some(source) => source.workspace()?.clone(),
            None => self.validate_client(&ClientView::new(workspace.clone()))?,
        };
        if crate::fs_security::checked_canonical_directory(&actual.git_common_dir)?
            != self.common_dir
        {
            return Err(DevMapError::InvalidDomain("client repository mismatch"));
        }
        if crate::fs_security::checked_directory_identity(&self.common_dir)? != self.common_identity
        {
            return Err(DevMapError::InvalidDomain(
                "application common identity changed",
            ));
        }
        let location = self.anchor.application_location(&self.common_dir)?;
        let replacement = match location {
            Some(location) if location.workspace_path != self.anchor.workspace_path => {
                // Never invoke Git at an unverified old-path occupant.
                let relocated = crate::git::SourceGitInspector::open(&location.workspace_path)?
                    .workspace_allow_unborn()?;
                let checked = crate::store::migration::application_anchor(&relocated)?;
                if checked != location
                    || crate::fs_security::checked_canonical_directory(&relocated.git_common_dir)?
                        != self.common_dir
                {
                    return Err(DevMapError::InvalidDomain(
                        "application relocated anchor changed",
                    ));
                }
                Some((relocated, checked))
            }
            Some(_) => None,
            None => {
                // Genuine disappearance retains the surviving-client fallback.
                let anchor = crate::store::migration::application_anchor(&actual)?;
                Some((actual, anchor))
            }
        };
        if crate::fs_security::checked_directory_identity(&self.common_dir)? != self.common_identity
        {
            return Err(DevMapError::InvalidDomain(
                "application common identity changed during relocation",
            ));
        }
        if epoch.is_some() && replacement.is_some() {
            return Err(DevMapError::Store(
                "application anchor changed during query".into(),
            ));
        }
        if let Some((workspace, anchor)) = replacement {
            self.workspace = workspace;
            self.anchor = anchor;
            self.dirty = true;
            self.storage.invalidate();
            self.ancestry.clear();
            self.git = None;
            self.collected = None;
        }
        let (generation, inputs) = match (epoch, verified) {
            (Some(epoch), _) => self.storage.read_in_query_epoch(&self.workspace, epoch)?,
            (None, Some(source)) => self
                .storage
                .read_with_configuration(&self.workspace, source.configuration())?,
            (None, None) => self.storage.read(&self.workspace)?,
        };
        #[cfg(test)]
        if let Some(mut hook) = self.test_after_storage.take() {
            hook()?;
        }
        let origin_fingerprint = self.storage.origin_fingerprint().map(str::to_owned);
        let plans = inputs
            .routes
            .as_ref()
            .map(|(plans, _)| plans.as_slice())
            .unwrap_or(&[]);
        let mut targets = plans
            .iter()
            .filter_map(|p| p.target_ref.clone())
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        if self.dirty
            || origin_fingerprint != self.origin_fingerprint
            || self
                .collected
                .is_none_or(|t| t.elapsed() >= self.git_max_age)
            || targets != self.targets
        {
            let git = match DockProjectionContext::collect(&self.workspace, plans) {
                Err(DevMapError::InvalidDomain("Git changed during collection")) => {
                    DockProjectionContext::collect(&self.workspace, plans)?
                }
                result => result?,
            };
            self.ancestry.clear();
            self.git = Some(git);
            self.git_at = Some(OffsetDateTime::now_utc());
            self.collected = Some(Instant::now());
            self.cycle += 1;
            self.targets = targets;
            self.dirty = false;
            self.origin_fingerprint = origin_fingerprint;
        }
        // Collection can finish after the caller sampled its projection clock.
        // Keep evaluation at least as recent as those newly collected facts.
        let now = now.max(self.git_at.unwrap());
        let context = self.git.as_mut().unwrap();
        let prepared = match verified {
            Some(source) => context.prepare_verified_query(source)?,
            None => context.prepare_client(workspace)?,
        };
        let mut next = prepared.project(
            inputs,
            now,
            &query.tasks,
            query.inventory_observed_at.clone(),
            query.complete,
        )?;
        let git_observed_at = self.git_at.unwrap().format(&Rfc3339)?;
        for facts in &mut next.workspace_facts {
            if facts.git_observed_at.is_some() {
                facts.git_observed_at = Some(git_observed_at.clone());
            }
        }
        dock::apply_history(&mut next, &query.previous_heads, |old, next| {
            let key = (old.to_owned(), next.to_owned());
            if let Some(cached) = self.ancestry.get(&key) {
                return Ok(*cached);
            }
            let result = dock::ancestry(workspace, old, next)?;
            if self.ancestry.len() >= 1024 {
                self.ancestry.clear();
            }
            self.ancestry.insert(key, result);
            Ok(result)
        })?;
        Ok(ApplicationSnapshot {
            model: next,
            store_generation: generation,
            store_inputs_observed_at: self
                .storage
                .inputs_observed_at()
                .map(|t| t.format(&Rfc3339))
                .transpose()?,
            git_observed_at,
            git_cycle: self.cycle,
        })
    }

    /// IPC write seam: accepts bounded prior inventory only, never a hydrated map.
    pub fn accept_inventory_query(
        &mut self,
        workspace: &SourceWorkspace,
        prior: ClientQuery,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
    ) -> Result<ClientQuery, DevMapError> {
        self.accept_inventory_query_with_setup(
            workspace,
            prior,
            tasks,
            complete,
            observed_at,
            false,
        )
    }
    pub(crate) fn accept_inventory_query_shared(
        &mut self,
        workspace: &SourceWorkspace,
        prior: ClientQuery,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
    ) -> Result<ClientQuery, DevMapError> {
        self.accept_inventory_query_with_setup(workspace, prior, tasks, complete, observed_at, true)
    }
    fn accept_inventory_query_with_setup(
        &mut self,
        workspace: &SourceWorkspace,
        prior: ClientQuery,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
        setup: bool,
    ) -> Result<ClientQuery, DevMapError> {
        let previous_heads = prior.previous_heads.clone();
        let mut view = ClientView::new(workspace.clone());
        view.apply_inventory(prior)?;
        self.accept_inventory_with_heads(
            &mut view,
            tasks,
            complete,
            observed_at,
            &previous_heads,
            setup,
        )?;
        let mut accepted = view.query_input()?;
        accepted.previous_heads = previous_heads;
        Ok(accepted)
    }
    /// Only this explicit acceptance operation writes binding observations. Older
    /// inventories cannot roll the client watermark back; partial reports merge.
    pub fn accept_inventory(
        &mut self,
        view: &mut ClientView,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
    ) -> Result<(), DevMapError> {
        let previous_heads = dock::previous_heads(view.snapshot.as_ref());
        self.accept_inventory_with_heads(view, tasks, complete, observed_at, &previous_heads, false)
    }
    fn accept_inventory_with_heads(
        &mut self,
        view: &mut ClientView,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
        previous_heads: &[dock::PreviousHead],
        setup: bool,
    ) -> Result<(), DevMapError> {
        crate::git_process::with_operation(|| {
            self.accept_inventory_inner(view, tasks, complete, observed_at, previous_heads, setup)
        })
    }
    fn accept_inventory_inner(
        &mut self,
        view: &mut ClientView,
        mut tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
        previous_heads: &[dock::PreviousHead],
        setup: bool,
    ) -> Result<(), DevMapError> {
        self.validate_client(view)?;
        ClientQuery {
            tasks: tasks.clone(),
            inventory_observed_at: Some(observed_at.format(&Rfc3339)?),
            complete,
            previous_heads: vec![],
        }
        .validate()?;
        tasks.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        if tasks.windows(2).any(|p| p[0].session_id == p[1].session_id) {
            return Err(DevMapError::InvalidDomain("codex_tasks.id"));
        }
        let worktrees = crate::worktrees::WorktreeScanner::scan(&view.workspace)?;
        for task in &tasks {
            if let Some(report) = &task.working_directory {
                let stamp = OffsetDateTime::parse(&report.observed_at, &Rfc3339).map_err(|_| {
                    DevMapError::InvalidDomain("codex_tasks.workingDirectory.observedAt")
                })?;
                if report.source != "agent_report"
                    || stamp > observed_at + time::Duration::seconds(30)
                    || !std::path::Path::new(&report.path).is_absolute()
                    || !std::path::Path::new(&report.path).is_dir()
                    || !worktrees
                        .iter()
                        .any(|w| dock::same_workspace_path(&report.path, &w.root))
                {
                    return Err(DevMapError::InvalidDomain("codex_tasks.workingDirectory"));
                }
            }
        }
        if view.observed_at.is_some_and(|t| observed_at < t) {
            return Ok(());
        }
        for task in &mut tasks {
            if let Some(old) = view
                .tasks
                .iter()
                .find(|old| old.session_id == task.session_id)
            {
                let incoming_report = task.working_directory.clone();
                if timestamp(&task.updated_at) < timestamp(&old.updated_at) {
                    *task = old.clone();
                    task.working_directory = incoming_report;
                }
                if let Some(report) = &old.working_directory
                    && task.working_directory.as_ref().is_none_or(|new| {
                        timestamp(&new.observed_at) < timestamp(&report.observed_at)
                    })
                {
                    task.working_directory = Some(report.clone());
                }
            }
        }
        let associations = tasks
            .iter()
            .filter(|t| t.lifecycle == TaskLifecycle::Present)
            .filter_map(|t| {
                worktrees
                    .iter()
                    .find(|w| dock::same_workspace_path(&t.workspace_path, &w.root))
                    .map(|w| (t.host.clone(), t.session_id.clone(), w.worktree_id.clone()))
            })
            .collect::<Vec<_>>();
        if !complete {
            for old in &view.tasks {
                if !tasks.iter().any(|t| t.session_id == old.session_id) {
                    tasks.push(old.clone());
                }
            }
        }
        tasks.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        // Validate the complete future transport state before any durable effect.
        // Retained partial tasks and history headers both count against the budget.
        ClientQuery {
            tasks: tasks.clone(),
            inventory_observed_at: Some(observed_at.format(&Rfc3339)?),
            complete,
            previous_heads: previous_heads.to_vec(),
        }
        .validate()?;
        let binding_time = observed_at.format(&Rfc3339)?;
        crate::journal::validate_binding_observation(&associations, &binding_time)?;
        if setup {
            // Full merged input and source reports are validated before setup.
            // Direct embedded callers retain their explicit legacy policy.
            crate::store::migration::prepare_first_origin_write(&view.workspace)?;
        }
        crate::journal::observe_task_bindings(&view.workspace, &associations, &binding_time)?;
        view.tasks = tasks;
        view.complete = complete;
        view.observed_at = Some(observed_at);
        // Generation invalidates domain cache automatically; no Git re-scan needed.
        Ok(())
    }
}
fn timestamp(text: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(text, &Rfc3339).ok()
}
