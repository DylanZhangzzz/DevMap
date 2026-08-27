use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

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
        let records = self.replay()?;
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
        if records
            .iter()
            .any(|record| record.event.event_id() == event.event_id())
        {
            return Err(corruption(format!(
                "duplicate event ID {}",
                event.event_id()
            )));
        }

        let previous_sha256 = records.last().map(|record| record.sha256.clone());
        let record = JournalRecord::new(event, previous_sha256)?;
        let bytes = canonical_json(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(record)
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
