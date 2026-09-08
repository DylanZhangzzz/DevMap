use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::canonical::canonical_json;
use crate::error::DevMapError;
use crate::events::{CaptureGrade, EventType};
use crate::fs_security::{
    FileIdentity, checked_directory_identity, checked_file, checked_metadata, checked_new_file,
    ensure_directory_chain, sync_directory,
};
use crate::git::SourceWorkspace;
use crate::journal::JournalRecord;
use crate::worktrees::{WorktreeScanner, repository_id};

pub const MAX_PRESENCE_BYTES: usize = 64 * 1024;
pub const MAX_PRESENCE_RECORDS: usize = 2_048;
pub const DEFAULT_LEASE_SECONDS: i64 = 120;
const MAX_PRESENCE_STRING_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Starting,
    Working,
    Waiting,
    Idle,
    Completed,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource {
    HostExplicit,
    CaptureEvent,
    Lease,
    GitOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Observed,
    Leased,
    Inferred,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresenceRecord {
    pub schema_version: u8,
    pub repository_id: String,
    pub worktree_id: String,
    pub session_id: String,
    pub actor_id: String,
    pub host: String,
    pub route_id: Option<String>,
    pub branch: Option<String>,
    pub head: String,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: CaptureGrade,
    pub last_event_at: String,
    pub lease_expires_at: Option<String>,
    pub current_activity_id: Option<String>,
    pub current_decision_id: Option<String>,
    pub blocker_count: u32,
    pub gap_count: u32,
}

#[derive(Deserialize)]
#[serde(remote = "PresenceRecord", deny_unknown_fields)]
struct PresenceRecordDef {
    pub schema_version: u8,
    pub repository_id: String,
    pub worktree_id: String,
    pub session_id: String,
    pub actor_id: String,
    pub host: String,
    pub route_id: Option<String>,
    pub branch: Option<String>,
    pub head: String,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: CaptureGrade,
    pub last_event_at: String,
    pub lease_expires_at: Option<String>,
    pub current_activity_id: Option<String>,
    pub current_decision_id: Option<String>,
    pub blocker_count: u32,
    pub gap_count: u32,
}

impl<'de> Deserialize<'de> for PresenceRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let record = PresenceRecordDef::deserialize(deserializer)?;
        validate_record(&record, None).map_err(serde::de::Error::custom)?;
        Ok(record)
    }
}

pub enum PresenceSignal<'a> {
    AcceptedRecords(&'a [JournalRecord]),
    ExplicitWaiting {
        session_id: &'a str,
        activity_id: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceLoadReport {
    pub records: Vec<PresenceRecord>,
    pub warnings: Vec<PresenceWarning>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceWarning {
    pub code: &'static str,
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PresenceStore {
    workspace: SourceWorkspace,
    root: PathBuf,
    root_identity: FileIdentity,
    repository_id: String,
    worktree_id: String,
    branch: Option<String>,
    head: String,
}

impl PresenceStore {
    pub fn open(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        if crate::store::active_existing(workspace)?.is_some() {
            return Self::from_root(workspace, workspace.git_common_dir.clone());
        }
        let root =
            ensure_directory_chain(&workspace.git_common_dir, &["devmap", "presence", "v1"])?;
        Self::from_root(workspace, root)
    }

    pub fn open_existing(workspace: &SourceWorkspace) -> Result<Option<Self>, DevMapError> {
        if crate::store::active_existing(workspace)?.is_some() {
            return Self::from_root(workspace, workspace.git_common_dir.clone()).map(Some);
        }
        let root = workspace.git_common_dir.join("devmap/presence/v1");
        match checked_metadata(&root)? {
            None => Ok(None),
            Some(metadata) if metadata.is_dir() => Self::from_root(workspace, root).map(Some),
            Some(_) => Err(DevMapError::UnsafeInstallerOverwrite(root)),
        }
    }

    fn from_root(workspace: &SourceWorkspace, root: PathBuf) -> Result<Self, DevMapError> {
        let current = WorktreeScanner::scan(workspace)?
            .into_iter()
            .find(|row| row.is_current)
            .ok_or_else(|| invalid("current worktree is missing from Git inventory"))?;
        Ok(Self {
            workspace: workspace.clone(),
            root_identity: checked_directory_identity(&root)?,
            root,
            repository_id: repository_id(workspace),
            worktree_id: current.worktree_id,
            branch: workspace.branch.clone(),
            head: workspace.head.clone(),
        })
    }

    pub fn observe(
        &self,
        signal: PresenceSignal<'_>,
        now: OffsetDateTime,
    ) -> Result<PresenceRecord, DevMapError> {
        crate::store::domain_write(&self.workspace, |tx| {
            if let Some(tx) = tx {
                let (record, changed) = self.observe_sql(tx, signal, now)?;
                if changed {
                    tx.execute(
                        "UPDATE store_meta SET generation=generation+1 WHERE singleton=1",
                        [],
                    )?;
                }
                return Ok(record);
            }
            self.observe_legacy(signal, now)
        })
    }

    pub(crate) fn observe_legacy(
        &self,
        signal: PresenceSignal<'_>,
        now: OffsetDateTime,
    ) -> Result<PresenceRecord, DevMapError> {
        let session_id = match &signal {
            PresenceSignal::AcceptedRecords(records) => records
                .first()
                .ok_or_else(|| invalid("accepted record batch must not be empty"))?
                .event
                .context()
                .session_id(),
            PresenceSignal::ExplicitWaiting { session_id, .. } => session_id,
        };
        check_session_component(session_id)?;
        let _lock = self.acquire_record_lock(session_id)?;
        let record = match signal {
            PresenceSignal::AcceptedRecords(records) => {
                self.project_records(records, now, self.load_one(session_id)?)?
            }
            PresenceSignal::ExplicitWaiting {
                session_id,
                activity_id,
            } => {
                check_session_component(session_id)?;
                let mut record = self
                    .load_one(session_id)?
                    .ok_or_else(|| DevMapError::MissingPresence(session_id.to_owned()))?;
                set_explicit_waiting(&mut record, activity_id, now)?;
                record
            }
        };
        self.persist_locked(&record)?;
        Ok(record)
    }

    pub fn load_all(&self) -> PresenceLoadReport {
        match crate::store::active_existing(&self.workspace) {
            Ok(Some(store)) => return self.load_all_sql(store.connection()),
            Err(_) => {
                return PresenceLoadReport {
                    records: vec![],
                    warnings: vec![PresenceWarning {
                        code: "presence_unreadable",
                        subject_id: None,
                    }],
                    truncated: false,
                };
            }
            Ok(None) => {}
        }
        let mut report = PresenceLoadReport {
            records: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
        };
        if self.validate_root().is_err() {
            report.warnings.push(PresenceWarning {
                code: "presence_root_replaced",
                subject_id: None,
            });
            return report;
        }
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(_) => {
                report.warnings.push(PresenceWarning {
                    code: "presence_unreadable",
                    subject_id: None,
                });
                return report;
            }
        };
        let mut inspected_records = 0usize;
        for entry in entries {
            let Ok(entry) = entry else {
                report.warnings.push(PresenceWarning {
                    code: "presence_entry_unreadable",
                    subject_id: None,
                });
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if inspected_records == MAX_PRESENCE_RECORDS {
                report.truncated = true;
                break;
            }
            inspected_records += 1;
            let subject_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned);
            let Some(expected_session_id) = subject_id.as_deref() else {
                report.warnings.push(PresenceWarning {
                    code: "presence_record_invalid",
                    subject_id: None,
                });
                continue;
            };
            match read_record(&path, &self.repository_id, expected_session_id) {
                Ok(record) => report.records.push(record),
                Err(_) => report.warnings.push(PresenceWarning {
                    code: "presence_record_invalid",
                    subject_id,
                }),
            }
        }
        report.records.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.actor_id.cmp(&right.actor_id))
        });
        report
    }

    fn project_records(
        &self,
        records: &[JournalRecord],
        now: OffsetDateTime,
        previous: Option<PresenceRecord>,
    ) -> Result<PresenceRecord, DevMapError> {
        let first = records
            .first()
            .ok_or_else(|| invalid("accepted record batch must not be empty"))?;
        let session_id = first.event.context().session_id();
        check_session_component(session_id)?;
        if records
            .iter()
            .any(|record| record.event.context().session_id() != session_id)
        {
            return Err(invalid("accepted records span multiple sessions"));
        }
        let mut status = previous.as_ref().map(|record| record.status);
        let mut gap_count = previous.as_ref().map_or(0, |record| record.gap_count);
        let blocker_count = previous.as_ref().map_or(0, |record| record.blocker_count);
        let mut current_activity_id = previous
            .as_ref()
            .and_then(|record| record.current_activity_id.clone());
        let mut current_decision_id = previous
            .as_ref()
            .and_then(|record| record.current_decision_id.clone());
        for record in records {
            status = Some(project_status(status, record.event.event_type()));
            match record.event.event_type() {
                EventType::CaptureGap => gap_count = gap_count.saturating_add(1),
                EventType::ToolRequested => {
                    current_activity_id = Some(record.event.event_id().to_owned())
                }
                EventType::ToolCompleted | EventType::TurnCompleted | EventType::SessionStopped => {
                    current_activity_id = None
                }
                EventType::DecisionRecorded => {
                    current_decision_id = Some(record.event.event_id().to_owned())
                }
                _ => {}
            }
        }
        let last = records.last().expect("non-empty checked above");
        let status = status.unwrap_or(PresenceStatus::Working);
        let completed = status == PresenceStatus::Completed;
        let capture_grade = last
            .event
            .payload()
            .get("capture_grade")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or(CaptureGrade::D);
        Ok(PresenceRecord {
            schema_version: 1,
            repository_id: self.repository_id.clone(),
            worktree_id: self.worktree_id.clone(),
            session_id: session_id.to_owned(),
            actor_id: last.event.actor().agent_id().to_owned(),
            host: last.event.host().name().to_owned(),
            route_id: last.event.context().route_id().map(str::to_owned),
            branch: last
                .event
                .context()
                .branch()
                .map(str::to_owned)
                .or_else(|| self.branch.clone()),
            head: last
                .event
                .context()
                .head()
                .map(str::to_owned)
                .unwrap_or_else(|| self.head.clone()),
            status,
            status_source: if status == PresenceStatus::Waiting {
                StatusSource::HostExplicit
            } else {
                StatusSource::CaptureEvent
            },
            confidence: Confidence::Observed,
            capture_grade,
            last_event_at: last.event.occurred_at().to_owned(),
            lease_expires_at: (!completed)
                .then(|| format_time(now + Duration::seconds(DEFAULT_LEASE_SECONDS)))
                .transpose()?,
            current_activity_id,
            current_decision_id,
            blocker_count,
            gap_count,
        })
    }

    fn load_one(&self, session_id: &str) -> Result<Option<PresenceRecord>, DevMapError> {
        let path = self.record_path(session_id)?;
        if checked_metadata(&path)?.is_none() {
            return Ok(None);
        }
        read_record(&path, &self.repository_id, session_id).map(Some)
    }

    fn acquire_record_lock(&self, session_id: &str) -> Result<std::fs::File, DevMapError> {
        self.validate_root()?;
        let lock_path = self.root.join(format!("{session_id}.lock"));
        let existed = checked_metadata(&lock_path)?.is_some();
        let lock = checked_file(&lock_path, true, true)?;
        if !existed {
            sync_directory(&self.root)?;
        }
        crate::store::lock_domain_file(&lock)?;
        self.validate_root()?;
        Ok(lock)
    }

    fn persist_locked(&self, record: &PresenceRecord) -> Result<(), DevMapError> {
        validate_record(record, Some(&self.repository_id))?;
        let bytes = canonical_json(record)?;
        if bytes.len() > MAX_PRESENCE_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "Presence record",
                limit: MAX_PRESENCE_BYTES,
            });
        }
        let path = self.record_path(&record.session_id)?;
        self.validate_root()?;

        let temporary = self.root.join(format!(
            ".{}.tmp-{}-{}",
            record.session_id,
            std::process::id(),
            now_nanos()
        ));
        let result = (|| {
            let mut file = checked_new_file(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_data()?;
            atomic_replace(&temporary, &path)?;
            sync_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn record_path(&self, session_id: &str) -> Result<PathBuf, DevMapError> {
        check_session_component(session_id)?;
        Ok(self.root.join(format!("{session_id}.json")))
    }

    fn validate_root(&self) -> Result<(), DevMapError> {
        if checked_directory_identity(&self.root)? != self.root_identity {
            return Err(DevMapError::UnsafeInstallerOverwrite(self.root.clone()));
        }
        Ok(())
    }
}

impl PresenceRecord {
    pub fn effective_at(&self, now: OffsetDateTime) -> PresenceRecord {
        let mut effective = self.clone();
        if self.status != PresenceStatus::Completed
            && self
                .lease_expires_at
                .as_deref()
                .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
                .is_some_and(|expires| now > expires)
        {
            effective.status = PresenceStatus::Stale;
            effective.status_source = StatusSource::Lease;
            effective.confidence = Confidence::Leased;
        }
        effective
    }
}

pub fn project_status(previous: Option<PresenceStatus>, event_type: &EventType) -> PresenceStatus {
    match event_type {
        EventType::SessionStarted => PresenceStatus::Starting,
        EventType::TurnCompleted => PresenceStatus::Idle,
        EventType::SessionStopped => PresenceStatus::Completed,
        EventType::ToolRequested
        | EventType::ToolCompleted
        | EventType::MutationObserved
        | EventType::DecisionRecorded
        | EventType::EvidenceRecorded
        | EventType::AgentStarted
        | EventType::AgentStopped
        | EventType::ContextCompacting
        | EventType::ContextCompacted => PresenceStatus::Working,
        _ => previous.unwrap_or(PresenceStatus::Working),
    }
}

fn read_record(
    path: &Path,
    repository_id: &str,
    expected_session_id: &str,
) -> Result<PresenceRecord, DevMapError> {
    let metadata = checked_metadata(path)?
        .ok_or_else(|| DevMapError::MissingPresence(path.to_string_lossy().into_owned()))?;
    if !metadata.is_file() || metadata.len() as usize > MAX_PRESENCE_BYTES {
        return Err(invalid("Presence file type or size is invalid"));
    }
    let file = checked_file(path, false, false)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_PRESENCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PRESENCE_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "Presence record",
            limit: MAX_PRESENCE_BYTES,
        });
    }
    let record: PresenceRecord = serde_json::from_slice(&bytes)?;
    validate_record(&record, Some(repository_id))?;
    if record.session_id != expected_session_id {
        return Err(invalid("session_id does not match the Presence filename"));
    }
    if bytes != canonical_json(&record)? {
        return Err(invalid("Presence record is not canonical JSON"));
    }
    Ok(record)
}

fn validate_record(
    record: &PresenceRecord,
    expected_repository_id: Option<&str>,
) -> Result<(), DevMapError> {
    if record.schema_version != 1 {
        return Err(invalid("unsupported schema version"));
    }
    for (name, value) in [
        ("repository_id", record.repository_id.as_str()),
        ("worktree_id", record.worktree_id.as_str()),
        ("session_id", record.session_id.as_str()),
        ("actor_id", record.actor_id.as_str()),
        ("host", record.host.as_str()),
        ("head", record.head.as_str()),
        ("last_event_at", record.last_event_at.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > MAX_PRESENCE_STRING_BYTES {
            return Err(invalid(format!("{name} is blank or too long")));
        }
    }
    if record
        .route_id
        .as_deref()
        .is_some_and(|value| !is_safe_route_id(value))
    {
        return Err(invalid("route_id contains unsafe characters"));
    }
    check_prefixed_hex(&record.repository_id, "sha256-", 64, "repository_id")?;
    check_prefixed_hex(&record.worktree_id, "wt-", 64, "worktree_id")?;
    check_lower_hex(&record.head, &[40, 64], "head")?;
    check_session_component(&record.session_id)?;
    parse_time(&record.last_event_at, "last_event_at")?;
    if let Some(value) = &record.lease_expires_at {
        parse_time(value, "lease_expires_at")?;
    }
    for (name, value) in [
        ("route_id", record.route_id.as_deref()),
        ("branch", record.branch.as_deref()),
        ("current_activity_id", record.current_activity_id.as_deref()),
        ("current_decision_id", record.current_decision_id.as_deref()),
    ] {
        if value
            .is_some_and(|value| value.trim().is_empty() || value.len() > MAX_PRESENCE_STRING_BYTES)
        {
            return Err(invalid(format!("{name} is blank or too long")));
        }
    }
    if expected_repository_id.is_some_and(|expected| record.repository_id != expected) {
        return Err(invalid(
            "repository_id does not match the current repository",
        ));
    }
    if record.status == PresenceStatus::Completed
        && (record.status_source == StatusSource::Lease || record.lease_expires_at.is_some())
    {
        return Err(invalid("completed status cannot be leased"));
    }
    if record.status_source == StatusSource::Lease
        && (record.status != PresenceStatus::Stale || record.confidence != Confidence::Leased)
    {
        return Err(invalid("lease source requires stale leased status"));
    }
    if record.status_source == StatusSource::GitOnly && record.status != PresenceStatus::Unknown {
        return Err(invalid("Git-only source requires unknown status"));
    }
    if record.status == PresenceStatus::Waiting
        && record.status_source != StatusSource::HostExplicit
    {
        return Err(invalid("waiting status must be explicit"));
    }
    if record.status == PresenceStatus::Unknown
        && (record.status_source != StatusSource::GitOnly
            || record.confidence != Confidence::Unknown)
    {
        return Err(invalid(
            "unknown status must be Git-only and unknown confidence",
        ));
    }
    Ok(())
}

fn check_session_component(value: &str) -> Result<(), DevMapError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid("session_id must be one bounded path component"));
    }
    Ok(())
}

fn is_safe_route_id(value: &str) -> bool {
    value.len() <= 512
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || (index > 0 && matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
}

fn check_prefixed_hex(
    value: &str,
    prefix: &str,
    digits: usize,
    name: &str,
) -> Result<(), DevMapError> {
    let Some(hex) = value.strip_prefix(prefix) else {
        return Err(invalid(format!("{name} has an invalid prefix")));
    };
    check_lower_hex(hex, &[digits], name)
}

fn check_lower_hex(value: &str, lengths: &[usize], name: &str) -> Result<(), DevMapError> {
    if !lengths.contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(format!("{name} must be lowercase hexadecimal")));
    }
    Ok(())
}

fn parse_time(value: &str, name: &str) -> Result<OffsetDateTime, DevMapError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| invalid(format!("{name} must be RFC 3339")))
}

fn format_time(value: OffsetDateTime) -> Result<String, DevMapError> {
    value.format(&Rfc3339).map_err(DevMapError::from)
}

fn now_nanos() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos()
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), DevMapError> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), DevMapError> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated and remain alive for the duration of the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> DevMapError {
    DevMapError::InvalidPresence(message.into())
}

impl PresenceStore {
    pub(crate) fn for_projection(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        Self::from_root(workspace, workspace.git_common_dir.clone())
    }
    pub(crate) fn observe_sql(
        &self,
        tx: &rusqlite::Transaction<'_>,
        signal: PresenceSignal<'_>,
        now: OffsetDateTime,
    ) -> Result<(PresenceRecord, bool), DevMapError> {
        use rusqlite::OptionalExtension;
        let id = match &signal {
            PresenceSignal::AcceptedRecords(records) => records
                .first()
                .ok_or_else(|| invalid("empty projection"))?
                .event
                .context()
                .session_id(),
            PresenceSignal::ExplicitWaiting { session_id, .. } => session_id,
        };
        check_session_component(id)?;
        let origin: (String, String) = tx.query_row(
            "SELECT worktree_id,incarnation FROM journal_sessions WHERE session_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if origin.0 != self.worktree_id
            || origin.1 != crate::journal::worktree_incarnation(&self.workspace)?
        {
            return Err(invalid("presence session origin mismatch"));
        }
        let accepted = crate::journal::sql_records(tx, id)?;
        let previous = self.load_one_sql(tx, id)?;
        let projection_advanced = matches!(&signal, PresenceSignal::AcceptedRecords(_));
        let record = match signal {
            PresenceSignal::AcceptedRecords(records) => {
                let cursor:Option<(i64,Option<String>)>=tx.query_row("SELECT covered_sequence,covered_sha256 FROM presence_projection WHERE session_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                let covered = cursor.as_ref().map_or(0, |v| v.0);
                if let Some((sequence, hash)) = &cursor {
                    if *sequence > 0 {
                        let saved:String=tx.query_row("SELECT record_json FROM journal_records WHERE session_id=?1 AND sequence=?2",rusqlite::params![id,sequence],|r|r.get(0))?;
                        let saved: JournalRecord = serde_json::from_str(&saved)?;
                        if Some(&saved.sha256) != hash.as_ref() {
                            return Err(invalid("projection cursor hash mismatch"));
                        }
                    }
                    if previous.is_none() {
                        return Err(invalid("projection cursor has no presence"));
                    }
                }
                let mut fresh = Vec::new();
                for record in records {
                    if record.event.context().session_id() != id || record.sequence == 0 {
                        return Err(invalid("projection records must belong to one session"));
                    }
                    let saved:String=tx.query_row("SELECT record_json FROM journal_records WHERE session_id=?1 AND sequence=?2",rusqlite::params![id,record.sequence as i64],|r|r.get(0))?;
                    if saved.as_bytes() != canonical_json(record)? {
                        return Err(invalid("projection record differs from accepted journal"));
                    }
                    if accepted.get(record.sequence as usize - 1) != Some(record) {
                        return Err(invalid("invalid accepted projection record"));
                    }
                    if record.sequence as i64 > covered {
                        fresh.push(record.clone());
                    }
                }
                if fresh.is_empty() {
                    return previous
                        .map(|r| (r, false))
                        .ok_or_else(|| invalid("duplicate projection missing presence"));
                }
                fresh.sort_by_key(|record| record.sequence);
                fresh.dedup_by_key(|record| record.sequence);
                let projected = self.project_records(&fresh, now, previous)?;
                let last = fresh.last().unwrap();
                tx.execute("INSERT INTO presence_projection(session_id,covered_sequence,covered_sha256,baseline_source) VALUES(?1,?2,?3,'capture') ON CONFLICT(session_id) DO UPDATE SET covered_sequence=excluded.covered_sequence,covered_sha256=excluded.covered_sha256",rusqlite::params![id,last.sequence as i64,last.sha256])?;
                projected
            }
            PresenceSignal::ExplicitWaiting { activity_id, .. } => {
                let mut record =
                    previous.ok_or_else(|| DevMapError::MissingPresence(id.to_owned()))?;
                set_explicit_waiting(&mut record, activity_id, now)?;
                record
            }
        };
        validate_record(&record, Some(&self.repository_id))?;
        let bytes = canonical_json(&record)?;
        if bytes.len() > MAX_PRESENCE_BYTES {
            return Err(DevMapError::ResourceLimit {
                resource: "Presence record",
                limit: MAX_PRESENCE_BYTES,
            });
        }
        let old: Option<String> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(record_json AS BLOB))<=?2 THEN record_json END FROM presence_records WHERE session_id=?1",
                rusqlite::params![id,MAX_PRESENCE_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?;
        let changed = projection_advanced || old.as_ref().is_none_or(|v| v.as_bytes() != bytes);
        tx.execute("INSERT INTO presence_records(session_id,record_json) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET record_json=excluded.record_json",rusqlite::params![id,String::from_utf8(bytes).map_err(|_|invalid("invalid utf8"))?])?;
        Ok((record, changed))
    }
    fn load_one_sql(
        &self,
        c: &rusqlite::Connection,
        id: &str,
    ) -> Result<Option<PresenceRecord>, DevMapError> {
        use rusqlite::OptionalExtension;
        let json: Option<String> = c
            .query_row(
                "SELECT CASE WHEN length(CAST(record_json AS BLOB))<=?2 THEN record_json END FROM presence_records WHERE session_id=?1",
                rusqlite::params![id,MAX_PRESENCE_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?;
        json.map(|json| parse_frozen_presence(&self.repository_id, id, json.as_bytes()))
            .transpose()
    }
    fn load_all_sql(&self, c: &rusqlite::Connection) -> PresenceLoadReport {
        let mut report = PresenceLoadReport {
            records: vec![],
            warnings: vec![],
            truncated: false,
        };
        let result = (|| -> Result<(), DevMapError> {
            let mut stmt = c.prepare(
                "SELECT session_id,CASE WHEN length(CAST(record_json AS BLOB))<=?2 THEN record_json END FROM presence_records ORDER BY session_id LIMIT ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![
                (MAX_PRESENCE_RECORDS + 1) as i64,
                MAX_PRESENCE_BYTES as i64
            ])?;
            let mut count = 0;
            while let Some(row) = rows.next()? {
                if count == MAX_PRESENCE_RECORDS {
                    report.truncated = true;
                    break;
                }
                count += 1;
                let id: String = row.get(0)?;
                let json: Option<String> = row.get(1)?;
                match json
                    .ok_or_else(|| invalid("presence exceeds byte limit"))
                    .and_then(|json| {
                        parse_frozen_presence(&self.repository_id, &id, json.as_bytes())
                    }) {
                    Ok(r) => report.records.push(r),
                    Err(_) => report.warnings.push(PresenceWarning {
                        code: "presence_record_invalid",
                        subject_id: Some(id),
                    }),
                }
            }
            Ok(())
        })();
        if result.is_err() {
            report.warnings.push(PresenceWarning {
                code: "presence_unreadable",
                subject_id: None,
            });
        }
        report
    }
}

/// Parses saved canonical bytes using the original repository identity and filename session.
/// The caller owns frozen-source inventory and pending-file detection.
pub fn parse_frozen_presence(
    repository_id: &str,
    session_id: &str,
    bytes: &[u8],
) -> Result<PresenceRecord, DevMapError> {
    if bytes.len() > MAX_PRESENCE_BYTES {
        return Err(invalid("presence exceeds byte limit"));
    }
    let record: PresenceRecord = serde_json::from_slice(bytes)?;
    validate_record(&record, Some(repository_id))?;
    if record.session_id != session_id || canonical_json(&record)? != bytes {
        return Err(invalid("presence identity or canonical JSON mismatch"));
    }
    Ok(record)
}

fn set_explicit_waiting(
    record: &mut PresenceRecord,
    activity_id: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), DevMapError> {
    record.status = PresenceStatus::Waiting;
    record.status_source = StatusSource::HostExplicit;
    record.confidence = Confidence::Observed;
    record.last_event_at = format_time(now)?;
    record.lease_expires_at = Some(format_time(now + Duration::seconds(DEFAULT_LEASE_SECONDS))?);
    record.current_activity_id = activity_id.map(str::to_owned);
    Ok(())
}
