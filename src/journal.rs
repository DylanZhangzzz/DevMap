use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::canonical::{canonical_json, sha256_hex};
use crate::error::DevMapError;
use crate::events::EventEnvelope;
use crate::git::SourceWorkspace;

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
}

struct JournalAppendLock {
    file: File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalIntent {
    events: Vec<EventEnvelope>,
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
            return Err(DevMapError::JournalCorruption(
                "session ID must be a non-empty path component".to_owned(),
            ));
        }

        let root = workspace.git_dir.join("devmap").join("sessions");
        fs::create_dir_all(root.join(session_id))?;

        Ok(Self {
            root,
            session_id: session_id.to_owned(),
        })
    }

    pub fn append(&self, event: EventEnvelope) -> Result<JournalRecord, DevMapError> {
        let mut records = self.append_batch_with(|_| Ok(vec![event]))?;
        Ok(records.remove(0))
    }

    pub fn append_batch_with<F>(&self, build: F) -> Result<Vec<JournalRecord>, DevMapError>
    where
        F: FnOnce(u64) -> Result<Vec<EventEnvelope>, DevMapError>,
    {
        let _lock = self.acquire_append_lock()?;
        self.recover_intent_locked()?;
        let existing = self.replay()?;
        let events = build(existing.len() as u64 + 1)?;
        if events.is_empty() {
            return Err(corruption("journal batch must contain at least one event"));
        }
        let appended = prepare_records(existing, events.clone())?;
        self.persist_intent(&events)?;
        self.append_records(&appended)?;
        self.remove_intent()?;
        Ok(appended)
    }

    pub fn replay(&self) -> Result<Vec<JournalRecord>, DevMapError> {
        let path = self.events_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path)?;
        let (records, complete_len) = parse_complete_records(&bytes)?;
        if complete_len != bytes.len() {
            return Err(corruption(format!(
                "record at line {} is missing its terminating newline",
                records.len() + 1
            )));
        }
        Ok(records)
    }

    fn acquire_append_lock(&self) -> Result<JournalAppendLock, DevMapError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())?;
        file.lock_exclusive()?;
        Ok(JournalAppendLock { file })
    }

    fn recover_intent_locked(&self) -> Result<(), DevMapError> {
        let intent_path = self.intent_path();
        if !intent_path.exists() {
            return Ok(());
        }
        let raw_intent = fs::read(&intent_path)?;
        let intent: JournalIntent = serde_json::from_slice(&raw_intent)
            .map_err(|error| corruption(format!("malformed journal intent: {error}")))?;
        if raw_intent != canonical_json(&intent)?.as_slice() {
            return Err(corruption("journal intent is not canonical JSON"));
        }
        if intent.events.is_empty() {
            return Err(corruption("journal intent must contain at least one event"));
        }

        let journal_path = self.events_path();
        let bytes = if journal_path.exists() {
            fs::read(&journal_path)?
        } else {
            Vec::new()
        };
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
            let file = OpenOptions::new().write(true).open(&journal_path)?;
            file.set_len(complete_len as u64)?;
            file.sync_data()?;
        }

        self.append_records(&expected[completed_intent_records..])?;
        self.remove_intent()
    }

    fn persist_intent(&self, events: &[EventEnvelope]) -> Result<(), DevMapError> {
        let bytes = canonical_json(&JournalIntent {
            events: events.to_vec(),
        })?;
        let temporary = self.intent_temporary_path();
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, self.intent_path())?;
        Ok(())
    }

    fn append_records(&self, records: &[JournalRecord]) -> Result<(), DevMapError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        for record in records {
            file.write_all(&canonical_json(record)?)?;
            file.write_all(b"\n")?;
            file.sync_data()?;
        }
        Ok(())
    }

    fn remove_intent(&self) -> Result<(), DevMapError> {
        let path = self.intent_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn events_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.ndjson")
    }

    fn intent_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.intent")
    }

    fn intent_temporary_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.intent.tmp")
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.lock")
    }
}

impl Drop for JournalAppendLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
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

fn parse_complete_records(bytes: &[u8]) -> Result<(Vec<JournalRecord>, usize), DevMapError> {
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut records = Vec::new();
    let mut event_ids = HashSet::new();
    let mut previous_sha256 = None;
    let lines: Vec<_> = bytes[..complete_len].split(|byte| *byte == b'\n').collect();
    for (line_index, line) in lines.iter().enumerate() {
        if line_index + 1 == lines.len() {
            break;
        }
        let line_number = line_index + 1;
        if line.is_empty() {
            return Err(corruption(format!("empty record at line {line_number}")));
        }
        let record: JournalRecord = serde_json::from_slice(line).map_err(|error| {
            corruption(format!("malformed JSON at line {line_number}: {error}"))
        })?;
        if *line != canonical_json(&record)? {
            return Err(corruption(format!(
                "record at line {line_number} is not canonical JSON"
            )));
        }
        let expected_sequence = records.len() as u64 + 1;
        if record.sequence < expected_sequence {
            return Err(DevMapError::DuplicateSequence(record.sequence));
        }
        if record.sequence != expected_sequence {
            return Err(corruption(format!(
                "expected sequence {expected_sequence}, found {} at line {line_number}",
                record.sequence
            )));
        }
        if record.event.sequence() != record.sequence {
            return Err(corruption(format!(
                "event sequence does not match record sequence at line {line_number}"
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
                "previous SHA-256 link mismatch at line {line_number}"
            )));
        }
        if record.sha256 != record.expected_sha256()? {
            return Err(corruption(format!(
                "SHA-256 mismatch at line {line_number}"
            )));
        }
        previous_sha256 = Some(record.sha256.clone());
        records.push(record);
    }
    Ok((records, complete_len))
}

fn is_normal_session_component(session_id: &str) -> bool {
    if session_id.trim().is_empty() || session_id.contains(['/', '\\']) {
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
