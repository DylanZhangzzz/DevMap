use std::collections::HashSet;
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

pub const MAX_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SESSION_RECORDS: usize = 100_000;
const MAX_INTENT_BYTES: usize = 1024 * 1024;
const MAX_INDEX_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = MAX_EVENT_BYTES + 16 * 1024;
const INDEX_VERSION: u8 = 1;
const BLOOM_WORDS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub sequence: u64,
    pub event: EventEnvelope,
    pub previous_sha256: Option<String>,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct JournalStore {
    root: PathBuf,
    session_id: String,
    root_identity: FileIdentity,
    session_identity: FileIdentity,
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

        let session_root =
            ensure_directory_chain(&workspace.git_dir, &["devmap", "sessions", session_id])?;
        let root = session_root
            .parent()
            .ok_or_else(|| corruption("session root has no parent"))?
            .to_path_buf();
        let root_identity = checked_directory_identity(&root)?;
        let session_identity = checked_directory_identity(&session_root)?;
        Ok(Self {
            root,
            session_id: session_id.to_owned(),
            root_identity,
            session_identity,
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
        file.lock_exclusive()?;
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

fn is_normal_session_component(session_id: &str) -> bool {
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
