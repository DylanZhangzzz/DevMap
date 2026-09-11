//! A generation-pinned, read-only input cache. No store creation or domain repair.
#[cfg(test)]
mod origin_cache_tests;
use super::RepositoryStore;
use crate::{
    dock::{DockBindingInputs, DockRoutePlans, DockStorageInputs},
    error::DevMapError,
    git::SourceWorkspace,
    journal::{self, JournalIntegrity, JournalSummary},
    presence::{self, PresenceLoadReport, PresenceStore},
    route_plan,
    worktrees::repository_id,
};
use std::collections::{BTreeMap, BTreeSet};

type JournalWatermark = (i64, Option<String>, i64, String, String, String);
type SummaryCache = BTreeMap<String, (JournalWatermark, JournalSummary)>;
type PresenceRegistration = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(crate) struct InputReader {
    origins: super::migration::ReadOriginCache,
    query_owner: std::sync::Arc<()>,
    store: Option<RepositoryStore>,
    identity: Option<crate::fs_security::FileIdentity>,
    cached: Option<(u64, DockStorageInputs)>,
    summaries: SummaryCache,
    data_version: Option<i64>,
    inputs_observed_at: Option<time::OffsetDateTime>,
    origin_fingerprint: Option<String>,
}
// Non-cloneable and non-wire. The cache itself is moved into this invocation,
// and can be restored only to its original, still-current reader generation.
pub(crate) struct QueryReadEpoch {
    origins: super::migration::ReadOriginCache,
    owner: std::sync::Arc<()>,
    workspace: SourceWorkspace,
}
impl QueryReadEpoch {
    fn matches(&self, workspace: &SourceWorkspace) -> bool {
        self.workspace.root == workspace.root
            && self.workspace.git_dir == workspace.git_dir
            && self.workspace.git_common_dir == workspace.git_common_dir
    }
    pub(crate) fn boundary(
        &self,
        workspace: &SourceWorkspace,
        configuration: &crate::git_relationship::QueryConfiguration,
    ) -> Result<(bool, bool), DevMapError> {
        if !self.matches(workspace) {
            return Err(DevMapError::Store("query epoch source changed".into()));
        }
        self.origins.query_boundary(workspace, configuration)
    }
}
impl InputReader {
    pub(crate) fn new() -> Self {
        Self {
            origins: super::migration::ReadOriginCache::default(),
            query_owner: std::sync::Arc::new(()),
            store: None,
            identity: None,
            cached: None,
            summaries: BTreeMap::new(),
            data_version: None,
            inputs_observed_at: None,
            origin_fingerprint: None,
        }
    }
    pub(crate) fn take_query_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        configuration: &crate::git_relationship::QueryConfiguration,
    ) -> Option<QueryReadEpoch> {
        // A retained SQL input snapshot exists only after an active-store read.
        // The body still performs all original live backend/pinned SQL checks.
        if self.store.is_none()
            || self.cached.is_none()
            || !self
                .origins
                .matches_query_configuration(workspace, configuration)
        {
            return None;
        }
        Some(QueryReadEpoch {
            origins: std::mem::take(&mut self.origins),
            owner: self.query_owner.clone(),
            workspace: workspace.clone(),
        })
    }
    fn validate_query_epoch(
        &self,
        workspace: &SourceWorkspace,
        epoch: &QueryReadEpoch,
    ) -> Result<(), DevMapError> {
        if !std::sync::Arc::ptr_eq(&self.query_owner, &epoch.owner)
            || !epoch.matches(workspace)
            || !self.origins.is_empty()
        {
            return Err(DevMapError::Store(
                "query epoch owner/source changed".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn restore_query_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        epoch: QueryReadEpoch,
    ) -> Result<(), DevMapError> {
        self.validate_query_epoch(workspace, &epoch)?;
        self.origins = epoch.origins;
        Ok(())
    }
    pub(crate) fn read_in_query_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        epoch: &QueryReadEpoch,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        self.validate_query_epoch(workspace, epoch)?;
        let result = self.read_checked_in_epoch(workspace, || Ok(()), Some(epoch));
        if result.is_err() {
            self.origins.clear();
            return result;
        }
        // A legacy/backend retry can invalidate the reader internally. Even a
        // successful later read cannot restore the earlier leased generation.
        self.validate_query_epoch(workspace, epoch)?;
        result
    }
    pub(crate) fn read(
        &mut self,
        workspace: &SourceWorkspace,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        let result = self.read_checked(workspace, || Ok(()));
        if result.is_err() {
            self.origins.clear();
        }
        result
    }
    fn read_checked(
        &mut self,
        workspace: &SourceWorkspace,
        after_legacy: impl FnMut() -> Result<(), DevMapError>,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        self.read_checked_in_epoch(workspace, after_legacy, None)
    }
    fn read_checked_in_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        mut after_legacy: impl FnMut() -> Result<(), DevMapError>,
        epoch: Option<&QueryReadEpoch>,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        for _ in 0..2 {
            if let Some(epoch) = epoch {
                self.validate_query_epoch(workspace, epoch)?;
            }
            let result = self.read_after_generation_in_epoch(workspace, || Ok(()), epoch)?;
            if result.0.is_some() {
                return Ok(result);
            }
            self.inputs_observed_at = Some(time::OffsetDateTime::now_utc());
            after_legacy()?;
            // Legacy has no SQL snapshot. Recheck activation/creation after reading
            // the explicitly legacy-only domains, then retry at most once.
            if let Some(probe) = RepositoryStore::open_existing(workspace)? {
                let active = super::is_active(probe.connection())?;
                if active || self.store.is_none() {
                    self.invalidate();
                    continue;
                }
            } else if self.store.is_some() {
                return Err(DevMapError::Store(
                    "database disappeared during legacy read".into(),
                ));
            }
            return Ok(result);
        }
        Err(DevMapError::Store(
            "backend changed during legacy read; retry required".into(),
        ))
    }
    #[cfg(test)]
    fn read_after_generation(
        &mut self,
        workspace: &SourceWorkspace,
        after: impl FnOnce() -> Result<(), DevMapError>,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        self.read_after_generation_in_epoch(workspace, after, None)
    }
    fn read_after_generation_in_epoch(
        &mut self,
        workspace: &SourceWorkspace,
        after: impl FnOnce() -> Result<(), DevMapError>,
        epoch: Option<&QueryReadEpoch>,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        if self.store.is_none() {
            self.store = RepositoryStore::open_existing(workspace)?;
            self.identity = self
                .store
                .as_ref()
                .map(|store| {
                    crate::fs_security::checked_file(store.path(), false, false)
                        .and_then(|f| crate::fs_security::file_identity(&f))
                })
                .transpose()?;
        }
        let Some(store) = &self.store else {
            return Ok((None, legacy(workspace)?));
        };
        // Revalidate path/selector/fence drift on every access until a reviewed bounded
        // owner policy replaces this conservative check. Cache is never a drift bypass.
        let probe = RepositoryStore::open_existing(workspace)?
            .ok_or_else(|| DevMapError::Store("database disappeared".into()))?;
        let identity = crate::fs_security::file_identity(&crate::fs_security::checked_file(
            probe.path(),
            false,
            false,
        )?)?;
        if self.identity.as_ref() != Some(&identity) {
            return Err(DevMapError::Store(
                "database identity changed; explicit reconciliation required".into(),
            ));
        }
        drop(probe);
        let before_version: i64 = store
            .connection()
            .query_row("PRAGMA data_version", [], |r| r.get(0))?;
        let tx = store.connection().unchecked_transaction()?;
        let generation: i64 = tx.query_row(
            "SELECT generation FROM store_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let pinned_version: i64 = tx.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        if before_version != pinned_version || self.data_version != Some(pinned_version) {
            self.cached = None;
            self.summaries.clear();
            self.inputs_observed_at = None;
        }
        let generation = u64::try_from(generation)
            .map_err(|_| DevMapError::Store("negative generation".into()))?;
        super::validate(
            &tx,
            workspace,
            &crate::fs_security::checked_canonical_directory(&workspace.git_common_dir)?,
        )?;
        after()?;
        if !super::is_active(&tx)? {
            tx.commit()?;
            return Ok((None, legacy(workspace)?));
        }
        let origins = match epoch {
            Some(epoch) => epoch.origins.observe_in_query_epoch(workspace, &tx)?,
            None => self.origins.observe(workspace, &tx)?,
        };
        let inputs = match &self.cached {
            Some((cached_generation, inputs)) if *cached_generation == generation => inputs.clone(),
            _ => {
                let inputs = sql_inputs(&tx, &repository_id(workspace), &mut self.summaries)?;
                self.inputs_observed_at = Some(time::OffsetDateTime::now_utc());
                inputs
            }
        };
        let mut qualified = inputs.clone();
        qualify_origins(&tx, &origins, &mut qualified)?;
        tx.commit()?;
        let after_version: i64 = store
            .connection()
            .query_row("PRAGMA data_version", [], |r| r.get(0))?;
        if after_version == pinned_version {
            self.cached = Some((generation, inputs.clone()));
            self.data_version = Some(pinned_version);
        } else {
            // The returned rows are one valid pinned snapshot, but an external
            // commit must not label those old rows with its newer connection version.
            self.cached = None;
            self.summaries.clear();
            self.data_version = None;
        }
        self.origin_fingerprint = Some(origins.fingerprint);
        Ok((Some(generation), qualified))
    }
    pub(crate) fn origin_fingerprint(&self) -> Option<&str> {
        self.origin_fingerprint.as_deref()
    }
    pub(crate) fn inputs_observed_at(&self) -> Option<time::OffsetDateTime> {
        self.inputs_observed_at
    }
    pub(crate) fn invalidate(&mut self) {
        // Revokes any outstanding invocation lease without a wrapping counter.
        self.query_owner = std::sync::Arc::new(());
        self.origins.clear();
        self.cached = None;
        self.store = None;
        self.identity = None;
        self.summaries.clear();
        self.data_version = None;
        self.inputs_observed_at = None;
        self.origin_fingerprint = None;
    }
    #[cfg(test)]
    pub(crate) fn test_cache_discarded(&self) -> bool {
        !self.origins.has_entry()
            && self.store.is_none()
            && self.identity.is_none()
            && self.cached.is_none()
            && self.summaries.is_empty()
            && self.data_version.is_none()
            && self.inputs_observed_at.is_none()
            && self.origin_fingerprint.is_none()
    }
}

fn qualify_origins(
    c: &rusqlite::Connection,
    origins: &super::migration::ActiveOriginReport,
    inputs: &mut DockStorageInputs,
) -> Result<(), DevMapError> {
    use rusqlite::OptionalExtension;
    let repository: String = c.query_row(
        "SELECT repository_id FROM store_meta WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let mut registry = BTreeMap::new();
    let replaced = origins.replaced_worktrees();
    let mut retained = Vec::new();
    for record in inputs.presence.records.drain(..) {
        let registration:Option<PresenceRegistration>=c.query_row(
            "SELECT s.worktree_id,s.incarnation,s.origin_path,r.git_dir,r.workspace_path,r.retired_at FROM journal_sessions s LEFT JOIN worktree_registry r ON r.worktree_id=s.worktree_id AND r.incarnation=s.incarnation WHERE s.session_id=?1",
            [&record.session_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
        let matched = if let Some((id, incarnation, origin, git, root, retired)) = registration {
            if id != record.worktree_id || git.as_deref() != Some(origin.as_str()) || root.is_none()
            {
                return Err(DevMapError::Store(
                    "presence session registration corrupt".into(),
                ));
            }
            let (historical, _) = super::origin_links::registered_origin(c, &id, &incarnation)?;
            retired.is_none()
                && origins
                    .current_origin(&id)
                    .is_some_and(|current| current.matches(&historical))
        } else {
            // Imported presence without a journal remains supported, but cannot
            // acquire a known replacement's physical identity by path alone.
            origins.current().contains_key(&record.worktree_id)
                && !replaced.contains(&record.worktree_id)
        };
        if matched {
            retained.push(record);
        } else {
            inputs.presence.warnings.push(presence::PresenceWarning {
                code: "presence_worktree_missing",
                subject_id: Some(record.session_id),
            });
        }
    }
    inputs.presence.records = retained;
    for observation in &origins.unavailable {
        inputs.presence.warnings.push(presence::PresenceWarning {
            code: observation.availability.warning_code(),
            subject_id: Some(observation.origin.worktree_id.clone()),
        });
    }
    if let Ok((plans, starts)) = &mut inputs.routes {
        let records = route_plan::sql_records(c, &repository)?;
        let links = super::origin_links::validate_routes(c, &records)?;
        let mut retained = Vec::new();
        for plan in plans.drain(..) {
            let link = links
                .get(&(plan.route_id.clone(), plan.revision))
                .ok_or_else(|| DevMapError::Store("route identity link missing".into()))?;
            // A missing workspace can retain historical intent. A currently
            // present path must match this revision's immutable incarnation.
            if matches!(
                link_availability(c, origins, link, &mut registry)?,
                LinkAvailability::Current | LinkAvailability::Missing
            ) {
                retained.push(plan);
                continue;
            }
            inputs.presence.warnings.push(presence::PresenceWarning {
                code: "planned_workspace_unavailable",
                subject_id: Some(plan.route_id.clone()),
            });
            starts.remove(&plan.route_id);
        }
        *plans = retained;
    }
    if let Ok(bindings) = &mut inputs.bindings {
        let snapshot = journal::sql_binding_snapshot(c, &repository)?;
        let metadata = super::origin_links::validate_bindings(c, &snapshot)?;
        for record in &mut bindings.records {
            let id = journal::binding_id(&record.observation)?;
            let links = metadata
                .links
                .get(&id)
                .ok_or_else(|| DevMapError::Store("binding identity link missing".into()))?;
            record.visible_worktrees.clear();
            for link in std::iter::once(&links.destination).chain(links.source.as_ref()) {
                if matches!(
                    link_availability(c, origins, link, &mut registry)?,
                    LinkAvailability::Current
                ) {
                    record.visible_worktrees.insert(link.worktree_id.clone());
                } else {
                    bindings
                        .incomplete_worktrees
                        .insert(link.worktree_id.clone());
                }
            }
        }
        for id in &bindings.incomplete_worktrees {
            inputs.presence.warnings.push(presence::PresenceWarning {
                code: "task_binding_history_unavailable",
                subject_id: Some(id.clone()),
            });
        }
    }
    Ok(())
}

enum LinkAvailability {
    Current,
    Missing,
    Unavailable,
}
type RegistryOrigins = BTreeMap<(String, String), (super::migration::FrozenOrigin, Option<String>)>;
fn link_availability(
    c: &rusqlite::Connection,
    origins: &super::migration::ActiveOriginReport,
    link: &super::origin_links::OriginLink,
    registry: &mut RegistryOrigins,
) -> Result<LinkAvailability, DevMapError> {
    let Some(incarnation) = &link.incarnation else {
        // Unknown identity may remain historical intent for a missing target;
        // it cannot become attached merely because that path ID is now live.
        return Ok(if origins.current().contains_key(&link.worktree_id) {
            LinkAvailability::Unavailable
        } else {
            LinkAvailability::Missing
        });
    };
    let key = (link.worktree_id.clone(), incarnation.clone());
    if !registry.contains_key(&key) {
        registry.insert(
            key.clone(),
            super::origin_links::registered_origin(c, &key.0, &key.1)?,
        );
    }
    let (registered, retired) = &registry[&key];
    let Some(current) = origins.current_origin(&link.worktree_id) else {
        return Ok(LinkAvailability::Missing);
    };
    if current.incarnation != *incarnation || retired.is_some() {
        return Ok(LinkAvailability::Unavailable);
    }
    if !current.matches(registered) {
        return Err(DevMapError::Store(
            "current origin registry paths mismatch".into(),
        ));
    }
    Ok(LinkAvailability::Current)
}
fn legacy(workspace: &SourceWorkspace) -> Result<DockStorageInputs, DevMapError> {
    let presence = PresenceStore::open_existing_legacy(workspace)?
        .map(|s| s.load_all_legacy())
        .unwrap_or(PresenceLoadReport {
            records: vec![],
            warnings: vec![],
            truncated: false,
        });
    let sessions = presence
        .records
        .iter()
        .map(|p| p.session_id.clone())
        .collect::<BTreeSet<_>>();
    Ok(DockStorageInputs {
        presence,
        journals: journal::summarize_legacy_sessions(workspace, &sessions),
        routes: route_plan::RoutePlanStore::open(workspace)
            .and_then(|s| s.legacy_snapshot())
            .map(|r| latest_routes(&r)),
        bindings: journal::legacy_binding_snapshot(workspace)
            .map(|s| DockBindingInputs::from(s.records)),
    })
}
fn latest_routes(records: &[route_plan::Record]) -> DockRoutePlans {
    let mut latest = BTreeMap::new();
    let mut starts = BTreeMap::new();
    for r in records {
        starts
            .entry(r.plan.route_id.clone())
            .or_insert_with(|| (r.plan.updated_at.clone(), r.plan.source.clone()));
        latest.insert(r.plan.route_id.clone(), r.plan.clone());
    }
    (latest.into_values().collect(), starts)
}
fn sql_inputs(
    c: &rusqlite::Connection,
    repository: &str,
    summaries: &mut SummaryCache,
) -> Result<DockStorageInputs, DevMapError> {
    let presence = presence::load_all_sql(c, repository);
    let sessions = presence
        .records
        .iter()
        .map(|p| p.session_id.as_str())
        .collect::<BTreeSet<_>>();
    summaries.retain(|id, _| sessions.contains(id.as_str()));
    let mut journals = BTreeMap::new();
    for p in &presence.records {
        let result = (|| -> Result<_, DevMapError> {
            let present = c.query_row(
                "SELECT count(*) FROM journal_sessions WHERE session_id=?1",
                [&p.session_id],
                |r| r.get::<_, i64>(0),
            )? == 1;
            let head = if present {
                Some(c.query_row("SELECT h.record_count,h.last_sha256,h.byte_length,s.worktree_id,s.incarnation,s.origin_path FROM journal_heads h JOIN journal_sessions s ON s.session_id=h.session_id WHERE h.session_id=?1", [&p.session_id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?)
            } else {
                None
            };
            if let Some(head) = &head
                && let Some((cached_head, summary)) = summaries.get(&p.session_id)
                && head == cached_head
            {
                return Ok(summary.clone());
            }
            let summary = journal::sql_summary(c, &p.session_id)?;
            if let Some(head) = head {
                summaries.insert(p.session_id.clone(), (head, summary.clone()));
            }
            Ok(summary)
        })();
        journals.insert(
            p.session_id.clone(),
            result.unwrap_or(JournalSummary {
                session_id: p.session_id.clone(),
                records: 0,
                last_sequence: None,
                last_sha256: None,
                integrity: JournalIntegrity::Corrupt,
            }),
        );
    }
    Ok(DockStorageInputs {
        presence,
        journals,
        routes: route_plan::sql_records(c, repository).map(|r| latest_routes(&r)),
        bindings: journal::sql_binding_snapshot(c, repository)
            .map(|s| DockBindingInputs::from(s.records)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit read-only component profiling of an owned scale fixture; not an acceptance benchmark"]
    fn profile_owned_scale_query_phases() {
        fn measure<T>(phase: &str, operation: impl FnOnce() -> T) -> T {
            eprintln!("{}", serde_json::json!({"phase":phase,"event":"start"}));
            let start = std::time::Instant::now();
            let result = operation();
            eprintln!(
                "{}",
                serde_json::json!({"phase":phase,"event":"end","milliseconds":start.elapsed().as_secs_f64()*1000.0})
            );
            result
        }
        let source = std::fs::canonicalize(
            std::env::var_os("DEVMAP_PROFILE_SOURCE").expect("explicit owned source"),
        )
        .unwrap();
        let allowed = std::fs::canonicalize(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/verification"),
        )
        .unwrap();
        assert!(source.starts_with(&allowed));
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(source.parent().unwrap().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["scope"], "synthetic_legacy_scale_fixture");
        assert_eq!(
            std::fs::canonicalize(manifest["source"].as_str().unwrap()).unwrap(),
            source
        );
        let workspace = measure("source_workspace", || {
            crate::git::SourceGitInspector::open(&source)
                .unwrap()
                .workspace()
                .unwrap()
        });
        let store = measure("readonly_store_open", || {
            RepositoryStore::open_existing(&workspace).unwrap().unwrap()
        });
        assert!(super::super::is_active(store.connection()).unwrap());
        let original_generation = store.generation().unwrap();
        let transaction = store.connection().unchecked_transaction().unwrap();
        measure("legacy_drift_full", || {
            super::super::migration::check_legacy_drift(&workspace, &transaction).unwrap()
        });
        let mut summaries = SummaryCache::new();
        let inputs = measure("sql_inputs_cold", || {
            sql_inputs(&transaction, &repository_id(&workspace), &mut summaries).unwrap()
        });
        assert_eq!(
            inputs.presence.records.len() as u64,
            manifest["sessions"].as_u64().unwrap()
        );
        assert!(
            inputs
                .journals
                .values()
                .all(|summary| summary.integrity == JournalIntegrity::Verified)
        );
        let warm = measure("sql_inputs_warm", || {
            sql_inputs(&transaction, &repository_id(&workspace), &mut summaries).unwrap()
        });
        assert_eq!(warm.journals, inputs.journals);
        transaction.commit().unwrap();
        let plans = &inputs.routes.as_ref().unwrap().0;
        let projection = measure("git_projection_collect", || {
            crate::dock::DockProjectionContext::collect(&workspace, plans).unwrap()
        });
        let model = measure("pure_projection", || {
            projection
                .project(inputs, time::OffsetDateTime::now_utc(), &[], None, false)
                .unwrap()
        });
        let bytes = measure("json_serialize", || serde_json::to_vec(&model).unwrap());
        assert_eq!(store.generation().unwrap(), original_generation);
        eprintln!(
            "{}",
            serde_json::json!({"scope":"readonly_scale_components","events":manifest["events"],"sessions":manifest["sessions"],"output_bytes":bytes.len(),"generation":original_generation,"debug_assertions":cfg!(debug_assertions)})
        );
    }
    #[test]
    fn external_same_generation_journal_tamper_invalidates_validated_summary() {
        use crate::events::{
            ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity,
            SessionContext,
        };
        let repo = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(repo.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial",
                    "--quiet"
                ])
                .status()
                .unwrap()
                .success()
        );
        let workspace = crate::git::SourceGitInspector::open(repo.path())
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let event = EventEnvelope::new(
            EVENT_SCHEMA_VERSION,
            "event",
            EventType::CaptureGap,
            1,
            "2026-09-08T10:00:00Z",
            HostIdentity::new("test", "1").unwrap(),
            ActorIdentity::new("a", None).unwrap(),
            SessionContext::new("session", None, "fixture", None, None, None).unwrap(),
            serde_json::json!({"reason":"fixture"}),
        )
        .unwrap();
        let record = journal::JournalStore::open(&workspace, "session")
            .unwrap()
            .append(event)
            .unwrap();
        PresenceStore::open(&workspace)
            .unwrap()
            .observe(
                presence::PresenceSignal::AcceptedRecords(&[record]),
                time::OffsetDateTime::now_utc(),
            )
            .unwrap();
        let backup = tempfile::tempdir().unwrap();
        super::super::migration::ensure(&workspace, &backup.path().join("frozen")).unwrap();
        let path = RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap()
            .path()
            .to_owned();
        let writer = rusqlite::Connection::open(path).unwrap();
        let mut reader = InputReader::new();
        let (generation, before) = reader.read(&workspace).unwrap();
        assert_eq!(
            before.journals["session"].integrity,
            JournalIntegrity::Verified
        );
        let (_, pinned) = reader
            .read_after_generation(&workspace, || {
                writer.execute(
                    "UPDATE journal_records SET record_json='{}' WHERE session_id='session'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            pinned.journals["session"].integrity,
            JournalIntegrity::Verified
        );
        let (after_generation, after) = reader.read(&workspace).unwrap();
        assert_eq!(generation, after_generation);
        assert_eq!(
            after.journals["session"].integrity,
            JournalIntegrity::Corrupt
        );
    }
    #[test]
    fn activation_during_legacy_load_retries_through_sql_selector() {
        let repo = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(repo.path())
                .status()
                .unwrap()
                .success()
        );
        let workspace = crate::git::SourceGitInspector::open(repo.path())
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let backup = tempfile::tempdir().unwrap();
        let mut reader = InputReader::new();
        let mut activated = false;
        let (generation, _) = reader
            .read_checked(&workspace, || {
                assert!(!activated);
                activated = true;
                super::super::migration::ensure(&workspace, &backup.path().join("frozen"))?;
                Ok(())
            })
            .unwrap();
        assert!(activated);
        assert_eq!(generation, Some(0));
    }
    #[test]
    fn generation_and_all_domains_remain_in_one_sql_snapshot() {
        let repo = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(repo.path())
                .status()
                .unwrap()
                .success()
        );
        let workspace = crate::git::SourceGitInspector::open(repo.path())
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let backup = tempfile::tempdir().unwrap();
        super::super::migration::ensure(&workspace, &backup.path().join("frozen")).unwrap();
        let path = RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap()
            .path()
            .to_owned();
        let writer = rusqlite::Connection::open(path).unwrap();
        let mut reader = InputReader::new();
        let (generation, before) = reader.read_after_generation(&workspace, || {
            writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO presence_records VALUES('bad','{}'); INSERT INTO route_records VALUES('bad',1,'bad','{}','{}'); INSERT INTO binding_records VALUES('bad','codex','bad','2026-09-08T00:00:00Z','{}'); UPDATE store_meta SET generation=1; COMMIT")?; Ok(())
        }).unwrap();
        assert_eq!(generation, Some(0));
        assert!(before.presence.warnings.is_empty());
        assert!(before.routes.is_ok());
        assert!(before.bindings.is_ok());
        let (generation, after) = reader.read(&workspace).unwrap();
        assert_eq!(generation, Some(1));
        assert_eq!(after.presence.warnings[0].code, "presence_record_invalid");
        assert!(after.routes.is_err());
        assert!(after.bindings.is_err());
        writer
            .execute("UPDATE store_meta SET schema_version=3", [])
            .unwrap();
        assert!(
            reader.read(&workspace).is_err(),
            "same generation must not bypass supported metadata validation"
        );
    }
}
