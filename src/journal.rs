use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

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
    path: PathBuf,
    file: Option<File>,
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
        let existing = self.replay()?;
        let events = build(existing.len() as u64 + 1)?;
        self.append_locked(existing, events)
    }

    pub fn replay(&self) -> Result<Vec<JournalRecord>, DevMapError> {
        let path = self.events_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = BufReader::new(File::open(path)?);
        let mut records = Vec::new();
        let mut event_ids = HashSet::new();
        let mut previous_sha256 = None;

        let mut line = Vec::new();
        let mut line_number = 0;
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            line_number += 1;
            if line.last() != Some(&b'\n') {
                return Err(corruption(format!(
                    "record at line {line_number} is missing its terminating newline"
                )));
            }
            let raw_record = &line[..line.len() - 1];
            if raw_record.is_empty() {
                return Err(corruption(format!("empty record at line {line_number}")));
            }
            let record: JournalRecord = serde_json::from_slice(raw_record).map_err(|error| {
                corruption(format!("malformed JSON at line {line_number}: {error}"))
            })?;
            if raw_record != canonical_json(&record)?.as_slice() {
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

        Ok(records)
    }

    fn events_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.ndjson")
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(&self.session_id).join("events.lock")
    }

    fn acquire_append_lock(&self) -> Result<JournalAppendLock, DevMapError> {
        let path = self.lock_path();
        for _ in 0..200 {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => {
                    return Ok(JournalAppendLock {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(DevMapError::JournalLockTimeout(path))
    }

    fn append_locked(
        &self,
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

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        for record in &appended {
            file.write_all(&canonical_json(record)?)?;
            file.write_all(b"\n")?;
        }
        file.sync_data()?;
        Ok(appended)
    }
}

impl Drop for JournalAppendLock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
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
