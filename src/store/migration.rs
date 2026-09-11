//! Strict legacy snapshots and transactional cutover. Never repairs source files.
use super::{RepositoryStore, is_active};
use crate::{
    canonical::sha256_hex,
    error::DevMapError,
    fs_security as safe,
    git::{SourceGitInspector, SourceWorkspace},
    journal, presence, route_plan,
    worktrees::{self, WorktreeScanner},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_FILES: usize = 100_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const SNAPSHOT: &str = "@snapshot";
const ACTIVATION: &str = "@activation";
const FENCE: &str = "activation-intent.json";
#[path = "migration/inventory_parallel.rs"]
mod inventory_parallel;
#[path = "origin_observation.rs"]
mod origin_observation;
pub(crate) use origin_observation::ReadOriginCache;
#[cfg(test)]
pub(crate) use origin_observation::profile_frozen_read_stages;
#[path = "startup.rs"]
mod startup;
pub(crate) use origin_observation::{
    ActiveOriginReport, VerifiedCurrentOrigin, application_anchor, observe_active_journal_origins,
    observe_active_read_origins,
};
pub(crate) use startup::prepare_first_journal_write;
pub(crate) use startup::prepare_first_origin_write;
pub use startup::{WriteBackend, prepare_first_write};
fn fail(s: impl Into<String>) -> DevMapError {
    DevMapError::Store(s.into())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenFile {
    pub origin: usize,
    pub relative: String,
    pub sha256: String,
    pub bytes: u64,
    pub record_count: u64,
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenOrigin {
    pub git_dir: PathBuf,
    pub workspace_path: PathBuf,
    pub worktree_id: String,
    pub incarnation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenManifest {
    pub format: String,
    pub repository_id: String,
    pub common_dir: PathBuf,
    pub evaluated_at: String,
    pub origins: Vec<FrozenOrigin>,
    pub directories: BTreeSet<(usize, String)>,
    pub files: Vec<FrozenFile>,
    pub legacy_only: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    manifest: FrozenManifest,
    snapshot_path: PathBuf,
    snapshot_sha256: String,
    activated_at: String,
    generation: u64,
    dock_sha256: String,
}
#[derive(Debug, Serialize)]
pub struct SnapshotComparison {
    pub legacy: crate::dock::DockReadModel,
    pub sql: crate::dock::DockReadModel,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ActivationFence {
    format: String,
    repository_id: String,
    snapshot_sha256: String,
}

// Durable BEFORE the SQL selector commit. A crash before commit leaves an intact
// shadow and legacy authority. A missing DB after cutover cannot look never-migrated.
fn write_activation_fence(
    w: &SourceWorkspace,
    manifest: &FrozenManifest,
) -> Result<(), DevMapError> {
    let expected = ActivationFence {
        format: "devmap-activation-intent/1".into(),
        repository_id: worktrees::repository_id(w),
        snapshot_sha256: sha256_hex(&serde_json::to_vec(manifest)?),
    };
    let path = w.git_common_dir.join("devmap").join(FENCE);
    if safe::checked_metadata(&path)?.is_some() {
        let saved: ActivationFence = serde_json::from_slice(&read(&path)?)?;
        if saved != expected {
            return Err(fail("activation fence differs; explicit recovery required"));
        }
        return Ok(());
    }
    write_new(&path, &serde_json::to_vec(&expected)?)
}
pub(crate) fn refuse_missing_database(w: &SourceWorkspace) -> Result<(), DevMapError> {
    if safe::checked_metadata(&w.git_common_dir.join("devmap").join(FENCE))?.is_some() {
        return Err(fail(
            "database missing after activation intent; legacy downgrade refused, retained backups require explicit recovery",
        ));
    }
    Ok(())
}
#[derive(Debug, Serialize)]
pub struct StorageReport {
    pub backend: String,
    pub generation: u64,
    pub database: PathBuf,
    pub snapshot: Option<PathBuf>,
    pub verified: bool,
}
struct Parsed {
    routes: Vec<route_plan::Record>,
    bindings: journal::BindingSnapshot,
    journals: Vec<journal::FrozenJournalSnapshot>,
    presence: Vec<presence::PresenceRecord>,
}
type CapturedFiles = BTreeMap<(usize, String), Vec<u8>>;
struct CapturedSnapshot {
    manifest: FrozenManifest,
    files: CapturedFiles,
}

/// Read-only state inspection; unknown schema and corruption are errors, never recreation.
pub fn inspect(workspace: &SourceWorkspace) -> Result<StorageReport, DevMapError> {
    let store = RepositoryStore::open_existing(workspace)?;
    let (backend, generation, snapshot) = if let Some(s) = &store {
        let active = is_active(s.connection())?;
        let activation = activation(s.connection())?;
        (
            if active { "active" } else { "shadow" },
            s.generation()?,
            activation.map(|a| a.snapshot_path),
        )
    } else {
        ("legacy", 0, None)
    };
    Ok(StorageReport {
        backend: backend.into(),
        generation,
        database: workspace.git_common_dir.join("devmap/devmap.db"),
        snapshot,
        verified: false,
    })
}

/// Freeze to a new directory outside Git administration. Existing originals are read only.
pub fn freeze(
    workspace: &SourceWorkspace,
    destination: &Path,
    now: OffsetDateTime,
) -> Result<FrozenManifest, DevMapError> {
    let guard = super::transition::Guard::acquire(workspace)?;
    freeze_guarded(workspace, destination, now, &guard)
}
fn freeze_guarded(
    workspace: &SourceWorkspace,
    destination: &Path,
    now: OffsetDateTime,
    _guard: &super::transition::Guard,
) -> Result<FrozenManifest, DevMapError> {
    outside_admin(workspace, destination)?;
    if safe::checked_metadata(destination)?.is_some() {
        return Err(fail("snapshot destination already exists"));
    }
    let mut store = RepositoryStore::open_guarded(workspace, _guard)?;
    store.transaction(|tx| {
        if is_active(tx)? {
            return Err(fail("active store cannot be frozen for legacy reimport"));
        }
        let origins = origins(workspace)?;
        let _locks = legacy_locks(&origins)?;
        let mut manifest = inventory(
            workspace,
            origins,
            now.format(&Rfc3339).map_err(|e| fail(e.to_string()))?,
        )?;
        safe::ensure_directory(destination)?;
        let destination = safe::checked_canonical_directory(destination)?;
        let mut captured = CapturedFiles::new();
        for item in &manifest.files {
            let original = manifest.origins[item.origin]
                .git_dir
                .join("devmap")
                .join(&item.relative);
            let target = frozen_path(&destination, item);
            let relative_parent = Path::new(&item.relative).parent().unwrap();
            let base = safe::ensure_directory_chain(&destination, &[&item.origin.to_string()])?;
            let mut parent = base;
            for part in relative_parent.components() {
                parent.push(part);
                safe::ensure_directory(&parent)?;
            }
            let bytes = read(&original)?;
            if sha256_hex(&bytes) != item.sha256 {
                return Err(fail("legacy source drift during freeze"));
            }
            write_new(&target, &bytes)?;
            captured.insert((item.origin, item.relative.clone()), bytes);
        }
        // Empty origins must exist for strict sibling watermark readers.
        for index in 0..manifest.origins.len() {
            safe::ensure_directory_chain(&destination, &[&index.to_string()])?;
        }
        for (index, relative) in &manifest.directories {
            let mut path = destination.join(index.to_string());
            for part in Path::new(relative).components() {
                path.push(part);
                safe::ensure_directory(&path)?;
            }
        }
        let parsed = parse(&manifest, &captured)?;
        assign_counts(&mut manifest, &parsed);
        revalidate(workspace, &manifest)?;
        write_new(
            &destination.join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(manifest)
    })
}

/// Import only a validated frozen copy. Failure rolls the complete import back.
pub fn import_shadow(
    workspace: &SourceWorkspace,
    snapshot: &Path,
) -> Result<StorageReport, DevMapError> {
    let guard = super::transition::Guard::acquire(workspace)?;
    import_shadow_guarded(workspace, snapshot, &guard)
}
fn import_shadow_guarded(
    workspace: &SourceWorkspace,
    snapshot: &Path,
    _guard: &super::transition::Guard,
) -> Result<StorageReport, DevMapError> {
    let CapturedSnapshot { manifest, files } = load_snapshot(workspace, snapshot)?;
    let parsed = parse(&manifest, &files)?;
    validate_counts(&manifest, &parsed)?;
    let mut store = RepositoryStore::open_guarded(workspace, _guard)?;
    store.transaction(|tx| {
        if is_active(tx)? {
            check_legacy_drift(workspace, tx)?;
            let saved =
                activation(tx)?.ok_or_else(|| fail("active store has no migration provenance"))?;
            if saved.manifest != manifest {
                return Err(fail("active database cannot be reset or downgraded"));
            }
            return Ok(());
        }
        let _locks = legacy_locks(&manifest.origins)?;
        revalidate(workspace, &manifest)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT record_json FROM migration_sources WHERE source_path=?1",
                [SNAPSHOT],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if serde_json::from_str::<FrozenManifest>(&previous)? != manifest {
                return Err(fail(
                    "different shadow snapshot requires explicit recovery; original retained",
                ));
            }
            return compare_domains(tx, &manifest, &parsed);
        }
        if tx.query_row("SELECT generation FROM store_meta", [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(fail("nonempty store cannot be reset"));
        }
        import(tx, &manifest, &parsed)?;
        compare_domains(tx, &manifest, &parsed)?;
        physical_integrity(tx)?;
        revalidate(workspace, &manifest)?;
        tx.execute(
            "INSERT INTO migration_sources VALUES(?1,?2,?3,'validated_shadow',?4)",
            params![
                SNAPSHOT,
                sha256_hex(&serde_json::to_vec(&manifest)?),
                manifest.files.len() as i64,
                serde_json::to_string(&manifest)?
            ],
        )?;
        Ok(())
    })?;
    inspect(workspace)
}

/// Final lock, source revalidation, domain verification and durable selector change are atomic.
pub fn activate(
    workspace: &SourceWorkspace,
    snapshot: &Path,
) -> Result<StorageReport, DevMapError> {
    let guard = super::transition::Guard::acquire(workspace)?;
    activate_guarded(workspace, snapshot, &guard)
}
fn activate_guarded(
    workspace: &SourceWorkspace,
    snapshot: &Path,
    _guard: &super::transition::Guard,
) -> Result<StorageReport, DevMapError> {
    let CapturedSnapshot { manifest, files } = load_snapshot(workspace, snapshot)?;
    let parsed = parse(&manifest, &files)?;
    validate_counts(&manifest, &parsed)?;
    let context =
        crate::dock::DockProjectionContext::collect(workspace, &latest_routes(&parsed.routes).0)?;
    let mut store = RepositoryStore::open_guarded(workspace, _guard)?;
    store.transaction(|tx| {
        if is_active(tx)? {
            check_legacy_drift(workspace, tx)?;
            let saved =
                activation(tx)?.ok_or_else(|| fail("active store has no migration provenance"))?;
            if saved.manifest != manifest {
                return Err(fail("active database cannot be reset or downgraded"));
            }
            return Ok(());
        }
        let _locks = legacy_locks(&manifest.origins)?;
        revalidate(workspace, &manifest)?;
        compare_domains(tx, &manifest, &parsed)?;
        let comparison = compare_projection(tx, &manifest, &parsed, &context, &[], None, false)?;
        physical_integrity(tx)?;
        let saved: String = tx.query_row(
            "SELECT record_json FROM migration_sources WHERE source_path=?1",
            [SNAPSHOT],
            |r| r.get(0),
        )?;
        if serde_json::from_str::<FrozenManifest>(&saved)? != manifest {
            return Err(fail("shadow provenance mismatch"));
        }
        let record = Activation {
            manifest: manifest.clone(),
            snapshot_path: safe::checked_canonical_directory(snapshot)?,
            snapshot_sha256: sha256_hex(&serde_json::to_vec(&manifest)?),
            activated_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|e| fail(e.to_string()))?,
            generation: 0,
            dock_sha256: sha256_hex(&serde_json::to_vec(&comparison.sql)?),
        };
        revalidate(workspace, &manifest)?;
        write_activation_fence(workspace, &manifest)?;
        tx.execute(
            "INSERT INTO migration_sources VALUES(?1,?2,0,'active',?3)",
            params![
                ACTIVATION,
                record.snapshot_sha256,
                serde_json::to_string(&record)?
            ],
        )?;
        tx.execute(
            "UPDATE store_meta SET backend_state='active' WHERE singleton=1",
            [],
        )?;
        Ok(())
    })?;
    verify(workspace)
}

/// Export a real matched-time pair from frozen legacy inputs and one SQL snapshot.
/// Tasks are supplied observations used identically on both sides, never persisted here.
pub fn compare_snapshot(
    workspace: &SourceWorkspace,
    snapshot: &Path,
    tasks: &[crate::dock::ObservedTask],
) -> Result<SnapshotComparison, DevMapError> {
    compare_snapshot_with_inventory(workspace, snapshot, tasks, None, false)
}

/// Preserve supplied complete/partial host inventory metadata without freshening it.
pub fn compare_snapshot_with_inventory(
    workspace: &SourceWorkspace,
    snapshot: &Path,
    tasks: &[crate::dock::ObservedTask],
    inventory_observed_at: Option<String>,
    complete: bool,
) -> Result<SnapshotComparison, DevMapError> {
    let CapturedSnapshot { manifest, files } = load_snapshot(workspace, snapshot)?;
    let parsed = parse(&manifest, &files)?;
    validate_counts(&manifest, &parsed)?;
    let context =
        crate::dock::DockProjectionContext::collect(workspace, &latest_routes(&parsed.routes).0)?;
    let store = RepositoryStore::open_existing(workspace)?
        .ok_or_else(|| fail("missing shadow database"))?;
    store.connection().execute_batch("BEGIN")?;
    revalidate(workspace, &manifest)?;
    compare_domains(store.connection(), &manifest, &parsed)?;
    compare_projection(
        store.connection(),
        &manifest,
        &parsed,
        &context,
        tasks,
        inventory_observed_at,
        complete,
    )
}

type PlansAndStarts = (
    Vec<route_plan::RoutePlan>,
    BTreeMap<String, (String, String)>,
);
fn latest_routes(records: &[route_plan::Record]) -> PlansAndStarts {
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
fn summary(id: &str, records: &[journal::JournalRecord], present: bool) -> journal::JournalSummary {
    journal::JournalSummary {
        session_id: id.into(),
        records: records.len() as u64,
        last_sequence: records.last().map(|r| r.sequence),
        last_sha256: records.last().map(|r| r.sha256.clone()),
        integrity: if present {
            journal::JournalIntegrity::Verified
        } else {
            journal::JournalIntegrity::Missing
        },
    }
}
fn compare_projection(
    c: &Connection,
    m: &FrozenManifest,
    p: &Parsed,
    context: &crate::dock::DockProjectionContext,
    tasks: &[crate::dock::ObservedTask],
    inventory_observed_at: Option<String>,
    complete: bool,
) -> Result<SnapshotComparison, DevMapError> {
    let now = OffsetDateTime::parse(&m.evaluated_at, &Rfc3339).map_err(|e| fail(e.to_string()))?;
    let legacy_journals = p
        .presence
        .iter()
        .map(|r| {
            let source = p.journals.iter().find(|j| j.session_id == r.session_id);
            (
                r.session_id.clone(),
                summary(
                    &r.session_id,
                    source.map_or(&[], |j| j.records.as_slice()),
                    source.is_some_and(|j| j.journal_present),
                ),
            )
        })
        .collect();
    let legacy = context.project(
        crate::dock::DockStorageInputs {
            presence: presence::PresenceLoadReport {
                records: p.presence.clone(),
                warnings: vec![],
                truncated: false,
            },
            journals: legacy_journals,
            routes: Ok(latest_routes(&p.routes)),
            bindings: Ok(p.bindings.records.clone().into()),
        },
        now,
        tasks,
        inventory_observed_at.clone(),
        complete,
    )?;
    let mut records = Vec::new();
    let mut stmt =
        c.prepare("SELECT session_id,record_json FROM presence_records ORDER BY session_id")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, json) = row?;
        records.push(presence::parse_frozen_presence(
            &m.repository_id,
            &id,
            json.as_bytes(),
        )?);
    }
    let mut journals = BTreeMap::new();
    for p in &records {
        let present = c.query_row(
            "SELECT count(*) FROM journal_sessions WHERE session_id=?1",
            [&p.session_id],
            |r| r.get::<_, i64>(0),
        )? == 1;
        journals.insert(
            p.session_id.clone(),
            summary(
                &p.session_id,
                &journal::sql_records(c, &p.session_id)?,
                present,
            ),
        );
    }
    let sql = context.project(
        crate::dock::DockStorageInputs {
            presence: presence::PresenceLoadReport {
                records,
                warnings: vec![],
                truncated: false,
            },
            journals,
            routes: Ok(latest_routes(&route_plan::sql_records(
                c,
                &m.repository_id,
            )?)),
            bindings: Ok(journal::sql_binding_snapshot(c, &m.repository_id)?
                .records
                .into()),
        },
        now,
        tasks,
        inventory_observed_at,
        complete,
    )?;
    if serde_json::to_value(&legacy)? != serde_json::to_value(&sql)? {
        return Err(fail("matched-time full dock/4 projection mismatch"));
    }
    Ok(SnapshotComparison { legacy, sql })
}

/// Application setup seam, also used by the maintenance CLI. Callers decide how to surface
/// a preactivation error; once active, errors must preserve SQL and must never fall back.
pub fn ensure(
    workspace: &SourceWorkspace,
    backup_dir: &Path,
) -> Result<StorageReport, DevMapError> {
    let guard = super::transition::Guard::acquire(workspace)?;
    if RepositoryStore::open_existing(workspace)?
        .as_ref()
        .map(|s| is_active(s.connection()))
        .transpose()?
        .unwrap_or(false)
    {
        return verify(workspace);
    }
    if !backup_dir.join("manifest.json").exists() {
        freeze_guarded(workspace, backup_dir, OffsetDateTime::now_utc(), &guard)?;
    }
    import_shadow_guarded(workspace, backup_dir, &guard)?;
    activate_guarded(workspace, backup_dir, &guard)
}

pub fn verify(workspace: &SourceWorkspace) -> Result<StorageReport, DevMapError> {
    let store =
        RepositoryStore::open_existing(workspace)?.ok_or_else(|| fail("no repository database"))?;
    store.connection().execute_batch("BEGIN")?;
    physical_integrity(store.connection())?;
    check_legacy_drift(workspace, store.connection())?;
    validate_domains(store.connection(), &worktrees::repository_id(workspace))?;
    let mut report = inspect(workspace)?;
    report.verified = true;
    Ok(report)
}
pub fn backup(
    workspace: &SourceWorkspace,
    destination: &Path,
) -> Result<StorageReport, DevMapError> {
    outside_admin(workspace, destination)?;
    let store =
        RepositoryStore::open_existing(workspace)?.ok_or_else(|| fail("no repository database"))?;
    store.backup_to(destination)?;
    inspect(workspace)
}

pub(crate) fn check_legacy_drift(
    workspace: &SourceWorkspace,
    connection: &Connection,
) -> Result<(), DevMapError> {
    if let Some(record) = validated_activation(workspace, connection)? {
        revalidate(workspace, &record.manifest).map_err(|e| fail(format!("legacy source changed after activation; SQL and legacy retained, forward recovery required: {e}")))?;
    }
    Ok(())
}

fn validated_activation(
    workspace: &SourceWorkspace,
    connection: &Connection,
) -> Result<Option<Activation>, DevMapError> {
    let record = activation(connection)?;
    if let Some(record) = &record {
        let fence: ActivationFence =
            serde_json::from_slice(&read(&workspace.git_common_dir.join("devmap").join(FENCE))?)?;
        if fence.format != "devmap-activation-intent/1"
            || fence.repository_id != worktrees::repository_id(workspace)
            || fence.snapshot_sha256 != record.snapshot_sha256
        {
            return Err(fail("activation fence mismatch; SQL retained"));
        }
        if sha256_hex(&serde_json::to_vec(&record.manifest)?) != record.snapshot_sha256 {
            return Err(fail("activation provenance corrupt; SQL retained"));
        }
        validate_provenance(connection, &record.manifest)?;
    } else if is_active(connection)? {
        let remaining: i64 =
            connection.query_row("SELECT count(*) FROM migration_sources", [], |r| r.get(0))?;
        if remaining != 0
            || safe::checked_metadata(&workspace.git_common_dir.join("devmap").join(FENCE))?
                .is_some()
        {
            return Err(fail("activation provenance missing; SQL retained"));
        }
    }
    Ok(record)
}
fn activation(c: &Connection) -> Result<Option<Activation>, DevMapError> {
    let json: Option<String> = c
        .query_row(
            "SELECT record_json FROM migration_sources WHERE source_path=?1",
            [ACTIVATION],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}

fn outside_admin(w: &SourceWorkspace, target: &Path) -> Result<(), DevMapError> {
    let parent = target
        .parent()
        .ok_or_else(|| fail("destination needs existing parent"))?;
    let parent = safe::checked_canonical_directory(parent)?;
    let common = safe::checked_canonical_directory(&w.git_common_dir)?;
    let git = safe::checked_canonical_directory(&w.git_dir)?;
    if parent.starts_with(common) || parent.starts_with(git) {
        return Err(fail("backup must be outside Git administration"));
    }
    Ok(())
}
fn origins(w: &SourceWorkspace) -> Result<Vec<FrozenOrigin>, DevMapError> {
    let rows = WorktreeScanner::scan(w)?;
    let mut result = Vec::new();
    for row in rows {
        if !row.root.exists() {
            if row.git_dir.join("devmap").exists() {
                return Err(fail("retired legacy origin requires explicit recovery"));
            }
            continue;
        }
        safe::checked_canonical_directory(&row.root)?;
        // Match SourceGitInspector's Git-reported workspace spelling; physical identity
        // is independently captured below and Git administration remains canonical.
        let origin = SourceWorkspace {
            root: row.root,
            git_dir: safe::checked_canonical_directory(&row.git_dir)?,
            git_common_dir: w.git_common_dir.clone(),
            branch: row.branch,
            head: row.head,
        };
        result.push(FrozenOrigin {
            git_dir: origin.git_dir.clone(),
            workspace_path: origin.root.clone(),
            worktree_id: row.worktree_id,
            incarnation: journal::worktree_incarnation(&origin)?,
        });
    }
    result.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
    let administration = w.git_common_dir.join("worktrees");
    if safe::checked_metadata(&administration)?.is_some() {
        for entry in fs::read_dir(safe::checked_canonical_directory(&administration)?)? {
            let dir = entry?.path();
            if safe::checked_metadata(&dir.join("devmap"))?.is_some()
                && !result.iter().any(|r| r.git_dir == dir)
            {
                return Err(fail(
                    "unregistered historical legacy origin; explicit recovery required",
                ));
            }
        }
    }
    Ok(result)
}
fn operational_bookkeeping(name: &str) -> bool {
    matches!(
        name,
        "devmap.db"
            | "devmap.db-wal"
            | "devmap.db-shm"
            | "devmap.db-journal"
            | "store-init.lock"
            | FENCE
    )
}
fn inventory(
    w: &SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    evaluated_at: String,
) -> Result<FrozenManifest, DevMapError> {
    inventory_parallel::run(w, origins, evaluated_at)
}
fn inventory_serial(
    w: &SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    evaluated_at: String,
) -> Result<FrozenManifest, DevMapError> {
    inventory_collect(
        w,
        origins,
        evaluated_at,
        true,
        inventory_parallel::Limits::default(),
        #[cfg(test)]
        None,
    )
}
fn inventory_collect(
    w: &SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    evaluated_at: String,
    hash: bool,
    limits: inventory_parallel::Limits,
    #[cfg(test)] pipeline: Option<&inventory_parallel::PipelineHooks>,
) -> Result<FrozenManifest, DevMapError> {
    let mut manifest = inventory_collect_emitting(
        w,
        origins,
        evaluated_at,
        &mut InventoryWalk {
            hash,
            limits,
            #[cfg(test)]
            pipeline,
            sink: &mut |_, _| Ok(()),
        },
    )?;
    manifest
        .files
        .sort_by(|a, b| (a.origin, &a.relative).cmp(&(b.origin, &b.relative)));
    Ok(manifest)
}
struct InventoryWalk<'a> {
    hash: bool,
    limits: inventory_parallel::Limits,
    #[cfg(test)]
    pipeline: Option<&'a inventory_parallel::PipelineHooks>,
    sink: &'a mut dyn FnMut(usize, &FrozenFile) -> Result<(), DevMapError>,
}
// The same serial walker owns every classification and global reservation.
// Descriptors remain in discovery order until the sink's results are merged.
fn inventory_collect_emitting(
    w: &SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    evaluated_at: String,
    context: &mut InventoryWalk<'_>,
) -> Result<FrozenManifest, DevMapError> {
    let mut manifest = FrozenManifest { format:"devmap-frozen-legacy/1".into(), repository_id:worktrees::repository_id(w), common_dir:safe::checked_canonical_directory(&w.git_common_dir)?, evaluated_at, origins,
        directories:BTreeSet::new(), files:Vec::new(), legacy_only:vec!["Context Git repositories and their objects remain in their original locations; not imported or modified".into()] };
    let mut total = 0;
    for index in 0..manifest.origins.len() {
        let root = manifest.origins[index].git_dir.join("devmap");
        if safe::checked_metadata(&root)?.is_some() {
            walk(&root, &root, index, &mut manifest, &mut total, context)?;
        }
    }
    Ok(manifest)
}
fn walk(
    root: &Path,
    directory: &Path,
    origin: usize,
    manifest: &mut FrozenManifest,
    total: &mut u64,
    context: &mut InventoryWalk<'_>,
) -> Result<(), DevMapError> {
    safe::checked_canonical_directory(directory)?;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| fail("invalid inventory path"))?
            .to_str()
            .ok_or_else(|| fail("non UTF8 legacy path"))?
            .replace('\\', "/");
        if relative == super::transition::DIRECTORY {
            if manifest.origins[origin].git_dir != manifest.common_dir {
                return Err(fail("transition metadata outside common Git directory"));
            }
            super::transition::validate_directory(&path)?;
            continue;
        }
        if operational_bookkeeping(&relative) {
            if manifest.origins[origin].git_dir != manifest.common_dir {
                return Err(fail(
                    "SQL operational metadata outside common Git directory",
                ));
            }
            let metadata = safe::checked_metadata(&path)?
                .ok_or_else(|| fail("operational metadata disappeared"))?;
            if !metadata.is_file()
                || super::link_count(&safe::checked_file(&path, false, false)?)? != 1
            {
                return Err(fail("unsafe SQL operational metadata"));
            }
            continue;
        }
        let metadata =
            safe::checked_metadata(&path)?.ok_or_else(|| fail("legacy inventory drift"))?;
        if metadata.is_dir() {
            let components: Vec<_> = relative.split('/').collect();
            if !matches!(
                components.as_slice(),
                ["sessions"] | ["sessions", _] | ["presence"] | ["presence", "v1"]
            ) {
                return Err(fail(format!("unknown legacy directory: {relative}")));
            }
            if manifest.directories.len() >= context.limits.directories {
                return Err(fail("legacy directory inventory resource limit"));
            }
            manifest.directories.insert((origin, relative));
            walk(root, &path, origin, manifest, total, context)?;
        } else {
            if !metadata.is_file() {
                return Err(fail("unsupported legacy object"));
            }
            let kind = classify(
                &relative,
                manifest.origins[origin].git_dir == manifest.common_dir,
            )?;
            if manifest.files.len() >= context.limits.files
                || metadata.len() > context.limits.bytes
                || total
                    .checked_add(metadata.len())
                    .is_none_or(|sum| sum > context.limits.bytes)
            {
                return Err(fail("legacy inventory resource limit"));
            }
            let (bytes, sha256) = if context.hash {
                inventory_hash(&path, context.limits.bytes.saturating_sub(*total))?
            } else {
                (metadata.len(), String::new())
            };
            *total += bytes;
            manifest.files.push(FrozenFile {
                origin,
                relative,
                sha256,
                bytes,
                record_count: 0,
                outcome: kind.into(),
            });
            #[cfg(test)]
            if let Some(pipeline) = context.pipeline {
                pipeline.reserved(
                    manifest.files.len() - 1,
                    manifest.files.last().unwrap(),
                    *total,
                    &path,
                );
            }
            // Reservations are permanent for this attempt. Dispatch follows
            // every global check and precedes the post-submission observation.
            (context.sink)(manifest.files.len() - 1, manifest.files.last().unwrap())?;
            #[cfg(test)]
            if let Some(pipeline) = context.pipeline {
                pipeline.admitted(manifest.files.len())?;
            }
        }
    }
    Ok(())
}
fn classify(relative: &str, common: bool) -> Result<&'static str, DevMapError> {
    let parts: Vec<_> = relative.split('/').collect();
    match parts.as_slice() {
        ["route-plans.jsonl" | "task-bindings.jsonl" | "task-binding-watermarks.json"]
            if common =>
        {
            Ok("domain")
        }
        ["sessions", _, "events.ndjson"] => Ok("domain"),
        ["sessions", _, "events.lock" | "events.index"] => Ok("legacy_bookkeeping"),
        ["presence", "v1", name] if common && name.ends_with(".json") => Ok("domain"),
        ["presence", "v1", name] if common && name.ends_with(".lock") => Ok("legacy_bookkeeping"),
        _ => Err(fail(format!(
            "pending or unknown legacy artifact: {relative}"
        ))),
    }
}
fn read(path: &Path) -> Result<Vec<u8>, DevMapError> {
    safe::checked_canonical_directory(path.parent().ok_or_else(|| fail("file has no parent"))?)?;
    let file = safe::checked_file(path, false, false)?;
    if super::link_count(&file)? != 1 {
        return Err(fail("hard-linked legacy artifact refused"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(fail("legacy file resource limit"));
    }
    Ok(bytes)
}
// One checked-open implementation for normal and instrumented full hashes.
fn inventory_checked_file(path: &Path) -> Result<fs::File, DevMapError> {
    safe::checked_canonical_directory(path.parent().ok_or_else(|| fail("file has no parent"))?)?;
    let file = safe::checked_file(path, false, false)?;
    if super::link_count(&file)? != 1 {
        return Err(fail("hard-linked legacy artifact refused"));
    }
    Ok(file)
}
fn inventory_hash(path: &Path, remaining: u64) -> Result<(u64, String), DevMapError> {
    inventory_hash_reader(inventory_checked_file(path)?, remaining)
}

trait InventoryReadObserver {
    fn returned(&mut self, _bytes: usize) {}
    fn accepted(&mut self, _bytes: usize) {}
    fn probe(&mut self, _bytes: usize) {}
    fn eof(&mut self) {}
}
struct NoopInventoryReadObserver;
impl InventoryReadObserver for NoopInventoryReadObserver {}

fn inventory_hash_reader(reader: impl Read, remaining: u64) -> Result<(u64, String), DevMapError> {
    inventory_hash_reader_observed(reader, remaining, NoopInventoryReadObserver)
}

#[cfg(test)]
fn inventory_hash_observed(
    path: &Path,
    remaining: u64,
    observer: impl InventoryReadObserver,
    opened: impl FnOnce(),
) -> Result<(u64, String), DevMapError> {
    let file = inventory_checked_file(path)?;
    opened(); // Successful checked payload open, not temporary identity handles.
    inventory_hash_reader_observed(file, remaining, observer)
}

fn inventory_hash_reader_observed(
    mut reader: impl Read,
    remaining: u64,
    mut observer: impl InventoryReadObserver,
) -> Result<(u64, String), DevMapError> {
    use sha2::{Digest, Sha256};
    let limit = remaining.min(MAX_BYTES);
    let mut total = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        // At the limit, one byte distinguishes EOF from growth/overflow. The
        // probe is never hashed or admitted. Subtraction is safe by invariant.
        let length = usize::try_from((limit - total).min(buffer.len() as u64))
            .unwrap()
            .max(1);
        let count = match reader.read(&mut buffer[..length]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        observer.returned(count);
        if count == 0 {
            observer.eof();
            return Ok((total, format!("{:x}", hash.finalize())));
        }
        if count as u64 > limit - total {
            observer.probe(count);
            return Err(fail(if remaining >= MAX_BYTES {
                "legacy file resource limit"
            } else {
                "legacy inventory resource limit"
            }));
        }
        total += count as u64;
        observer.accepted(count);
        hash.update(&buffer[..count]);
    }
}

#[cfg(test)]
mod inventory_hash_tests {
    use super::*;

    #[test]
    fn interrupted_short_reads_and_errors_preserve_read_semantics() {
        struct ShortReader {
            bytes: std::io::Cursor<Vec<u8>>,
            interrupted: bool,
            fail_at_end: bool,
        }
        impl Read for ShortReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                if self.fail_at_end && self.bytes.position() == self.bytes.get_ref().len() as u64 {
                    return Err(std::io::Error::other("owned reader failure"));
                }
                let length = buffer.len().min(3);
                self.bytes.read(&mut buffer[..length])
            }
        }
        let bytes = b"short-read-complete-bytes".to_vec();
        let make = |fail_at_end| ShortReader {
            bytes: std::io::Cursor::new(bytes.clone()),
            interrupted: false,
            fail_at_end,
        };
        assert_eq!(
            inventory_hash_reader(make(false), bytes.len() as u64).unwrap(),
            (bytes.len() as u64, sha256_hex(&bytes))
        );
        let error = inventory_hash_reader(make(true), bytes.len() as u64).unwrap_err();
        assert!(error.to_string().contains("owned reader failure"));
    }

    #[test]
    fn actual_bytes_exceeding_remaining_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.ndjson");
        fs::write(&path, b"12345678").unwrap();
        // A stale metadata precheck can have admitted fewer bytes than now read.
        // Exercise the remaining-budget boundary directly without a timing race.
        assert!(inventory_hash(&path, 7).is_err());
        assert!(inventory_hash(&path, 0).is_err());
    }

    #[test]
    fn growth_after_metadata_admission_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.ndjson");
        fs::write(&path, b"1234").unwrap();
        let admitted = fs::metadata(&path).unwrap().len();
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"5678").unwrap();
        drop(writer);
        // Deliberate sequential growth after metadata, not a concurrency claim.
        assert!(inventory_hash(&path, admitted).is_err());
    }

    #[test]
    fn exact_empty_and_multichunk_hashes_match_original() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.ndjson");
        for bytes in [
            Vec::new(),
            b"12345678".to_vec(),
            "完整字节🙂\n".repeat(20000).into_bytes(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert_eq!(
                inventory_hash(&path, bytes.len() as u64).unwrap(),
                (bytes.len() as u64, sha256_hex(&bytes))
            );
        }
    }

    #[test]
    fn hardlinked_and_non_file_inputs_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.ndjson");
        fs::write(&path, b"content").unwrap();
        fs::hard_link(&path, directory.path().join("alias")).unwrap();
        assert!(inventory_hash(&path, MAX_BYTES).is_err());
        assert!(inventory_hash(directory.path(), MAX_BYTES).is_err());
        assert!(inventory_hash(&directory.path().join("missing"), MAX_BYTES).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_file_and_parent_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        let file = real.join("events.ndjson");
        fs::write(&file, b"bytes").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&file, &alias).unwrap();
        assert!(inventory_hash(&alias, MAX_BYTES).is_err());
        let parent = directory.path().join("parent");
        std::os::unix::fs::symlink(&real, &parent).unwrap();
        assert!(inventory_hash(&parent.join("events.ndjson"), MAX_BYTES).is_err());
    }
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), DevMapError> {
    let mut file = safe::checked_new_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    safe::sync_directory(path.parent().unwrap())?;
    Ok(())
}
fn frozen_path(root: &Path, file: &FrozenFile) -> PathBuf {
    root.join(file.origin.to_string()).join(&file.relative)
}
fn legacy_locks(origins: &[FrozenOrigin]) -> Result<Vec<File>, DevMapError> {
    let mut paths = BTreeSet::new();
    for origin in origins {
        let root = origin.git_dir.join("devmap");
        for name in ["route-plans.jsonl", "task-bindings.jsonl"] {
            let p = root.join(name);
            if safe::checked_metadata(&p)?.is_some() {
                paths.insert(p);
            }
        }
        let sessions = root.join("sessions");
        if safe::checked_metadata(&sessions)?.is_some() {
            for e in fs::read_dir(safe::checked_canonical_directory(&sessions)?)? {
                let session = e?.path();
                safe::checked_canonical_directory(&session)?;
                let p = session.join("events.lock");
                if safe::checked_metadata(&p)?.is_some() {
                    paths.insert(p);
                }
            }
        }
        let presence = root.join("presence/v1");
        if safe::checked_metadata(&presence)?.is_some() {
            for e in fs::read_dir(safe::checked_canonical_directory(&presence)?)? {
                let p = e?.path();
                if p.extension().is_some_and(|e| e == "lock") {
                    paths.insert(p);
                }
            }
        }
    }
    let mut locks = Vec::new();
    for path in paths {
        let file = safe::checked_file(&path, false, false)?;
        if super::link_count(&file)? != 1 {
            return Err(fail("hard-linked legacy lock refused"));
        }
        // Shared freezes exclude every cooperating exclusive writer while permitting
        // separately opened read handles on Windows (exclusive byte locks do not).
        let start = std::time::Instant::now();
        loop {
            match fs2::FileExt::try_lock_shared(&file) {
                Ok(()) => break,
                Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                    if start.elapsed() >= super::TIMEOUT {
                        return Err(fail("legacy freeze lock timeout"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e.into()),
            }
        }
        locks.push(file);
    }
    Ok(locks)
}
fn revalidate(w: &SourceWorkspace, manifest: &FrozenManifest) -> Result<(), DevMapError> {
    let current_origins = origins(w)?;
    // New empty worktrees are harmless, but every original physical origin must still match.
    for old in &manifest.origins {
        if !current_origins.contains(old) {
            return Err(fail("legacy origin identity drift"));
        }
    }
    let current = inventory(w, current_origins, manifest.evaluated_at.clone())?;
    validate_inventory_equality(&current, manifest)
}

fn validate_inventory_equality(
    current: &FrozenManifest,
    manifest: &FrozenManifest,
) -> Result<(), DevMapError> {
    let flatten = |m: &FrozenManifest| {
        m.files
            .iter()
            .map(|f| {
                (
                    m.origins[f.origin].git_dir.clone(),
                    f.relative.clone(),
                    f.sha256.clone(),
                    f.bytes,
                )
            })
            .collect::<BTreeSet<_>>()
    };
    let dirs = |m: &FrozenManifest| {
        m.directories
            .iter()
            .map(|(i, p)| (m.origins[*i].git_dir.clone(), p.clone()))
            .collect::<BTreeSet<_>>()
    };
    if current.repository_id != manifest.repository_id
        || current.common_dir != manifest.common_dir
        || flatten(current) != flatten(manifest)
        || dirs(current) != dirs(manifest)
    {
        return Err(fail("legacy source inventory/hash drift"));
    }
    Ok(())
}
fn load_snapshot(w: &SourceWorkspace, root: &Path) -> Result<CapturedSnapshot, DevMapError> {
    outside_admin(w, root)?;
    let manifest: FrozenManifest = serde_json::from_slice(&read(&root.join("manifest.json"))?)?;
    if manifest.format != "devmap-frozen-legacy/1"
        || manifest.repository_id != worktrees::repository_id(w)
        || manifest.common_dir != safe::checked_canonical_directory(&w.git_common_dir)?
    {
        return Err(fail(
            "frozen snapshot original repository identity mismatch",
        ));
    }
    if manifest.files.len() > MAX_FILES
        || manifest.directories.len() > MAX_FILES
        || manifest.origins.len() > 256
    {
        return Err(fail("frozen snapshot inventory resource limit"));
    }
    let mut captured = CapturedFiles::new();
    let mut total_bytes = 0u64;
    let mut seen = BTreeSet::new();
    for f in &manifest.files {
        if f.origin >= manifest.origins.len()
            || !seen.insert((f.origin, f.relative.clone()))
            || Path::new(&f.relative)
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
        {
            return Err(fail("invalid frozen file path"));
        }
        classify(
            &f.relative,
            manifest.origins[f.origin].git_dir == manifest.common_dir,
        )?;
        if f.bytes > MAX_BYTES.saturating_sub(total_bytes) {
            return Err(fail("frozen snapshot aggregate byte limit"));
        }
        let bytes = read(&frozen_path(root, f))?;
        if bytes.len() as u64 != f.bytes || sha256_hex(&bytes) != f.sha256 {
            return Err(fail("frozen snapshot hash mismatch"));
        }
        total_bytes += bytes.len() as u64;
        captured.insert((f.origin, f.relative.clone()), bytes);
    }
    let mut expected_files = manifest
        .files
        .iter()
        .map(|f| format!("{}/{}", f.origin, f.relative))
        .collect::<BTreeSet<_>>();
    expected_files.insert("manifest.json".into());
    let mut expected_dirs = manifest
        .directories
        .iter()
        .map(|(i, p)| format!("{i}/{p}"))
        .collect::<BTreeSet<_>>();
    for i in 0..manifest.origins.len() {
        expected_dirs.insert(i.to_string());
    }
    let (mut actual_files, mut actual_dirs) = (BTreeSet::new(), BTreeSet::new());
    snapshot_inventory(root, root, &mut actual_files, &mut actual_dirs)?;
    if expected_files != actual_files || expected_dirs != actual_dirs {
        return Err(fail(
            "frozen snapshot contains missing or unlisted artifacts",
        ));
    }
    Ok(CapturedSnapshot {
        manifest,
        files: captured,
    })
}
fn snapshot_inventory(
    root: &Path,
    dir: &Path,
    files: &mut BTreeSet<String>,
    dirs: &mut BTreeSet<String>,
) -> Result<(), DevMapError> {
    safe::checked_canonical_directory(dir)?;
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        let relative = p
            .strip_prefix(root)
            .map_err(|_| fail("snapshot path escape"))?
            .to_str()
            .ok_or_else(|| fail("snapshot non UTF8 path"))?
            .replace('\\', "/");
        let meta = safe::checked_metadata(&p)?.ok_or_else(|| fail("snapshot inventory drift"))?;
        if files.len() + dirs.len() >= MAX_FILES * 2 {
            return Err(fail("snapshot inventory resource limit"));
        }
        if meta.is_dir() {
            dirs.insert(relative);
            snapshot_inventory(root, &p, files, dirs)?;
        } else if meta.is_file() {
            files.insert(relative);
        } else {
            return Err(fail("unsupported frozen artifact"));
        }
    }
    Ok(())
}
fn validate_counts(m: &FrozenManifest, p: &Parsed) -> Result<(), DevMapError> {
    let mut expected = m.clone();
    assign_counts(&mut expected, p);
    if expected.files != m.files {
        return Err(fail("frozen record counts mismatch"));
    }
    Ok(())
}
fn parse(m: &FrozenManifest, captured: &CapturedFiles) -> Result<Parsed, DevMapError> {
    let common = m
        .origins
        .iter()
        .position(|o| o.git_dir == m.common_dir)
        .ok_or_else(|| fail("common origin missing"))?;
    let bytes = |origin: usize, relative: &str| {
        captured
            .get(&(origin, relative.to_owned()))
            .map(Vec::as_slice)
    };
    let routes = route_plan::parse_frozen_routes(
        bytes(common, "route-plans.jsonl").unwrap_or_default(),
        &m.repository_id,
    )?;
    let bindings = journal::parse_frozen_bindings(
        bytes(common, "task-bindings.jsonl").unwrap_or_default(),
        bytes(common, "task-binding-watermarks.json"),
        &m.repository_id,
    )?;
    let mut sessions: BTreeMap<(usize, String), BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    for (index, dir) in &m.directories {
        if let Some(id) = dir.strip_prefix("sessions/") {
            sessions.entry((*index, id.into())).or_default();
        }
    }
    let mut presence = Vec::new();
    for f in &m.files {
        let captured_bytes =
            bytes(f.origin, &f.relative).ok_or_else(|| fail("verified frozen bytes missing"))?;
        let parts: Vec<_> = f.relative.split('/').collect();
        if let ["sessions", id, name] = parts.as_slice() {
            sessions
                .entry((f.origin, (*id).into()))
                .or_default()
                .insert((*name).into(), captured_bytes.to_vec());
        }
        if let ["presence", "v1", name] = parts.as_slice()
            && let Some(id) = name.strip_suffix(".json")
        {
            presence.push(presence::parse_frozen_presence(
                &m.repository_id,
                id,
                captured_bytes,
            )?);
        }
    }
    let sources = sessions
        .into_iter()
        .map(|((i, id), files)| journal::FrozenJournalSource {
            session_id: id,
            origin_path: m.origins[i].git_dir.clone(),
            files,
        })
        .collect::<Vec<_>>();
    let journals = journal::parse_frozen_journal_sources(&sources)?;
    presence.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    if presence.len() > presence::MAX_PRESENCE_RECORDS {
        return Err(fail(
            "legacy presence inventory is partial; safe migration requires complete projection inputs",
        ));
    }
    Ok(Parsed {
        routes,
        bindings,
        journals,
        presence,
    })
}
fn assign_counts(m: &mut FrozenManifest, p: &Parsed) {
    for f in &mut m.files {
        f.record_count = match f.relative.as_str() {
            "route-plans.jsonl" => p.routes.len() as u64,
            "task-bindings.jsonl" => p.bindings.records.len() as u64,
            "task-binding-watermarks.json" => p.bindings.watermarks.len() as u64,
            s if s.starts_with("presence/") && s.ends_with(".json") => 1,
            s if s.ends_with("/events.ndjson") => p
                .journals
                .iter()
                .find(|j| {
                    s == format!("sessions/{}/events.ndjson", j.session_id)
                        && m.origins[f.origin].git_dir == j.origin_path
                })
                .map_or(0, |j| j.records.len() as u64),
            _ => 0,
        };
    }
}

fn import(tx: &Transaction<'_>, m: &FrozenManifest, p: &Parsed) -> Result<(), DevMapError> {
    for o in &m.origins {
        tx.execute("INSERT INTO worktree_registry(worktree_id,incarnation,git_dir,workspace_path) VALUES(?1,?2,?3,?4)",params![o.worktree_id,o.incarnation,o.git_dir.to_string_lossy(),o.workspace_path.to_string_lossy()])?;
    }
    for r in &p.routes {
        tx.execute(
            "INSERT INTO route_records VALUES(?1,?2,?3,?4,?5)",
            params![
                r.plan.route_id,
                r.plan.revision as i64,
                r.input.request_id,
                serde_json::to_string(&r.input)?,
                serde_json::to_string(&r.plan)?
            ],
        )?;
        super::origin_links::insert_route(
            tx,
            &r.plan,
            &super::origin_links::frozen_link(&r.plan.worktree_id, &m.origins),
        )?;
    }
    let binding_metadata = super::origin_links::frozen_bindings(&p.bindings, &m.origins)?;
    for b in &p.bindings.records {
        tx.execute(
            "INSERT INTO binding_records VALUES(?1,?2,?3,?4,?5)",
            params![
                journal::binding_id(b)?,
                b.host,
                b.task_id,
                b.observed_at,
                serde_json::to_string(b)?
            ],
        )?;
        let identity = journal::binding_id(b)?;
        let link = binding_metadata
            .links
            .get(&identity)
            .ok_or_else(|| fail("frozen binding identity link missing"))?;
        super::origin_links::insert_binding(tx, b, link)?;
    }
    for ((host, task), time) in &p.bindings.watermarks {
        tx.execute(
            "INSERT INTO binding_watermarks VALUES(?1,?2,?3)",
            params![
                serde_json::to_string(&(host, task))?,
                time,
                serde_json::to_string(&(host, task, time))?
            ],
        )?;
        let cursor = binding_metadata
            .cursors
            .get(&(host.clone(), task.clone()))
            .ok_or_else(|| fail("frozen binding cursor missing"))?;
        super::origin_links::upsert_cursor(tx, host, task, cursor)?;
    }
    for j in &p.journals {
        if !j.journal_present {
            continue;
        }
        let o = m
            .origins
            .iter()
            .find(|o| o.git_dir == j.origin_path)
            .ok_or_else(|| fail("unknown frozen origin"))?;
        tx.execute(
            "INSERT INTO journal_sessions VALUES(?1,?2,?3,?4)",
            params![
                j.session_id,
                o.worktree_id,
                o.incarnation,
                o.git_dir.to_string_lossy()
            ],
        )?;
        let mut total = 0;
        for r in &j.records {
            let bytes = crate::canonical::canonical_json(r)?;
            total += bytes.len() + 1;
            let json = String::from_utf8(bytes).map_err(|_| fail("invalid record UTF8"))?;
            tx.execute(
                "INSERT INTO journal_records VALUES(?1,?2,?3,?4,?5)",
                params![
                    j.session_id,
                    r.sequence as i64,
                    r.event.event_id(),
                    json,
                    json.len() as i64
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO journal_heads VALUES(?1,?2,?3,?4)",
            params![
                j.session_id,
                j.records.len() as i64,
                j.records.last().map(|r| &r.sha256),
                total as i64
            ],
        )?;
    }
    for p in &p.presence {
        let json = String::from_utf8(crate::canonical::canonical_json(p)?)
            .map_err(|_| fail("invalid presence UTF8"))?;
        tx.execute(
            "INSERT INTO presence_records VALUES(?1,?2)",
            params![p.session_id, json],
        )?;
        tx.execute("INSERT INTO presence_projection SELECT session_id,record_count,last_sha256,'legacy_import' FROM journal_heads WHERE session_id=?1",[&p.session_id])?;
    }
    for f in &m.files {
        tx.execute(
            "INSERT INTO migration_sources VALUES(?1,?2,?3,?4,?5)",
            params![
                m.origins[f.origin]
                    .git_dir
                    .join("devmap")
                    .join(&f.relative)
                    .to_string_lossy(),
                f.sha256,
                f.record_count as i64,
                f.outcome,
                serde_json::to_string(f)?
            ],
        )?;
    }
    Ok(())
}
fn compare_domains(c: &Connection, m: &FrozenManifest, p: &Parsed) -> Result<(), DevMapError> {
    validate_provenance(c, m)?;
    // This is exact frozen-snapshot comparison, not active-store verification.
    // Preserve the existing full-manifest registry baseline and reject extras;
    // native additions are validated separately without relabeling their links.
    let registry_count: i64 =
        c.query_row("SELECT count(*) FROM worktree_registry", [], |r| r.get(0))?;
    if registry_count != m.origins.len() as i64 {
        return Err(fail("imported worktree registry inventory mismatch"));
    }
    for o in &m.origins {
        let saved:(String,String,Option<String>)=c.query_row("SELECT git_dir,workspace_path,retired_at FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",params![o.worktree_id,o.incarnation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if saved
            != (
                o.git_dir.to_string_lossy().into_owned(),
                o.workspace_path.to_string_lossy().into_owned(),
                None,
            )
        {
            return Err(fail("imported worktree registry mismatch"));
        }
    }
    if serde_json::to_value(route_plan::sql_records(c, &m.repository_id)?)?
        != serde_json::to_value(&p.routes)?
    {
        return Err(fail("route input/revision comparison failed"));
    }
    let expected_route_links = p
        .routes
        .iter()
        .map(|record| {
            (
                (record.plan.route_id.clone(), record.plan.revision),
                super::origin_links::frozen_link(&record.plan.worktree_id, &m.origins),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if super::origin_links::validate_routes(c, &p.routes)? != expected_route_links {
        return Err(fail("imported route identity links mismatch"));
    }
    let bindings = journal::sql_binding_snapshot(c, &m.repository_id)?;
    if bindings.records != p.bindings.records || bindings.watermarks != p.bindings.watermarks {
        return Err(fail("binding history/watermark comparison failed"));
    }
    if super::origin_links::validate_bindings(c, &bindings)?
        != super::origin_links::frozen_bindings(&p.bindings, &m.origins)?
    {
        return Err(fail("imported binding identity links/cursors mismatch"));
    }
    let expected = p.journals.iter().filter(|j| j.journal_present).count() as i64;
    if c.query_row("SELECT count(*) FROM journal_sessions", [], |r| {
        r.get::<_, i64>(0)
    })? != expected
    {
        return Err(fail("journal session inventory mismatch"));
    }
    for j in &p.journals {
        let registration: Option<(String,String,String)> = c.query_row(
            "SELECT worktree_id,incarnation,origin_path FROM journal_sessions WHERE session_id=?1",
            [&j.session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?;
        let expected_registration = if j.journal_present {
            let origin = m
                .origins
                .iter()
                .find(|o| o.git_dir == j.origin_path)
                .ok_or_else(|| fail("unknown frozen journal origin"))?;
            Some((
                origin.worktree_id.clone(),
                origin.incarnation.clone(),
                origin.git_dir.to_string_lossy().into_owned(),
            ))
        } else {
            None
        };
        if registration != expected_registration {
            return Err(fail(
                "journal session registration differs from frozen origin",
            ));
        }
        if journal::sql_records(c, &j.session_id)? != j.records {
            return Err(fail("journal record/hash comparison failed"));
        }
    }
    let mut loaded = Vec::new();
    let mut stmt =
        c.prepare("SELECT session_id,record_json FROM presence_records ORDER BY session_id")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, json) = row?;
        loaded.push(presence::parse_frozen_presence(
            &m.repository_id,
            &id,
            json.as_bytes(),
        )?);
    }
    if loaded != p.presence {
        return Err(fail("explicit presence comparison failed"));
    }
    let expected_projection = p
        .presence
        .iter()
        .filter_map(|r| {
            p.journals
                .iter()
                .find(|j| j.session_id == r.session_id && j.journal_present)
        })
        .map(|j| {
            (
                j.session_id.clone(),
                j.records.len() as i64,
                j.records.last().map(|r| r.sha256.clone()),
                "legacy_import".to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut stmt=c.prepare("SELECT session_id,covered_sequence,covered_sha256,baseline_source FROM presence_projection")?;
    let actual = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual != expected_projection {
        return Err(fail("imported presence projection cursor mismatch"));
    }
    Ok(())
}
fn validate_provenance(c: &Connection, m: &FrozenManifest) -> Result<(), DevMapError> {
    for f in &m.files {
        let path = m.origins[f.origin].git_dir.join("devmap").join(&f.relative);
        let saved:(String,i64,String,String)=c.query_row("SELECT source_hash,record_count,outcome,record_json FROM migration_sources WHERE source_path=?1",[path.to_string_lossy()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if saved.0 != f.sha256
            || saved.1 != f.record_count as i64
            || saved.2 != f.outcome
            || serde_json::from_str::<FrozenFile>(&saved.3)? != *f
        {
            return Err(fail("migration source provenance mismatch"));
        }
    }
    if c.query_row(
        "SELECT count(*) FROM migration_sources WHERE source_path NOT IN (?1,?2)",
        params![SNAPSHOT, ACTIVATION],
        |r| r.get::<_, i64>(0),
    )? != m.files.len() as i64
    {
        return Err(fail("migration source provenance inventory mismatch"));
    }
    Ok(())
}
fn validate_domains(c: &Connection, repository: &str) -> Result<(), DevMapError> {
    let routes = route_plan::sql_records(c, repository)?;
    super::origin_links::validate_routes(c, &routes)?;
    let bindings = journal::sql_binding_snapshot(c, repository)?;
    super::origin_links::validate_bindings(c, &bindings)?;
    let mut stmt = c.prepare("SELECT session_id FROM journal_sessions")?;
    for id in stmt.query_map([], |r| r.get::<_, String>(0))? {
        journal::sql_records(c, &id?)?;
    }
    let mut stmt = c.prepare("SELECT session_id,record_json FROM presence_records")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, json) = row?;
        presence::parse_frozen_presence(repository, &id, json.as_bytes())?;
    }
    let mut stmt=c.prepare("SELECT p.session_id,p.covered_sequence,p.covered_sha256,r.record_json FROM presence_projection p LEFT JOIN presence_records r ON p.session_id=r.session_id")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })? {
        let (id, sequence, hash, presence) = row?;
        if presence.is_none() || sequence < 0 {
            return Err(fail("invalid presence projection baseline"));
        }
        let expected = if sequence == 0 {
            None
        } else {
            let json: String = c.query_row(
                "SELECT record_json FROM journal_records WHERE session_id=?1 AND sequence=?2",
                params![id, sequence],
                |r| r.get(0),
            )?;
            Some(serde_json::from_str::<journal::JournalRecord>(&json)?.sha256)
        };
        if hash != expected {
            return Err(fail("presence projection hash mismatch"));
        }
    }
    Ok(())
}
fn physical_integrity(c: &Connection) -> Result<(), DevMapError> {
    let mut stmt = c.prepare("PRAGMA integrity_check")?;
    for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
        if r? != "ok" {
            return Err(fail("SQLite integrity failure"));
        }
    }
    if c.prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(fail("SQLite foreign key failure"));
    }
    Ok(())
}

pub(crate) fn dispatch(
    command: crate::cli::StorageCommand,
) -> Result<crate::CommandOutput, DevMapError> {
    use crate::cli::StorageCommand::*;
    let source = match &command {
        Inspect(a) | Verify(a) => &a.source,
        Migrate(a) => &a.source,
        Backup(a) => &a.source,
    };
    let workspace = SourceGitInspector::open(source)?.workspace()?;
    let result = match command {
        Inspect(_) => inspect(&workspace)?,
        Verify(_) => verify(&workspace)?,
        Migrate(a) => ensure(&workspace, &a.backup_dir)?,
        Backup(a) => backup(&workspace, &a.destination)?,
    };
    Ok(crate::CommandOutput {
        stdout: format!("{}\n", serde_json::to_string_pretty(&result)?),
        exit_code: 0,
    })
}

#[cfg(test)]
mod verified_capture_tests {
    use super::*;
    use crate::events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    };

    fn source() -> (tempfile::TempDir, SourceWorkspace) {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let w = SourceGitInspector::open(dir.path())
            .unwrap()
            .workspace()
            .unwrap();
        (dir, w)
    }

    #[test]
    fn validated_snapshot_never_reopens_replaced_domain_files() {
        let (_source, w) = source();
        let now = OffsetDateTime::parse("2026-09-08T10:00:00Z", &Rfc3339).unwrap();
        let event = EventEnvelope::new(
            EVENT_SCHEMA_VERSION,
            "verified",
            EventType::CaptureGap,
            1,
            "2026-09-08T10:00:00Z",
            HostIdentity::new("host", "1").unwrap(),
            ActorIdentity::new("actor", None).unwrap(),
            SessionContext::new("s", None, "fixture", None, None, None).unwrap(),
            serde_json::json!({"reason":"verified"}),
        )
        .unwrap();
        let record = journal::JournalStore::open(&w, "s")
            .unwrap()
            .append(event)
            .unwrap();
        presence::PresenceStore::open(&w)
            .unwrap()
            .observe(presence::PresenceSignal::AcceptedRecords(&[record]), now)
            .unwrap();
        let worktree = WorktreeScanner::scan(&w).unwrap()[0].worktree_id.clone();
        route_plan::RoutePlanStore::open(&w)
            .unwrap()
            .set(route_plan::PlanInput {
                delivery: Default::default(),
                request_id: "request".into(),
                route_id: None,
                expected_revision: 0,
                worktree_id: worktree.clone(),
                goal: "verified".into(),
                target_ref: None,
                milestones: vec![],
                source: "user".into(),
                abandoned: false,
            })
            .unwrap();
        journal::observe_task_bindings(
            &w,
            &[("host".into(), "task".into(), worktree)],
            "2026-09-08T10:00:10Z",
        )
        .unwrap();
        let destination = tempfile::tempdir().unwrap();
        let root = destination.path().join("snapshot");
        freeze(&w, &root, now).unwrap();
        let captured = load_snapshot(&w, &root).unwrap();
        let expected = parse(&captured.manifest, &captured.files).unwrap();
        // Deterministic interleaving: validation has completed, then another writer
        // replaces every domain with syntactically valid same-record-count data.
        for file in &captured.manifest.files {
            if file.outcome != "domain" {
                continue;
            }
            let path = frozen_path(&root, file);
            let bytes = fs::read(&path).unwrap();
            let newline = bytes.last() == Some(&b'\n');
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            match file.relative.as_str() {
                "route-plans.jsonl" => {
                    value["input"]["goal"] = "tampered".into();
                    value["plan"]["goal"] = "tampered".into();
                }
                "task-bindings.jsonl" => value["observed_at"] = "2026-09-08T10:00:11Z".into(),
                "task-binding-watermarks.json" => {
                    value["observations"][0][2] = "2026-09-08T10:00:21Z".into()
                }
                name if name.ends_with("events.ndjson") => {
                    value["event"]["payload"]["reason"] = "tampered".into();
                    value.as_object_mut().unwrap().remove("sha256");
                    let hash = sha256_hex(&crate::canonical::canonical_json(&value).unwrap());
                    value["sha256"] = hash.into();
                }
                _ => value["actor_id"] = "other".into(),
            }
            let mut changed = crate::canonical::canonical_json(&value).unwrap();
            if newline {
                changed.push(b'\n');
            }
            fs::write(path, changed).unwrap();
        }
        let mut changed_manifest = serde_json::to_value(&captured.manifest).unwrap();
        changed_manifest["evaluated_at"] = "2026-09-09T10:00:00Z".into();
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&changed_manifest).unwrap(),
        )
        .unwrap();
        let actual = parse(&captured.manifest, &captured.files).unwrap();
        assert_eq!(
            serde_json::to_value(&actual.routes).unwrap(),
            serde_json::to_value(&expected.routes).unwrap()
        );
        assert_eq!(actual.bindings.records, expected.bindings.records);
        assert_eq!(actual.bindings.watermarks, expected.bindings.watermarks);
        assert_eq!(actual.journals[0].records, expected.journals[0].records);
        assert_eq!(actual.presence, expected.presence);
        assert_eq!(captured.manifest.evaluated_at, "2026-09-08T10:00:00Z");
        let changed_bytes = captured
            .manifest
            .files
            .iter()
            .map(|f| {
                (
                    (f.origin, f.relative.clone()),
                    fs::read(frozen_path(&root, f)).unwrap(),
                )
            })
            .collect();
        let changed = parse(&captured.manifest, &changed_bytes).unwrap();
        assert_eq!(actual.routes.len(), changed.routes.len());
        assert_ne!(
            serde_json::to_value(&actual.routes).unwrap(),
            serde_json::to_value(&changed.routes).unwrap()
        );
        assert_ne!(actual.bindings.records, changed.bindings.records);
        assert_ne!(actual.bindings.watermarks, changed.bindings.watermarks);
        assert_ne!(actual.journals[0].records, changed.journals[0].records);
        assert_ne!(actual.presence, changed.presence);
    }

    #[test]
    fn captured_missing_watermark_stays_missing_after_later_file_creation() {
        let (_source, w) = source();
        let worktree = WorktreeScanner::scan(&w).unwrap()[0].worktree_id.clone();
        journal::observe_task_bindings(
            &w,
            &[("host".into(), "task".into(), worktree)],
            "2026-09-08T10:00:10Z",
        )
        .unwrap();
        fs::remove_file(w.git_common_dir.join("devmap/task-binding-watermarks.json")).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let root = destination.path().join("snapshot");
        freeze(&w, &root, OffsetDateTime::now_utc()).unwrap();
        let captured = load_snapshot(&w, &root).unwrap();
        let common = captured
            .manifest
            .origins
            .iter()
            .position(|o| o.git_dir == captured.manifest.common_dir)
            .unwrap();
        let orphan = serde_json::json!({"schema_version":"devmap/task-binding-watermarks/1","repository_id":captured.manifest.repository_id,"observations":[["host","task","2026-09-08T10:00:20Z"]]});
        fs::write(
            root.join(common.to_string())
                .join("task-binding-watermarks.json"),
            serde_json::to_vec(&orphan).unwrap(),
        )
        .unwrap();
        let parsed = parse(&captured.manifest, &captured.files).unwrap();
        assert_eq!(
            parsed.bindings.watermarks[&("host".into(), "task".into())],
            "2026-09-08T10:00:10Z"
        );
        assert!(
            load_snapshot(&w, &root).is_err(),
            "a newly inventoried unlisted watermark must still fail closed"
        );
    }
}
