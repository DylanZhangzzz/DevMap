//! A generation-pinned, read-only input cache. No store creation or domain repair.
use super::RepositoryStore;
use crate::{
    dock::{DockRoutePlans, DockStorageInputs},
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

pub(crate) struct InputReader {
    store: Option<RepositoryStore>,
    identity: Option<crate::fs_security::FileIdentity>,
    cached: Option<(u64, DockStorageInputs)>,
    summaries: SummaryCache,
    data_version: Option<i64>,
    inputs_observed_at: Option<time::OffsetDateTime>,
}
impl InputReader {
    pub(crate) fn new() -> Self {
        Self {
            store: None,
            identity: None,
            cached: None,
            summaries: BTreeMap::new(),
            data_version: None,
            inputs_observed_at: None,
        }
    }
    pub(crate) fn read(
        &mut self,
        workspace: &SourceWorkspace,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        self.read_checked(workspace, || Ok(()))
    }
    fn read_checked(
        &mut self,
        workspace: &SourceWorkspace,
        mut after_legacy: impl FnMut() -> Result<(), DevMapError>,
    ) -> Result<(Option<u64>, DockStorageInputs), DevMapError> {
        for _ in 0..2 {
            let result = self.read_after_generation(workspace, || Ok(()))?;
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
    fn read_after_generation(
        &mut self,
        workspace: &SourceWorkspace,
        after: impl FnOnce() -> Result<(), DevMapError>,
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
        super::migration::check_legacy_drift(workspace, &tx)?;
        let inputs = match &self.cached {
            Some((cached_generation, inputs)) if *cached_generation == generation => inputs.clone(),
            _ => {
                let inputs = sql_inputs(&tx, &repository_id(workspace), &mut self.summaries)?;
                self.inputs_observed_at = Some(time::OffsetDateTime::now_utc());
                inputs
            }
        };
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
        Ok((Some(generation), inputs))
    }
    pub(crate) fn inputs_observed_at(&self) -> Option<time::OffsetDateTime> {
        self.inputs_observed_at
    }
    pub(crate) fn invalidate(&mut self) {
        self.cached = None;
        self.store = None;
        self.identity = None;
        self.summaries.clear();
        self.data_version = None;
        self.inputs_observed_at = None;
    }
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
        bindings: journal::legacy_binding_snapshot(workspace).map(|s| s.records),
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
            let records = journal::sql_records(c, &p.session_id)?;
            let summary = JournalSummary {
                session_id: p.session_id.clone(),
                records: records.len() as u64,
                last_sequence: records.last().map(|r| r.sequence),
                last_sha256: records.last().map(|r| r.sha256.clone()),
                integrity: if present {
                    JournalIntegrity::Verified
                } else {
                    JournalIntegrity::Missing
                },
            };
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
        bindings: journal::sql_binding_snapshot(c, repository).map(|s| s.records),
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
            .execute("UPDATE store_meta SET schema_version=2", [])
            .unwrap();
        assert!(
            reader.read(&workspace).is_err(),
            "same generation must not bypass supported metadata validation"
        );
    }
}
