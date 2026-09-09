use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::canonical::{canonical_json, sha256_hex};
use crate::error::DevMapError;
use crate::events::{EventEnvelope, MAX_EVENT_BYTES};
use crate::fs_security::{
    FileIdentity, checked_directory_identity, checked_file, checked_metadata, checked_new_file,
    ensure_directory_chain, sync_directory,
};
use crate::git::SourceWorkspace;

#[cfg(test)]
#[path = "journal_summary_tests.rs"]
mod journal_summary_tests;

pub const MAX_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SESSION_RECORDS: usize = 100_000;
const MAX_INTENT_BYTES: usize = 1024 * 1024;
const MAX_INDEX_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = MAX_EVENT_BYTES + 16 * 1024;
const INDEX_VERSION: u8 = 1;
const BLOOM_WORDS: usize = 4096;

const MAX_BINDING_BYTES: u64 = 2 * 1024 * 1024;
const MAX_BINDING_RECORDS: usize = 2048;

/// A host-reported association change between observations. This is not proof
/// of when a task moved, who moved it, or where an agent actually executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBindingObservation {
    pub schema_version: String,
    pub repository_id: String,
    pub kind: String,
    pub host: String,
    pub task_id: String,
    pub worktree_id: String,
    pub from_worktree_id: Option<String>,
    pub source: String,
    pub observed_at: String,
    pub previous_observed_at: Option<String>,
    pub event_at: Option<String>,
}

pub(crate) type BindingWatermarkMap = BTreeMap<(String, String), String>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingWatermarks {
    schema_version: String,
    repository_id: String,
    observations: Vec<(String, String, String)>,
}

fn read_binding_watermarks(
    journal_path: &Path,
    repository: &str,
    records: &[TaskBindingObservation],
) -> Result<BindingWatermarkMap, DevMapError> {
    let path = journal_path.with_file_name("task-binding-watermarks.json");
    let temporary = path.with_extension("pending");
    // A pending update may contain a newer observation than the published file.
    // Do not discard it and accept delayed observations after an interruption.
    if checked_metadata(&temporary)?.is_some() {
        return Err(corruption(
            "pending task binding watermark; reconciliation required",
        ));
    }
    let bytes = if checked_metadata(&path)?.is_some() {
        let file = checked_file(&path, false, false)?;
        let mut bytes = Vec::new();
        file.take(MAX_BINDING_BYTES + 1).read_to_end(&mut bytes)?;
        Some(bytes)
    } else {
        None
    };
    parse_binding_watermarks(bytes.as_deref(), repository, records)
}

fn parse_binding_watermarks(
    bytes: Option<&[u8]>,
    repository: &str,
    records: &[TaskBindingObservation],
) -> Result<BindingWatermarkMap, DevMapError> {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let mut watermarks = BindingWatermarkMap::new();
    if let Some(bytes) = bytes {
        if bytes.len() as u64 > MAX_BINDING_BYTES {
            return Err(corruption("task binding watermark limit reached"));
        }
        let saved: BindingWatermarks = serde_json::from_slice(bytes)
            .map_err(|_| corruption("invalid task binding watermarks"))?;
        if saved.schema_version != "devmap/task-binding-watermarks/1"
            || saved.repository_id != repository
            || saved.observations.len() > MAX_BINDING_RECORDS
        {
            return Err(corruption("inconsistent task binding watermarks"));
        }
        for (host, task_id, observed_at) in saved.observations {
            if [&host, &task_id]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
                || OffsetDateTime::parse(&observed_at, &Rfc3339).is_err()
                || watermarks.insert((host, task_id), observed_at).is_some()
            {
                return Err(corruption("invalid task binding watermark observation"));
            }
        }
    }
    // Old journals predate watermarks; their recorded event times are a lower
    // bound, never evidence that no newer unchanged observation occurred.
    for record in records {
        let watermark = watermarks
            .entry((record.host.clone(), record.task_id.clone()))
            .or_insert_with(|| record.observed_at.clone());
        if OffsetDateTime::parse(watermark, &Rfc3339).unwrap()
            < OffsetDateTime::parse(&record.observed_at, &Rfc3339).unwrap()
        {
            *watermark = record.observed_at.clone();
        }
    }
    Ok(watermarks)
}

fn write_binding_watermarks(
    journal_path: &Path,
    repository: &str,
    watermarks: &BindingWatermarkMap,
) -> Result<(), DevMapError> {
    let path = journal_path.with_file_name("task-binding-watermarks.json");
    let temporary = path.with_extension("pending");
    let bytes = serde_json::to_vec(&BindingWatermarks {
        schema_version: "devmap/task-binding-watermarks/1".into(),
        repository_id: repository.into(),
        observations: watermarks
            .iter()
            .map(|((host, task), at)| (host.clone(), task.clone(), at.clone()))
            .collect(),
    })?;
    if bytes.len() as u64 > MAX_BINDING_BYTES || watermarks.len() > MAX_BINDING_RECORDS {
        return Err(corruption("task binding watermark limit reached"));
    }
    // All watermark reads/writes are protected by the stable journal file lock.
    let mut file = checked_new_file(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    checked_metadata(&path)?;
    // Replace in one rename; never delete the old durable watermark first.
    fs::rename(&temporary, &path)?;
    sync_directory(path.parent().expect("binding metadata directory"))
}

fn binding_path(workspace: &SourceWorkspace, create: bool) -> Result<Option<PathBuf>, DevMapError> {
    let root = crate::fs_security::checked_canonical_directory(&workspace.git_common_dir)?;
    let directory = if create {
        ensure_directory_chain(&root, &["devmap"])?
    } else {
        let path = root.join("devmap");
        let Some(metadata) = checked_metadata(&path)? else {
            return Ok(None);
        };
        if !metadata.is_dir() {
            return Err(corruption("invalid binding directory"));
        }
        path
    };
    let path = directory.join("task-bindings.jsonl");
    if !create && checked_metadata(&path)?.is_none() {
        return Ok(None);
    }
    Ok(Some(path))
}

fn read_binding_records(
    file: &mut File,
    repository: &str,
) -> Result<Vec<TaskBindingObservation>, DevMapError> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(MAX_BINDING_BYTES + 1).read_to_end(&mut bytes)?;
    parse_binding_records(&bytes, repository)
}
fn parse_binding_records(
    bytes: &[u8],
    repository: &str,
) -> Result<Vec<TaskBindingObservation>, DevMapError> {
    if bytes.len() as u64 > MAX_BINDING_BYTES || (!bytes.is_empty() && bytes.last() != Some(&b'\n'))
    {
        return Err(corruption("oversized or incomplete task binding journal"));
    }
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let mut records = Vec::new();
    let mut latest = BTreeMap::<(String, String), TaskBindingObservation>::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let record: TaskBindingObservation =
            serde_json::from_slice(line).map_err(|_| corruption("invalid task binding journal"))?;
        let key = (record.host.clone(), record.task_id.clone());
        let previous = latest.get(&key);
        if record.schema_version != "devmap/task-binding/1"
            || record.repository_id != repository
            || record.source != "host_task_inventory"
            || record.event_at.is_some()
            || OffsetDateTime::parse(&record.observed_at, &Rfc3339).is_err()
            || [&record.host, &record.task_id, &record.worktree_id]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
            || record.from_worktree_id.as_ref() != previous.map(|p| &p.worktree_id)
            || record.previous_observed_at.as_ref() != previous.map(|p| &p.observed_at)
            || record.kind
                != if previous.is_some() {
                    "task_migration_observed"
                } else {
                    "task_association_observed"
                }
            || previous.is_some_and(|p| {
                p.worktree_id == record.worktree_id
                    || OffsetDateTime::parse(&p.observed_at, &Rfc3339).unwrap()
                        >= OffsetDateTime::parse(&record.observed_at, &Rfc3339).unwrap()
            })
        {
            return Err(corruption("inconsistent task binding journal"));
        }
        latest.insert(key, record.clone());
        records.push(record);
        if records.len() > MAX_BINDING_RECORDS {
            return Err(corruption("task binding record limit reached"));
        }
    }
    Ok(records)
}

pub(crate) fn read_task_bindings(
    workspace: &SourceWorkspace,
) -> Result<Vec<TaskBindingObservation>, DevMapError> {
    if let Some(store) = crate::store::active_existing(workspace)? {
        return sql_binding_snapshot(
            store.connection(),
            &crate::worktrees::repository_id(workspace),
        )
        .map(|s| s.records);
    }
    legacy_binding_snapshot(workspace).map(|s| s.records)
}

#[derive(Debug, Clone)]
pub(crate) struct BindingSnapshot {
    pub(crate) records: Vec<TaskBindingObservation>,
    pub(crate) watermarks: BindingWatermarkMap,
}
/// Parse one already captured journal/watermark byte set without reopening paths.
/// None means the watermark was absent in that inventory, not an empty file.
pub(crate) fn parse_frozen_bindings(
    bytes: &[u8],
    watermark: Option<&[u8]>,
    repository: &str,
) -> Result<BindingSnapshot, DevMapError> {
    let records = parse_binding_records(bytes, repository)?;
    let watermarks = parse_binding_watermarks(watermark, repository, &records)?;
    Ok(BindingSnapshot {
        records,
        watermarks,
    })
}
/// Strict migration input: the independent watermark is retained even if no journal exists.
pub(crate) fn legacy_binding_snapshot(
    workspace: &SourceWorkspace,
) -> Result<BindingSnapshot, DevMapError> {
    let repository = crate::worktrees::repository_id(workspace);
    let path = crate::fs_security::checked_canonical_directory(&workspace.git_common_dir)?
        .join("devmap/task-bindings.jsonl");
    // Validate the legacy directory even when only watermark metadata exists.
    binding_path(workspace, false)?;
    legacy_binding_snapshot_at(&path, &repository)
}
/// Read a frozen journal and sibling watermark copy using its original identity.
pub(crate) fn legacy_binding_snapshot_at(
    path: &Path,
    repository: &str,
) -> Result<BindingSnapshot, DevMapError> {
    if checked_metadata(path)?.is_some() {
        let mut file = checked_file(path, false, false)?;
        FileExt::lock_shared(&file)?;
        let records = read_binding_records(&mut file, repository)?;
        let watermarks = read_binding_watermarks(path, repository, &records)?;
        return Ok(BindingSnapshot {
            records,
            watermarks,
        });
    }
    let records = Vec::new();
    let watermarks = read_binding_watermarks(path, repository, &records)?;
    Ok(BindingSnapshot {
        records,
        watermarks,
    })
}

pub(crate) fn sql_binding_snapshot(
    c: &rusqlite::Connection,
    repository: &str,
) -> Result<BindingSnapshot, DevMapError> {
    // Pin both tables to one generation; callers in a write transaction already hold a snapshot.
    let own = c.is_autocommit();
    if own {
        c.execute_batch("BEGIN")?;
    }
    let result = (|| {
        let mut bytes = Vec::new();
        let mut stmt = c.prepare("SELECT observation_id,host,task_id,observed_at,record_json FROM binding_records ORDER BY rowid")?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (id, host, task, at, json) = row?;
            let record: TaskBindingObservation = serde_json::from_str(&json)?;
            if host != record.host
                || task != record.task_id
                || at != record.observed_at
                || id != binding_id(&record)?
            {
                return Err(corruption("inconsistent SQL binding keys"));
            }
            bytes.extend(json.as_bytes());
            bytes.push(b'\n');
            if bytes.len() as u64 > MAX_BINDING_BYTES {
                return Err(corruption("task binding journal limit reached"));
            }
        }
        let records = parse_binding_records(&bytes, repository)?;
        let mut watermarks = BindingWatermarkMap::new();
        let mut stmt =
            c.prepare("SELECT source_scope,observed_at,record_json FROM binding_watermarks")?;
        let mut watermark_bytes = 0usize;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (key, at, json) = row?;
            watermark_bytes += json.len();
            let (host, task, time): (String, String, String) = serde_json::from_str(&json)?;
            if key != serde_json::to_string(&(host.clone(), task.clone()))?
                || at != time
                || [&host, &task]
                    .iter()
                    .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
                || time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
                    .is_err()
                || watermarks.insert((host, task), at).is_some()
                || watermarks.len() > MAX_BINDING_RECORDS
                || watermark_bytes as u64 > MAX_BINDING_BYTES
            {
                return Err(corruption("invalid SQL binding watermark"));
            }
        }
        for record in &records {
            let at = watermarks
                .get(&(record.host.clone(), record.task_id.clone()))
                .ok_or_else(|| corruption("missing SQL binding watermark"))?;
            if time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339)
                .map_err(|_| corruption("invalid SQL binding time"))?
                < time::OffsetDateTime::parse(
                    &record.observed_at,
                    &time::format_description::well_known::Rfc3339,
                )
                .map_err(|_| corruption("invalid SQL binding time"))?
            {
                return Err(corruption("stale SQL binding watermark"));
            }
        }
        Ok(BindingSnapshot {
            records,
            watermarks,
        })
    })();
    if own {
        c.execute_batch("ROLLBACK")?;
    }
    result
}
pub(crate) fn binding_id(record: &TaskBindingObservation) -> Result<String, DevMapError> {
    Ok(sha256_hex(&serde_json::to_vec(record)?))
}

pub(crate) fn validate_binding_observation(
    associations: &[(String, String, String)],
    observed_at: &str,
) -> Result<time::OffsetDateTime, DevMapError> {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let timestamp = OffsetDateTime::parse(observed_at, &Rfc3339)
        .map_err(|_| corruption("invalid binding observation time"))?;
    if associations.len() > MAX_BINDING_RECORDS
        || associations.iter().any(|(host, task, workspace)| {
            [host, task, workspace]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
        })
    {
        return Err(corruption("invalid task binding observation"));
    }
    Ok(timestamp)
}

pub(crate) fn observe_task_bindings(
    workspace: &SourceWorkspace,
    associations: &[(String, String, String)],
    observed_at: &str,
) -> Result<Vec<TaskBindingObservation>, DevMapError> {
    if associations.is_empty() {
        return read_task_bindings(workspace);
    }
    let timestamp = validate_binding_observation(associations, observed_at)?;
    crate::store::origin_write(workspace, |tx| {
        let repository = crate::worktrees::repository_id(workspace);
        if let Some((tx, admission)) = tx {
            let snapshot = sql_binding_snapshot(tx, &repository)?;
            return update_sql_bindings(
                tx,
                admission,
                &repository,
                snapshot,
                associations,
                observed_at,
                timestamp,
            );
        }
        let path = binding_path(workspace, true)?.expect("created binding directory");
        let mut file = checked_file(&path, true, true)?;
        crate::store::lock_domain_file(&file)?;
        let mut records = read_binding_records(&mut file, &repository)?;
        let mut watermarks = read_binding_watermarks(&path, &repository, &records)?;
        let (changed, additions) = update_bindings(
            &repository,
            &mut records,
            &mut watermarks,
            associations,
            observed_at,
            timestamp,
        )?;
        if file
            .metadata()?
            .len()
            .saturating_add(additions.len() as u64)
            > MAX_BINDING_BYTES
        {
            return Err(corruption("task binding journal limit reached"));
        }
        if changed {
            write_binding_watermarks(&path, &repository, &watermarks)?;
        }
        if !additions.is_empty() {
            file.seek(SeekFrom::End(0))?;
            file.write_all(&additions)?;
            file.sync_all()?;
        }
        Ok(records)
    })
}
fn update_sql_bindings(
    tx: &rusqlite::Transaction<'_>,
    admission: &crate::store::OriginAdmission,
    repository: &str,
    mut snapshot: BindingSnapshot,
    associations: &[(String, String, String)],
    observed_at: &str,
    timestamp: time::OffsetDateTime,
) -> Result<Vec<TaskBindingObservation>, DevMapError> {
    use crate::store::origin_links::{self, BindingCursor, BindingLink};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let mut metadata = origin_links::validate_bindings(tx, &snapshot)?;
    let mut changed = false;
    for (host, task, worktree) in associations {
        let key = (host.clone(), task.clone());
        let prior = metadata.cursors.get(&key).cloned();
        if let Some(cursor) = &prior {
            let prior_time = OffsetDateTime::parse(&cursor.observed_at, &Rfc3339)
                .map_err(|_| corruption("invalid binding cursor timestamp"))?;
            if timestamp < prior_time {
                continue;
            }
            if timestamp == prior_time {
                // Watermark-only legacy evidence blocks equal-time association
                // invention. A known equal-time observation cannot change identity.
                if let Some(current) = &cursor.current {
                    let target = admission.current_origin(worktree)?;
                    if current.worktree_id != *worktree
                        || current.incarnation.as_ref() != Some(&target.incarnation)
                    {
                        return Err(corruption("conflicting equal-time binding origin"));
                    }
                }
                continue;
            }
        }
        let destination = origin_links::register_origin(tx, admission.current_origin(worktree)?)?;
        let previous = snapshot
            .records
            .iter()
            .rev()
            .find(|r| &r.host == host && &r.task_id == task);
        let mut history = prior
            .as_ref()
            .and_then(|p| p.history_observation_id.clone());
        if previous.is_none_or(|r| r.worktree_id != *worktree) {
            let record = TaskBindingObservation {
                schema_version: "devmap/task-binding/1".into(),
                repository_id: repository.into(),
                kind: if previous.is_some() {
                    "task_migration_observed"
                } else {
                    "task_association_observed"
                }
                .into(),
                host: host.clone(),
                task_id: task.clone(),
                worktree_id: worktree.clone(),
                from_worktree_id: previous.map(|r| r.worktree_id.clone()),
                source: "host_task_inventory".into(),
                observed_at: observed_at.into(),
                previous_observed_at: previous.map(|r| r.observed_at.clone()),
                event_at: None,
            };
            let link = BindingLink {
                destination: destination.clone(),
                source: prior.as_ref().and_then(|p| p.current.clone()),
            };
            let id = binding_id(&record)?;
            tx.execute("INSERT INTO binding_records(observation_id,host,task_id,observed_at,record_json) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![id,record.host,record.task_id,record.observed_at,serde_json::to_string(&record)?])?;
            origin_links::insert_binding(tx, &record, &link)?;
            metadata.links.insert(id.clone(), link);
            history = Some(id);
            snapshot.records.push(record);
        }
        let cursor = BindingCursor {
            observed_at: observed_at.into(),
            current: Some(destination),
            history_observation_id: history,
        };
        tx.execute("INSERT INTO binding_watermarks(source_scope,observed_at,record_json) VALUES(?1,?2,?3) ON CONFLICT(source_scope) DO UPDATE SET observed_at=excluded.observed_at,record_json=excluded.record_json",rusqlite::params![serde_json::to_string(&(host,task))?,observed_at,serde_json::to_string(&(host,task,observed_at))?])?;
        origin_links::upsert_cursor(tx, host, task, &cursor)?;
        snapshot.watermarks.insert(key.clone(), observed_at.into());
        metadata.cursors.insert(key, cursor);
        changed = true;
    }
    if changed {
        // Reuse the bounded public history parser and exact metadata validator
        // before committing any newly accepted cursor or record.
        let accepted = sql_binding_snapshot(tx, repository)?;
        origin_links::validate_bindings(tx, &accepted)?;
        tx.execute(
            "UPDATE store_meta SET generation=generation+1 WHERE singleton=1",
            [],
        )?;
    }
    Ok(snapshot.records)
}
fn update_bindings(
    repository: &str,
    records: &mut Vec<TaskBindingObservation>,
    watermarks: &mut BindingWatermarkMap,
    associations: &[(String, String, String)],
    observed_at: &str,
    timestamp: time::OffsetDateTime,
) -> Result<(bool, Vec<u8>), DevMapError> {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let mut watermark_changed = false;
    let mut additions = Vec::new();
    for (host, task_id, worktree_id) in associations {
        let previous = records
            .iter()
            .rev()
            .find(|r| &r.host == host && &r.task_id == task_id);
        let key = (host.clone(), task_id.clone());
        if watermarks
            .get(&key)
            .is_some_and(|at| OffsetDateTime::parse(at, &Rfc3339).unwrap() >= timestamp)
        {
            continue;
        }
        watermarks.insert(key, observed_at.into());
        watermark_changed = true;
        if previous.is_some_and(|p| &p.worktree_id == worktree_id) {
            continue;
        }
        let record = TaskBindingObservation {
            schema_version: "devmap/task-binding/1".into(),
            repository_id: repository.to_owned(),
            kind: if previous.is_some() {
                "task_migration_observed"
            } else {
                "task_association_observed"
            }
            .into(),
            host: host.clone(),
            task_id: task_id.clone(),
            worktree_id: worktree_id.clone(),
            from_worktree_id: previous.map(|p| p.worktree_id.clone()),
            source: "host_task_inventory".into(),
            observed_at: observed_at.into(),
            previous_observed_at: previous.map(|p| p.observed_at.clone()),
            event_at: None,
        };
        additions.extend_from_slice(&serde_json::to_vec(&record)?);
        additions.push(b'\n');
        records.push(record);
    }
    let total = records
        .iter()
        .try_fold(0u64, |n, r| -> Result<u64, DevMapError> {
            Ok(n + serde_json::to_vec(r)?.len() as u64 + 1)
        })?;
    let saved = BindingWatermarks {
        schema_version: "devmap/task-binding-watermarks/1".into(),
        repository_id: repository.into(),
        observations: watermarks
            .iter()
            .map(|((h, t), a)| (h.clone(), t.clone(), a.clone()))
            .collect(),
    };
    if records.len() > MAX_BINDING_RECORDS
        || total > MAX_BINDING_BYTES
        || watermarks.len() > MAX_BINDING_RECORDS
        || serde_json::to_vec(&saved)?.len() as u64 > MAX_BINDING_BYTES
    {
        return Err(corruption("task binding journal limit reached"));
    }
    Ok((watermark_changed, additions))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub sequence: u64,
    pub event: EventEnvelope,
    pub previous_sha256: Option<String>,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalIntegrity {
    Verified,
    Missing,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalSummary {
    pub session_id: String,
    pub records: u64,
    pub last_sequence: Option<u64>,
    pub last_sha256: Option<String>,
    pub integrity: JournalIntegrity,
}

pub fn summarize_existing_sessions(
    workspace: &SourceWorkspace,
    session_ids: &BTreeSet<String>,
) -> BTreeMap<String, JournalSummary> {
    match crate::store::active_existing(workspace) {
        Ok(Some(store)) => {
            let result = (|| -> Result<_, DevMapError> {
                store.connection().execute_batch("BEGIN")?;
                Ok(session_ids
                    .iter()
                    .map(|id| {
                        let summary =
                            match sql_session_exists(store.connection(), id).and_then(|exists| {
                                if exists {
                                    sql_records(store.connection(), id).map(Some)
                                } else {
                                    Ok(None)
                                }
                            }) {
                                Ok(Some(records)) => JournalSummary {
                                    session_id: id.clone(),
                                    records: records.len() as u64,
                                    last_sequence: records.last().map(|r| r.sequence),
                                    last_sha256: records.last().map(|r| r.sha256.clone()),
                                    integrity: JournalIntegrity::Verified,
                                },
                                Ok(None) => empty_summary(id, JournalIntegrity::Missing),
                                Err(_) => empty_summary(id, JournalIntegrity::Corrupt),
                            };
                        (id.clone(), summary)
                    })
                    .collect())
            })();
            return result.unwrap_or_else(|_| {
                session_ids
                    .iter()
                    .map(|id| (id.clone(), empty_summary(id, JournalIntegrity::Corrupt)))
                    .collect()
            });
        }
        Err(_) => {
            return session_ids
                .iter()
                .map(|id| (id.clone(), empty_summary(id, JournalIntegrity::Corrupt)))
                .collect();
        }
        Ok(None) => {}
    }
    summarize_legacy_sessions(workspace, session_ids)
}

pub(crate) fn summarize_legacy_sessions(
    workspace: &SourceWorkspace,
    session_ids: &BTreeSet<String>,
) -> BTreeMap<String, JournalSummary> {
    let git_dirs = summary_git_directories(workspace);
    session_ids
        .iter()
        .map(|session_id| {
            (
                session_id.clone(),
                match &git_dirs {
                    Ok(git_dirs) => summarize_existing_session(git_dirs, session_id),
                    Err(_) => empty_summary(session_id, JournalIntegrity::Corrupt),
                },
            )
        })
        .collect()
}

fn empty_summary(session_id: &str, integrity: JournalIntegrity) -> JournalSummary {
    JournalSummary {
        session_id: session_id.to_owned(),
        records: 0,
        last_sequence: None,
        last_sha256: None,
        integrity,
    }
}

fn summary_git_directories(workspace: &SourceWorkspace) -> Result<Vec<PathBuf>, DevMapError> {
    const MAX_LINKED_WORKTREES: usize = 256;
    let mut git_dirs = vec![workspace.git_common_dir.clone()];
    let administration_root = workspace.git_common_dir.join("worktrees");
    match checked_metadata(&administration_root)? {
        None => return Ok(git_dirs),
        Some(metadata) if metadata.is_dir() => {}
        Some(_) => {
            return Err(corruption(
                "worktree administration root is not a directory",
            ));
        }
    }
    for (index, entry) in fs::read_dir(&administration_root)?.enumerate() {
        if index == MAX_LINKED_WORKTREES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal summary worktrees",
                limit: MAX_LINKED_WORKTREES,
            });
        }
        let path = entry?.path();
        match checked_metadata(&path)? {
            Some(metadata) if metadata.is_dir() => git_dirs.push(path),
            _ => return Err(corruption("invalid linked-worktree administration entry")),
        }
    }
    Ok(git_dirs)
}

fn summarize_existing_session(git_dirs: &[PathBuf], session_id: &str) -> JournalSummary {
    if !is_normal_session_component(session_id) {
        return empty_summary(session_id, JournalIntegrity::Corrupt);
    }
    let mut session_roots = Vec::new();
    for git_dir in git_dirs {
        let session_root = git_dir.join("devmap/sessions").join(session_id);
        match checked_metadata(&session_root) {
            Ok(Some(metadata)) if metadata.is_dir() => session_roots.push(session_root),
            Ok(None) => {}
            _ => return empty_summary(session_id, JournalIntegrity::Corrupt),
        }
    }
    if session_roots.is_empty() {
        return empty_summary(session_id, JournalIntegrity::Missing);
    }
    if session_roots.len() != 1 {
        return empty_summary(session_id, JournalIntegrity::Corrupt);
    }
    let session_root = &session_roots[0];
    let events = session_root.join("events.ndjson");
    match checked_metadata(&events) {
        Ok(Some(metadata)) if metadata.is_file() => {}
        Ok(None) => return empty_summary(session_id, JournalIntegrity::Missing),
        _ => return empty_summary(session_id, JournalIntegrity::Corrupt),
    }
    let records = read_limited(&events, MAX_JOURNAL_BYTES, "journal summary").and_then(|bytes| {
        let (records, complete_len) = parse_complete_records(&bytes)?;
        if complete_len != bytes.len() || records.len() > MAX_SESSION_RECORDS {
            return Err(corruption(
                "journal summary has an incomplete or oversized tail",
            ));
        }
        Ok(records)
    });
    match records {
        Ok(records) => JournalSummary {
            session_id: session_id.to_owned(),
            records: records.len() as u64,
            last_sequence: records.last().map(|record| record.sequence),
            last_sha256: records.last().map(|record| record.sha256.clone()),
            integrity: JournalIntegrity::Verified,
        },
        Err(_) => empty_summary(session_id, JournalIntegrity::Corrupt),
    }
}

#[derive(Debug, Clone)]
pub struct JournalStore {
    workspace: SourceWorkspace,
    project_capture: bool,
    root: PathBuf,
    session_id: String,
    root_identity: FileIdentity,
    session_identity: FileIdentity,
    opened_incarnation: String,
}

struct JournalAppendLock {
    file: File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalIntent {
    events: Vec<EventEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalIndex {
    version: u8,
    journal_bytes: u64,
    records: u64,
    last_record: Option<JournalRecord>,
    event_id_bloom: Vec<u64>,
}

#[derive(Serialize)]
struct UnsignedJournalRecord<'a> {
    sequence: u64,
    event: &'a EventEnvelope,
    previous_sha256: &'a Option<String>,
}

impl JournalStore {
    pub fn open(workspace: &SourceWorkspace, session_id: &str) -> Result<Self, DevMapError> {
        if !is_normal_session_component(session_id) {
            return Err(corruption("session ID must be a non-empty path component"));
        }

        let _transition = crate::store::transition::Guard::acquire(workspace)?;
        let opened_incarnation = worktree_incarnation(workspace)?;
        if crate::store::journal_read(workspace, session_id, &opened_incarnation, |_, _| Ok(()))?
            .is_some()
        {
            let identity = checked_directory_identity(&workspace.git_dir)?;
            return Ok(Self {
                workspace: workspace.clone(),
                project_capture: false,
                root: workspace.git_dir.clone(),
                session_id: session_id.to_owned(),
                root_identity: identity.clone(),
                session_identity: identity,
                opened_incarnation,
            });
        }
        let session_root =
            ensure_directory_chain(&workspace.git_dir, &["devmap", "sessions", session_id])?;
        let root = session_root
            .parent()
            .ok_or_else(|| corruption("session root has no parent"))?
            .to_path_buf();
        let root_identity = checked_directory_identity(&root)?;
        let session_identity = checked_directory_identity(&session_root)?;
        Ok(Self {
            workspace: workspace.clone(),
            project_capture: false,
            root,
            session_id: session_id.to_owned(),
            root_identity,
            session_identity,
            opened_incarnation,
        })
    }

    pub fn append(&self, event: EventEnvelope) -> Result<JournalRecord, DevMapError> {
        let mut records = self.append_batch_with(|_| Ok(vec![event]))?;
        Ok(records.remove(0))
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn append_batch_with<F>(&self, build: F) -> Result<Vec<JournalRecord>, DevMapError>
    where
        F: FnOnce(u64) -> Result<Vec<EventEnvelope>, DevMapError>,
    {
        if self.project_capture {
            return self.append_capture_batch_with(time::OffsetDateTime::now_utc(), build);
        }
        crate::store::journal_write_guarded(
            &self.workspace,
            &self.session_id,
            &self.opened_incarnation,
            |tx, _guard| {
                if let Some((tx, admission)) = tx {
                    return self.append_sql(tx, admission, build);
                }
                self.append_legacy(build)
            },
        )
    }

    /// Opt application capture into journal plus presence acceptance. Direct stores retain journal-only behavior.
    pub fn with_presence_projection(mut self) -> Self {
        self.project_capture = true;
        self
    }

    /// SQL acceptance is atomic. Legacy acceptance retains its historical best-effort projection.
    pub fn append_capture_batch_with<F>(
        &self,
        now: time::OffsetDateTime,
        build: F,
    ) -> Result<Vec<JournalRecord>, DevMapError>
    where
        F: FnOnce(u64) -> Result<Vec<EventEnvelope>, DevMapError>,
    {
        crate::store::journal_write_guarded(
            &self.workspace,
            &self.session_id,
            &self.opened_incarnation,
            |tx, guard| {
                if let Some((tx, admission)) = tx {
                    let before: i64 = tx.query_row(
                        "SELECT generation FROM store_meta WHERE singleton=1",
                        [],
                        |r| r.get(0),
                    )?;
                    let records = self.append_sql(tx, admission, build)?;
                    let (_, changed) =
                        crate::presence::PresenceStore::for_projection(&self.workspace)?
                            .observe_sql(
                                tx,
                                crate::presence::PresenceSignal::AcceptedRecords(&records),
                                now,
                            )?;
                    let after: i64 = tx.query_row(
                        "SELECT generation FROM store_meta WHERE singleton=1",
                        [],
                        |r| r.get(0),
                    )?;
                    if changed && before == after {
                        tx.execute(
                            "UPDATE store_meta SET generation=generation+1 WHERE singleton=1",
                            [],
                        )?;
                    }
                    return Ok(records);
                }
                let records = self.append_legacy(build)?;
                if let Err(error) =
                    crate::presence::PresenceStore::open_guarded(&self.workspace, guard).and_then(
                        |store| {
                            store.observe_legacy(
                                crate::presence::PresenceSignal::AcceptedRecords(&records),
                                now,
                            )
                        },
                    )
                {
                    eprintln!("devmap: presence update skipped: {error}");
                }
                Ok(records)
            },
        )
    }

    fn append_legacy<F>(&self, build: F) -> Result<Vec<JournalRecord>, DevMapError>
    where
        F: FnOnce(u64) -> Result<Vec<EventEnvelope>, DevMapError>,
    {
        let _lock = self.acquire_append_lock()?;
        self.recover_intent_locked()?;
        let mut index = self.load_or_rebuild_index_locked()?;
        let next_sequence = index
            .records
            .checked_add(1)
            .ok_or_else(|| corruption("journal sequence overflow"))?;
        let events = build(next_sequence)?;
        if events.is_empty() {
            return Err(corruption("journal batch must contain at least one event"));
        }
        self.validate_event_sessions(&events)?;
        if index.records as usize + events.len() > MAX_SESSION_RECORDS {
            return Err(DevMapError::ResourceLimit {
                resource: "journal session records",
                limit: MAX_SESSION_RECORDS,
            });
        }

        let might_be_retry = events
            .iter()
            .any(|event| index.might_contain(event.event_id()));
        if might_be_retry {
            let existing = self.replay_locked()?;
            let matching = events
                .iter()
                .map(|event| {
                    existing
                        .iter()
                        .find(|record| record.event.event_id() == event.event_id())
                })
                .collect::<Vec<_>>();
            if matching.iter().all(|record| record.is_some()) {
                let records = matching
                    .into_iter()
                    .map(Option::unwrap)
                    .cloned()
                    .collect::<Vec<_>>();
                if records
                    .iter()
                    .zip(&events)
                    .all(|(record, event)| equivalent_retry(&record.event, event))
                {
                    return Ok(records);
                }
                return Err(corruption("an event ID was reused for different content"));
            }
            if matching.iter().any(|record| record.is_some()) {
                return Err(corruption(
                    "a retried journal batch is only partially present",
                ));
            }
        }

        let appended = prepare_records_from_tail(&index, events.clone())?;
        let encoded = encode_records(&appended)?;
        let projected_size = index.journal_bytes as usize + encoded.len();
        if projected_size > MAX_JOURNAL_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal",
                limit: MAX_JOURNAL_BYTES,
            });
        }

        self.persist_intent(&events)?;
        self.append_encoded(&encoded)?;
        for record in &appended {
            index.push(record, canonical_json(record)?.len() as u64 + 1);
        }
        self.persist_index(&index)?;
        self.remove_intent()?;
        Ok(appended)
    }

    pub fn replay(&self) -> Result<Vec<JournalRecord>, DevMapError> {
        if let Some(records) = crate::store::journal_read(
            &self.workspace,
            &self.session_id,
            &self.opened_incarnation,
            |c, admission| {
                self.validate_sql_origin_at(c, &admission.sql_origin())?;
                sql_records(c, &self.session_id)
            },
        )? {
            return Ok(records);
        }
        crate::store::journal_write_guarded(
            &self.workspace,
            &self.session_id,
            &self.opened_incarnation,
            |tx, _guard| {
                if let Some((tx, admission)) = tx {
                    self.validate_sql_origin_at(tx, &admission.sql_origin())?;
                    return sql_records(tx, &self.session_id);
                }
                self.replay_legacy()
            },
        )
    }

    fn replay_legacy(&self) -> Result<Vec<JournalRecord>, DevMapError> {
        let _lock = self.acquire_append_lock()?;
        self.recover_intent_locked()?;
        let records = self.replay_locked()?;
        let journal_bytes = self.journal_len()?;
        self.persist_index(&JournalIndex::from_records(&records, journal_bytes))?;
        Ok(records)
    }

    fn validate_event_sessions(&self, events: &[EventEnvelope]) -> Result<(), DevMapError> {
        if let Some(event) = events
            .iter()
            .find(|event| event.context().session_id() != self.session_id)
        {
            return Err(DevMapError::SessionMismatch {
                journal_session: self.session_id.clone(),
                event_session: event.context().session_id().to_owned(),
            });
        }
        Ok(())
    }

    fn acquire_append_lock(&self) -> Result<JournalAppendLock, DevMapError> {
        self.validate_storage_identity()?;
        let path = self.lock_path();
        let existed = checked_metadata(&path)?.is_some();
        let file = checked_file(&path, true, true)?;
        self.validate_storage_identity()?;
        if !existed {
            sync_directory(&self.session_path())?;
        }
        crate::store::lock_domain_file(&file)?;
        Ok(JournalAppendLock { file })
    }

    fn recover_intent_locked(&self) -> Result<(), DevMapError> {
        self.remove_stale_temporary(&self.intent_temporary_path())?;
        self.remove_stale_temporary(&self.index_temporary_path())?;
        let intent_path = self.intent_path();
        if checked_metadata(&intent_path)?.is_none() {
            return Ok(());
        }
        let raw_intent = read_limited(&intent_path, MAX_INTENT_BYTES, "journal intent")?;
        let intent: JournalIntent = serde_json::from_slice(&raw_intent)
            .map_err(|error| corruption(format!("malformed journal intent: {error}")))?;
        if raw_intent != canonical_json(&intent)? {
            return Err(corruption("journal intent is not canonical JSON"));
        }
        if intent.events.is_empty() {
            return Err(corruption("journal intent must contain at least one event"));
        }
        self.validate_event_sessions(&intent.events)?;

        let bytes = self.read_journal_allowing_torn_tail()?;
        let (records, complete_len) = parse_complete_records(&bytes)?;
        let first_sequence = intent.events[0].sequence();
        if first_sequence == 0 {
            return Err(corruption("journal intent starts with sequence zero"));
        }
        for (index, event) in intent.events.iter().enumerate() {
            let expected_sequence = first_sequence + index as u64;
            if event.sequence() != expected_sequence {
                return Err(corruption(
                    "journal intent has non-contiguous event sequences",
                ));
            }
        }

        let base_len = (first_sequence - 1) as usize;
        if records.len() < base_len || records.len() > base_len + intent.events.len() {
            return Err(corruption(
                "journal records do not match the durable intent boundary",
            ));
        }
        let expected = prepare_records(records[..base_len].to_vec(), intent.events.clone())?;
        let completed_intent_records = records.len() - base_len;
        for (actual, intended) in records[base_len..].iter().zip(expected.iter()) {
            if actual != intended {
                return Err(corruption(
                    "journal records do not match the durable intent",
                ));
            }
        }

        if complete_len != bytes.len() {
            let expected_tail = expected
                .get(completed_intent_records)
                .ok_or_else(|| corruption("torn data follows a complete durable intent"))?;
            let expected_bytes = canonical_json(expected_tail)?;
            let tail = &bytes[complete_len..];
            if tail.is_empty() || !expected_bytes.starts_with(tail) {
                return Err(corruption(
                    "torn journal tail does not match the durable intent",
                ));
            }
            let file = checked_file(&self.events_path(), true, false)?;
            file.set_len(complete_len as u64)?;
            file.sync_data()?;
        }

        let remaining = &expected[completed_intent_records..];
        if !remaining.is_empty() {
            self.append_encoded(&encode_records(remaining)?)?;
        }
        let records = self.replay_locked()?;
        self.persist_index(&JournalIndex::from_records(&records, self.journal_len()?))?;
        self.remove_intent()
    }

    fn replay_locked(&self) -> Result<Vec<JournalRecord>, DevMapError> {
        let path = self.events_path();
        if checked_metadata(&path)?.is_none() {
            return Ok(Vec::new());
        }
        let bytes = read_limited(&path, MAX_JOURNAL_BYTES, "journal")?;
        let (records, complete_len) = parse_complete_records(&bytes)?;
        if complete_len != bytes.len() {
            return Err(corruption(format!(
                "record at line {} is missing its terminating newline",
                records.len() + 1
            )));
        }
        if records.len() > MAX_SESSION_RECORDS {
            return Err(DevMapError::ResourceLimit {
                resource: "journal session records",
                limit: MAX_SESSION_RECORDS,
            });
        }
        self.validate_event_sessions(
            &records
                .iter()
                .map(|record| record.event.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(records)
    }

    fn load_or_rebuild_index_locked(&self) -> Result<JournalIndex, DevMapError> {
        let path = self.index_path();
        if checked_metadata(&path)?.is_none() {
            let records = self.replay_locked()?;
            let index = JournalIndex::from_records(&records, self.journal_len()?);
            if index.records > 0 {
                self.persist_index(&index)?;
            }
            return Ok(index);
        }
        let bytes = read_limited(&path, MAX_INDEX_BYTES, "journal index")?;
        let index: JournalIndex = serde_json::from_slice(&bytes)
            .map_err(|error| corruption(format!("malformed journal index: {error}")))?;
        if bytes != canonical_json(&index)? {
            return Err(corruption("journal index is not canonical JSON"));
        }
        index.validate_shape()?;
        let actual_len = self.journal_len()?;
        if index.journal_bytes != actual_len {
            return Err(corruption("journal index length does not match journal"));
        }
        if index.records == 0 {
            if index.last_record.is_some() || actual_len != 0 {
                return Err(corruption("empty journal index has a non-empty tail"));
            }
        } else {
            let tail = self.read_tail_record()?;
            if index.last_record.as_ref() != Some(&tail)
                || tail.sequence != index.records
                || tail.sha256 != tail.expected_sha256()?
            {
                return Err(corruption("journal index tail validation failed"));
            }
        }
        Ok(index)
    }

    fn read_tail_record(&self) -> Result<JournalRecord, DevMapError> {
        let path = self.events_path();
        let mut file = checked_file(&path, false, false)?;
        let length = file.metadata()?.len() as usize;
        if length == 0 || length > MAX_JOURNAL_BYTES {
            return Err(corruption("journal tail length is invalid"));
        }
        let start = length.saturating_sub(MAX_RECORD_BYTES);
        file.seek(SeekFrom::Start(start as u64))?;
        let mut bytes = Vec::with_capacity(length - start);
        file.read_to_end(&mut bytes)?;
        if !bytes.ends_with(b"\n") {
            return Err(corruption("journal tail is not newline terminated"));
        }
        let without_final = &bytes[..bytes.len() - 1];
        let line_start = without_final
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        if start > 0 && line_start == 0 {
            return Err(corruption("journal record exceeds the record byte limit"));
        }
        let line = &without_final[line_start..];
        parse_record(line, 0)
    }

    fn persist_intent(&self, events: &[EventEnvelope]) -> Result<(), DevMapError> {
        let bytes = canonical_json(&JournalIntent {
            events: events.to_vec(),
        })?;
        if bytes.len() > MAX_INTENT_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal intent",
                limit: MAX_INTENT_BYTES,
            });
        }
        self.write_atomic_rebuildable(&self.intent_path(), &self.intent_temporary_path(), &bytes)
    }

    fn persist_index(&self, index: &JournalIndex) -> Result<(), DevMapError> {
        let bytes = canonical_json(index)?;
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal index",
                limit: MAX_INDEX_BYTES,
            });
        }
        self.write_atomic_rebuildable(&self.index_path(), &self.index_temporary_path(), &bytes)
    }

    fn write_atomic_rebuildable(
        &self,
        destination: &Path,
        temporary: &Path,
        bytes: &[u8],
    ) -> Result<(), DevMapError> {
        self.validate_storage_identity()?;
        if checked_metadata(temporary)?.is_some() {
            return Err(corruption(format!(
                "stale temporary journal path: {}",
                temporary.display()
            )));
        }
        let mut file = checked_new_file(temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        self.validate_storage_identity()?;
        if checked_metadata(destination)?.is_some() {
            fs::remove_file(destination)?;
        }
        fs::rename(temporary, destination)?;
        self.validate_storage_identity()?;
        sync_directory(&self.session_path())
    }

    fn append_encoded(&self, encoded: &[u8]) -> Result<(), DevMapError> {
        if encoded.is_empty() {
            return Ok(());
        }
        self.validate_storage_identity()?;
        let path = self.events_path();
        let existed = checked_metadata(&path)?.is_some();
        let mut file = checked_file(&path, true, true)?;
        self.validate_storage_identity()?;
        file.seek(SeekFrom::End(0))?;
        file.write_all(encoded)?;
        file.sync_data()?;
        drop(file);
        self.validate_storage_identity()?;
        if !existed {
            sync_directory(&self.session_path())?;
        }
        Ok(())
    }

    fn remove_intent(&self) -> Result<(), DevMapError> {
        self.validate_storage_identity()?;
        let path = self.intent_path();
        if checked_metadata(&path)?.is_some() {
            fs::remove_file(path)?;
            self.validate_storage_identity()?;
            sync_directory(&self.session_path())?;
        }
        Ok(())
    }

    fn remove_stale_temporary(&self, path: &Path) -> Result<(), DevMapError> {
        self.validate_storage_identity()?;
        if let Some(metadata) = checked_metadata(path)? {
            if !metadata.is_file() {
                return Err(corruption(format!(
                    "temporary journal path is not a file: {}",
                    path.display()
                )));
            }
            fs::remove_file(path)?;
            self.validate_storage_identity()?;
            sync_directory(&self.session_path())?;
        }
        Ok(())
    }

    fn read_journal_allowing_torn_tail(&self) -> Result<Vec<u8>, DevMapError> {
        let path = self.events_path();
        if checked_metadata(&path)?.is_none() {
            return Ok(Vec::new());
        }
        read_limited(&path, MAX_JOURNAL_BYTES, "journal")
    }

    fn journal_len(&self) -> Result<u64, DevMapError> {
        Ok(checked_metadata(&self.events_path())?
            .map(|metadata| metadata.len())
            .unwrap_or(0))
    }

    fn validate_storage_identity(&self) -> Result<(), DevMapError> {
        if checked_directory_identity(&self.root)? != self.root_identity
            || checked_directory_identity(&self.session_path())? != self.session_identity
        {
            return Err(corruption("journal storage directory identity changed"));
        }
        Ok(())
    }

    fn session_path(&self) -> PathBuf {
        self.root.join(&self.session_id)
    }
    fn events_path(&self) -> PathBuf {
        self.session_path().join("events.ndjson")
    }
    fn intent_path(&self) -> PathBuf {
        self.session_path().join("events.intent")
    }
    fn intent_temporary_path(&self) -> PathBuf {
        self.session_path().join("events.intent.tmp")
    }
    fn index_path(&self) -> PathBuf {
        self.session_path().join("events.index")
    }
    fn index_temporary_path(&self) -> PathBuf {
        self.session_path().join("events.index.tmp")
    }
    fn lock_path(&self) -> PathBuf {
        self.session_path().join("events.lock")
    }
}

impl Drop for JournalAppendLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl JournalIndex {
    fn empty() -> Self {
        Self {
            version: INDEX_VERSION,
            journal_bytes: 0,
            records: 0,
            last_record: None,
            event_id_bloom: vec![0; BLOOM_WORDS],
        }
    }

    fn from_records(records: &[JournalRecord], journal_bytes: u64) -> Self {
        let mut index = Self::empty();
        index.journal_bytes = journal_bytes;
        index.records = records.len() as u64;
        index.last_record = records.last().cloned();
        for record in records {
            index.insert_id(record.event.event_id());
        }
        index
    }

    fn validate_shape(&self) -> Result<(), DevMapError> {
        if self.version != INDEX_VERSION || self.event_id_bloom.len() != BLOOM_WORDS {
            return Err(corruption("unsupported journal index shape"));
        }
        if self.records as usize > MAX_SESSION_RECORDS {
            return Err(DevMapError::ResourceLimit {
                resource: "journal session records",
                limit: MAX_SESSION_RECORDS,
            });
        }
        Ok(())
    }

    fn might_contain(&self, event_id: &str) -> bool {
        bloom_positions(event_id)
            .into_iter()
            .all(|position| self.event_id_bloom[position / 64] & (1 << (position % 64)) != 0)
    }

    fn insert_id(&mut self, event_id: &str) {
        for position in bloom_positions(event_id) {
            self.event_id_bloom[position / 64] |= 1 << (position % 64);
        }
    }

    fn push(&mut self, record: &JournalRecord, encoded_len: u64) {
        self.journal_bytes += encoded_len;
        self.records += 1;
        self.last_record = Some(record.clone());
        self.insert_id(record.event.event_id());
    }
}

fn bloom_positions(event_id: &str) -> [usize; 4] {
    let digest = sha256_hex(event_id.as_bytes());
    let mut positions = [0; 4];
    for (index, position) in positions.iter_mut().enumerate() {
        let start = index * 8;
        let value = u32::from_str_radix(&digest[start..start + 8], 16).unwrap_or(0);
        *position = value as usize % (BLOOM_WORDS * 64);
    }
    positions
}

fn prepare_records_from_tail(
    index: &JournalIndex,
    events: Vec<EventEnvelope>,
) -> Result<Vec<JournalRecord>, DevMapError> {
    let mut sequence = index.records;
    let mut previous_sha256 = index
        .last_record
        .as_ref()
        .map(|record| record.sha256.clone());
    let mut appended = Vec::with_capacity(events.len());
    for event in events {
        sequence += 1;
        if event.sequence() != sequence {
            if event.sequence() < sequence {
                return Err(DevMapError::DuplicateSequence(event.sequence()));
            }
            return Err(corruption(format!(
                "expected sequence {sequence}, found {}",
                event.sequence()
            )));
        }
        let record = JournalRecord::new(event, previous_sha256)?;
        previous_sha256 = Some(record.sha256.clone());
        appended.push(record);
    }
    Ok(appended)
}

fn prepare_records(
    mut records: Vec<JournalRecord>,
    events: Vec<EventEnvelope>,
) -> Result<Vec<JournalRecord>, DevMapError> {
    let mut appended = Vec::with_capacity(events.len());
    let mut known_ids: HashSet<String> = records
        .iter()
        .map(|record| record.event.event_id().to_owned())
        .collect();
    for event in events {
        let expected_sequence = records.len() as u64 + 1;
        if event.sequence() != expected_sequence {
            if event.sequence() < expected_sequence {
                return Err(DevMapError::DuplicateSequence(event.sequence()));
            }
            return Err(corruption(format!(
                "expected sequence {expected_sequence}, found {}",
                event.sequence()
            )));
        }
        if !known_ids.insert(event.event_id().to_owned()) {
            return Err(corruption(format!(
                "duplicate event ID {}",
                event.event_id()
            )));
        }
        let previous_sha256 = records.last().map(|record| record.sha256.clone());
        let record = JournalRecord::new(event, previous_sha256)?;
        records.push(record.clone());
        appended.push(record);
    }
    Ok(appended)
}

fn encode_records(records: &[JournalRecord]) -> Result<Vec<u8>, DevMapError> {
    let mut bytes = Vec::new();
    for record in records {
        let encoded = canonical_json(record)?;
        if encoded.len() > MAX_RECORD_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal record",
                limit: MAX_RECORD_BYTES,
            });
        }
        bytes.extend_from_slice(&encoded);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn parse_complete_records(bytes: &[u8]) -> Result<(Vec<JournalRecord>, usize), DevMapError> {
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut records = Vec::new();
    let mut event_ids = HashSet::new();
    let mut previous_sha256 = None;
    let complete = &bytes[..complete_len];
    let records_bytes = complete.strip_suffix(b"\n").unwrap_or(complete);
    if records_bytes.is_empty() {
        if complete_len == 0 {
            return Ok((records, complete_len));
        }
        return Err(corruption("empty record at line 1"));
    }
    for (line_index, line) in records_bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() {
            return Err(corruption(format!(
                "empty record at line {}",
                line_index + 1
            )));
        }
        let record = parse_record(line, line_index + 1)?;
        let expected_sequence = records.len() as u64 + 1;
        if record.sequence < expected_sequence {
            return Err(DevMapError::DuplicateSequence(record.sequence));
        }
        if record.sequence != expected_sequence {
            return Err(corruption(format!(
                "expected sequence {expected_sequence}, found {} at line {}",
                record.sequence,
                line_index + 1
            )));
        }
        if record.event.sequence() != record.sequence {
            return Err(corruption(format!(
                "event sequence does not match record sequence at line {}",
                line_index + 1
            )));
        }
        if !event_ids.insert(record.event.event_id().to_owned()) {
            return Err(corruption(format!(
                "duplicate event ID {}",
                record.event.event_id()
            )));
        }
        if record.previous_sha256 != previous_sha256 {
            return Err(corruption(format!(
                "previous SHA-256 link mismatch at line {}",
                line_index + 1
            )));
        }
        previous_sha256 = Some(record.sha256.clone());
        records.push(record);
    }
    Ok((records, complete_len))
}

fn parse_record(line: &[u8], line_number: usize) -> Result<JournalRecord, DevMapError> {
    if line.len() > MAX_RECORD_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "journal record",
            limit: MAX_RECORD_BYTES,
        });
    }
    let record: JournalRecord = serde_json::from_slice(line)
        .map_err(|error| corruption(format!("malformed JSON at line {line_number}: {error}")))?;
    if line != canonical_json(&record)? {
        return Err(corruption(format!(
            "record at line {line_number} is not canonical JSON"
        )));
    }
    if record.sha256 != record.expected_sha256()? {
        return Err(corruption(format!(
            "SHA-256 mismatch at line {line_number}"
        )));
    }
    Ok(record)
}

fn read_limited(path: &Path, limit: usize, resource: &'static str) -> Result<Vec<u8>, DevMapError> {
    let metadata = checked_metadata(path)?.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, path.display().to_string())
    })?;
    if metadata.len() > limit as u64 {
        return Err(DevMapError::ResourceLimit { resource, limit });
    }
    let file = checked_file(path, false, false)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(DevMapError::ResourceLimit { resource, limit });
    }
    Ok(bytes)
}

fn equivalent_retry(existing: &EventEnvelope, retried: &EventEnvelope) -> bool {
    let Ok(mut existing) = serde_json::to_value(existing) else {
        return false;
    };
    let Ok(mut retried) = serde_json::to_value(retried) else {
        return false;
    };
    for value in [&mut existing, &mut retried] {
        if let Some(object) = value.as_object_mut() {
            object.remove("sequence");
            object.remove("occurred_at");
        }
    }
    existing == retried
}

pub(crate) fn is_normal_session_component(session_id: &str) -> bool {
    if session_id.trim().is_empty()
        || session_id.contains(['/', '\\'])
        || (session_id.len() >= 2 && session_id.as_bytes()[1] == b':')
    {
        return false;
    }
    let mut components = Path::new(session_id).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

impl JournalRecord {
    fn new(event: EventEnvelope, previous_sha256: Option<String>) -> Result<Self, DevMapError> {
        let sequence = event.sequence();
        let mut record = Self {
            sequence,
            event,
            previous_sha256,
            sha256: String::new(),
        };
        record.sha256 = record.expected_sha256()?;
        Ok(record)
    }

    fn expected_sha256(&self) -> Result<String, DevMapError> {
        let unsigned = UnsignedJournalRecord {
            sequence: self.sequence,
            event: &self.event,
            previous_sha256: &self.previous_sha256,
        };
        Ok(sha256_hex(&canonical_json(&unsigned)?))
    }
}

fn corruption(message: impl Into<String>) -> DevMapError {
    DevMapError::JournalCorruption(message.into())
}

#[cfg(test)]
mod binding_snapshot_tests {
    use super::*;
    #[test]
    fn strict_binding_snapshot_keeps_orphan_watermark_and_rejects_pending() {
        let d = tempfile::tempdir().unwrap();
        let common = d.path().join("git");
        std::fs::create_dir_all(common.join("devmap")).unwrap();
        let w = SourceWorkspace {
            root: d.path().into(),
            git_dir: common.clone(),
            git_common_dir: common.clone(),
            branch: None,
            head: String::new(),
        };
        let p = common.join("devmap/task-binding-watermarks.json");
        let saved = BindingWatermarks {
            schema_version: "devmap/task-binding-watermarks/1".into(),
            repository_id: crate::worktrees::repository_id(&w),
            observations: vec![("local".into(), "task".into(), "2026-09-08T12:00:00Z".into())],
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        std::fs::write(&p, &bytes).unwrap();
        let snapshot = legacy_binding_snapshot(&w).unwrap();
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.watermarks.len(), 1);
        let captured = parse_frozen_bindings(&[], Some(&bytes), &saved.repository_id).unwrap();
        assert_eq!(captured.records, snapshot.records);
        assert_eq!(captured.watermarks, snapshot.watermarks);
        assert!(
            parse_frozen_bindings(&[], None, &saved.repository_id)
                .unwrap()
                .watermarks
                .is_empty()
        );
        assert!(parse_frozen_bindings(&[], Some(&[]), &saved.repository_id).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        assert!(!common.join("devmap/task-bindings.jsonl").exists());
        std::fs::write(p.with_extension("pending"), b"pending").unwrap();
        assert!(legacy_binding_snapshot(&w).is_err());
        assert!(p.with_extension("pending").exists());
    }
}

impl JournalStore {
    fn validate_sql_origin_at(
        &self,
        c: &rusqlite::Connection,
        origin: &(String, String, String),
    ) -> Result<(), DevMapError> {
        use rusqlite::OptionalExtension;
        let saved: Option<(String, String, String)> = c.query_row(
            "SELECT worktree_id,incarnation,origin_path FROM journal_sessions WHERE session_id=?1",[&self.session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if saved.is_some_and(|saved| &saved != origin) {
            return Err(corruption(
                "session origin or worktree incarnation mismatch",
            ));
        }
        Ok(())
    }
    fn append_sql<F>(
        &self,
        tx: &rusqlite::Transaction<'_>,
        admission: &crate::store::JournalAdmission,
        build: F,
    ) -> Result<Vec<JournalRecord>, DevMapError>
    where
        F: FnOnce(u64) -> Result<Vec<EventEnvelope>, DevMapError>,
    {
        use rusqlite::OptionalExtension;
        let current_origin = admission.sql_origin();
        self.validate_sql_origin_at(tx, &current_origin)?;
        // Registry identity is authoritative even before this session is registered.
        // Validate it under the acceptance transaction before invoking the builder.
        let registry: Option<(String, String, Option<String>)> = tx.query_row(
            "SELECT git_dir,workspace_path,retired_at FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",
            rusqlite::params![current_origin.0, current_origin.1],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        if let Some((git_dir, workspace_path, retired_at)) = registry {
            if git_dir != current_origin.2
                || workspace_path != self.workspace.root.to_string_lossy()
            {
                return Err(corruption("worktree registry origin mismatch"));
            }
            if retired_at.is_some() {
                return Err(corruption("retired worktree incarnation cannot append"));
            }
        }
        let existing = sql_records(tx, &self.session_id)?;
        let events = build(existing.len() as u64 + 1)?;
        if events.is_empty() {
            return Err(corruption("journal batch must contain at least one event"));
        }
        self.validate_event_sessions(&events)?;
        if events
            .iter()
            .any(|event| event.context().route_id().is_some())
        {
            // Lifecycle qualification grants no route association, including
            // exact receipt retries. The original strict policy still applies.
            crate::store::migration::check_legacy_drift(&self.workspace, tx)?;
        }
        let matching = events
            .iter()
            .map(|event| {
                existing
                    .iter()
                    .find(|r| r.event.event_id() == event.event_id())
            })
            .collect::<Vec<_>>();
        if matching.iter().all(|r| r.is_some()) {
            let records = matching
                .into_iter()
                .map(|r| r.unwrap().clone())
                .collect::<Vec<_>>();
            if records
                .iter()
                .zip(&events)
                .all(|(r, e)| equivalent_retry(&r.event, e))
            {
                return Ok(records);
            }
            return Err(corruption("an event ID was reused for different content"));
        }
        if matching.iter().any(|r| r.is_some()) {
            return Err(corruption(
                "a retried journal batch is only partially present",
            ));
        }
        if existing.len() + events.len() > MAX_SESSION_RECORDS {
            return Err(DevMapError::ResourceLimit {
                resource: "journal session records",
                limit: MAX_SESSION_RECORDS,
            });
        }
        let old_bytes = encode_records(&existing)?.len();
        let records = prepare_records(existing, events)?;
        if old_bytes + encode_records(&records)?.len() > MAX_JOURNAL_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "journal",
                limit: MAX_JOURNAL_BYTES,
            });
        }
        let (worktree, incarnation, origin) = current_origin;
        tx.execute("INSERT OR IGNORE INTO worktree_registry(worktree_id,incarnation,git_dir,workspace_path) VALUES(?1,?2,?3,?4)",rusqlite::params![worktree,incarnation,origin,self.workspace.root.to_string_lossy()])?;
        tx.execute("INSERT OR IGNORE INTO journal_sessions(session_id,worktree_id,incarnation,origin_path) VALUES(?1,?2,?3,?4)",rusqlite::params![self.session_id,worktree,incarnation,origin])?;
        for r in &records {
            let bytes = canonical_json(r)?;
            tx.execute("INSERT INTO journal_records(session_id,sequence,event_id,record_json,byte_length) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![self.session_id,r.sequence as i64,r.event.event_id(),String::from_utf8(bytes.clone()).map_err(|_|corruption("invalid utf8"))?,bytes.len() as i64])?;
        }
        let last = records.last().expect("non-empty validated batch");
        tx.execute("INSERT INTO journal_heads(session_id,record_count,last_sha256,byte_length) VALUES(?1,?2,?3,?4) ON CONFLICT(session_id) DO UPDATE SET record_count=excluded.record_count,last_sha256=excluded.last_sha256,byte_length=excluded.byte_length",rusqlite::params![self.session_id,last.sequence as i64,last.sha256,(old_bytes+encode_records(&records)?.len()) as i64])?;
        tx.execute(
            "UPDATE store_meta SET generation=generation+1 WHERE singleton=1",
            [],
        )?;
        Ok(records)
    }
}

fn sql_session_exists(c: &rusqlite::Connection, id: &str) -> Result<bool, DevMapError> {
    use rusqlite::OptionalExtension;
    struct Registration {
        worktree: String,
        incarnation: String,
        origin: String,
        git_dir: Option<String>,
    }
    let saved = c.query_row(
        "SELECT s.worktree_id,s.incarnation,s.origin_path,w.git_dir FROM journal_sessions s LEFT JOIN worktree_registry w ON s.worktree_id=w.worktree_id AND s.incarnation=w.incarnation WHERE session_id=?1",
        [id],
        |r| Ok(Registration { worktree:r.get(0)?, incarnation:r.get(1)?, origin:r.get(2)?, git_dir:r.get(3)? }),
    ).optional()?;
    if let Some(Registration {
        worktree,
        incarnation,
        origin,
        git_dir,
    }) = saved
    {
        if worktree.is_empty()
            || incarnation.is_empty()
            || origin.is_empty()
            || git_dir.as_deref() != Some(origin.as_str())
        {
            return Err(corruption("invalid session registration"));
        }
        let repository: String = c.query_row(
            "SELECT repository_id FROM store_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let normalized = origin.replace('\\', "/");
        let normalized = if cfg!(windows) {
            normalized.to_lowercase()
        } else {
            normalized
        };
        if worktree
            != format!(
                "wt-{}",
                sha256_hex(format!("{repository}\0{normalized}").as_bytes())
            )
        {
            return Err(corruption("session worktree identity mismatch"));
        }
        Ok(true)
    } else {
        let n: i64 = c.query_row(
            "SELECT (SELECT count(*) FROM journal_records WHERE session_id=?1)+(SELECT count(*) FROM journal_heads WHERE session_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if n != 0 {
            return Err(corruption("unregistered journal records"));
        }
        Ok(false)
    }
}

/// Verify one SQL journal without retaining its full event payloads. The caller
/// owns the read transaction, just as for `sql_records`.
pub(crate) fn sql_summary(
    c: &rusqlite::Connection,
    id: &str,
) -> Result<JournalSummary, DevMapError> {
    if !is_normal_session_component(id) {
        return Err(corruption("invalid frozen journal identity or size"));
    }
    let registered = sql_session_exists(c, id)?;
    let mut stmt = c.prepare(
        "SELECT sequence,CASE WHEN length(CAST(event_id AS BLOB))<=?2 THEN event_id END,CASE WHEN length(CAST(record_json AS BLOB))<=?2 THEN record_json END,byte_length FROM journal_records WHERE session_id=?1 ORDER BY sequence")?;
    let mut rows = stmt.query(rusqlite::params![id, MAX_RECORD_BYTES as i64])?;
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut previous_sha256 = None;
    let mut event_ids = HashSet::new();
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let event_id: String = row.get(1)?;
        let json: String = row.get(2)?;
        let size: i64 = row.get(3)?;
        if json.len() as i64 != size {
            return Err(corruption("journal byte length mismatch"));
        }
        // Count the NDJSON separator used by the authoritative saved extent.
        bytes = bytes
            .checked_add(json.len() + 1)
            .ok_or_else(|| corruption("journal resource limit exceeded"))?;
        count += 1;
        if bytes > MAX_JOURNAL_BYTES || count > MAX_SESSION_RECORDS {
            return Err(corruption("journal resource limit exceeded"));
        }
        let record = parse_record(json.as_bytes(), count)?;
        if record.sequence as i64 != sequence
            || record.event.event_id() != event_id
            || record.event.context().session_id() != id
        {
            return Err(corruption("journal row identity mismatch"));
        }
        if record.sequence < count as u64 {
            return Err(DevMapError::DuplicateSequence(record.sequence));
        }
        if record.sequence != count as u64 {
            return Err(corruption(format!(
                "expected sequence {count}, found {} at line {count}",
                record.sequence
            )));
        }
        if record.event.sequence() != record.sequence {
            return Err(corruption(format!(
                "event sequence does not match record sequence at line {count}"
            )));
        }
        if !event_ids.insert(event_id) {
            return Err(corruption(format!(
                "duplicate event ID {}",
                record.event.event_id()
            )));
        }
        if record.previous_sha256 != previous_sha256 {
            return Err(corruption(format!(
                "previous SHA-256 link mismatch at line {count}"
            )));
        }
        previous_sha256 = Some(record.sha256);
    }
    if registered {
        let (saved_count, saved_hash, saved_bytes): (i64, Option<String>, i64) = c.query_row(
            "SELECT record_count,last_sha256,byte_length FROM journal_heads WHERE session_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if saved_count != count as i64
            || saved_hash != previous_sha256
            || saved_bytes != bytes as i64
        {
            return Err(corruption("journal accepted extent mismatch"));
        }
    }
    Ok(JournalSummary {
        session_id: id.to_owned(),
        records: count as u64,
        last_sequence: (count > 0).then_some(count as u64),
        last_sha256: previous_sha256,
        integrity: if registered {
            JournalIntegrity::Verified
        } else {
            JournalIntegrity::Missing
        },
    })
}

pub(crate) fn sql_records(
    c: &rusqlite::Connection,
    id: &str,
) -> Result<Vec<JournalRecord>, DevMapError> {
    let registered = sql_session_exists(c, id)?;
    let mut stmt = c.prepare(
        "SELECT sequence,event_id,CASE WHEN length(CAST(record_json AS BLOB))<=?2 THEN record_json END,byte_length FROM journal_records WHERE session_id=?1 ORDER BY sequence")?;
    let mut rows = stmt.query(rusqlite::params![id, MAX_RECORD_BYTES as i64])?;
    let mut bytes = Vec::new();
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let event_id: String = row.get(1)?;
        let json: String = row.get(2)?;
        let size: i64 = row.get(3)?;
        if json.len() as i64 != size {
            return Err(corruption("journal byte length mismatch"));
        }
        let record = parse_record(json.as_bytes(), count + 1)?;
        if record.sequence as i64 != sequence
            || record.event.event_id() != event_id
            || record.event.context().session_id() != id
        {
            return Err(corruption("journal row identity mismatch"));
        }
        bytes.extend_from_slice(json.as_bytes());
        bytes.push(b'\n');
        count += 1;
        if bytes.len() > MAX_JOURNAL_BYTES || count > MAX_SESSION_RECORDS {
            return Err(corruption("journal resource limit exceeded"));
        }
    }
    let records = parse_frozen_journal(id, &bytes)?;
    if registered {
        let (saved_count, saved_hash, saved_bytes): (i64, Option<String>, i64) = c.query_row(
            "SELECT record_count,last_sha256,byte_length FROM journal_heads WHERE session_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if saved_count != records.len() as i64
            || saved_hash != records.last().map(|r| r.sha256.clone())
            || saved_bytes != bytes.len() as i64
        {
            return Err(corruption("journal accepted extent mismatch"));
        }
    }
    Ok(records)
}

/// Strict parser for frozen bytes. Never opens, repairs or locks the source.
pub fn parse_frozen_journal(
    session_id: &str,
    bytes: &[u8],
) -> Result<Vec<JournalRecord>, DevMapError> {
    if !is_normal_session_component(session_id) || bytes.len() > MAX_JOURNAL_BYTES {
        return Err(corruption("invalid frozen journal identity or size"));
    }
    let (records, complete) = parse_complete_records(bytes)?;
    if complete != bytes.len() || records.len() > MAX_SESSION_RECORDS {
        return Err(corruption("frozen journal incomplete or oversized tail"));
    }
    if records
        .iter()
        .any(|r| r.event.context().session_id() != session_id)
    {
        return Err(corruption("frozen journal session mismatch"));
    }
    Ok(records)
}

/// Physical workspace plus Git administration identity; directory contents/mtime are not identity.
pub(crate) fn worktree_incarnation(workspace: &SourceWorkspace) -> Result<String, DevMapError> {
    Ok(format!(
        "{}|{}",
        checked_directory_identity(&workspace.git_dir)?.stable_text(),
        checked_directory_identity(&workspace.root)?.stable_text()
    ))
}

/// One inventoried immutable session directory. origin_path is the ORIGINAL Git directory,
/// not the directory holding its backup. Include every file, including pending artifacts.
#[derive(Debug, Clone)]
pub struct FrozenJournalSource {
    pub session_id: String,
    pub origin_path: PathBuf,
    pub files: BTreeMap<String, Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct FrozenJournalSnapshot {
    pub session_id: String,
    pub origin_path: PathBuf,
    pub journal_present: bool,
    pub records: Vec<JournalRecord>,
}
/// Parses an already frozen inventory with no filesystem access or source repair.
pub fn parse_frozen_journal_sources(
    sources: &[FrozenJournalSource],
) -> Result<Vec<FrozenJournalSnapshot>, DevMapError> {
    let mut seen = HashSet::new();
    let mut snapshots = Vec::new();
    for source in sources {
        if !seen.insert(source.session_id.clone()) {
            return Err(corruption("duplicate frozen session origin"));
        }
        if !source.origin_path.is_absolute() || !is_normal_session_component(&source.session_id) {
            return Err(corruption("invalid frozen session source identity"));
        }
        for (name, bytes) in &source.files {
            if !matches!(
                name.as_str(),
                "events.ndjson" | "events.index" | "events.lock"
            ) {
                return Err(corruption("pending or unsupported frozen journal artifact"));
            }
            if bytes.len() > MAX_JOURNAL_BYTES {
                return Err(corruption("frozen journal artifact exceeds limit"));
            }
        }
        let journal = source.files.get("events.ndjson");
        let records =
            parse_frozen_journal(&source.session_id, journal.map_or(&[][..], Vec::as_slice))?;
        snapshots.push(FrozenJournalSnapshot {
            session_id: source.session_id.clone(),
            origin_path: source.origin_path.clone(),
            journal_present: journal.is_some(),
            records,
        });
    }
    Ok(snapshots)
}
