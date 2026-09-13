use super::*;
use crate::events::{ActorIdentity, EVENT_SCHEMA_VERSION, EventType, HostIdentity, SessionContext};
use serde_json::json;

fn original_parse_record(line: &[u8], line_number: usize) -> Result<JournalRecord, DevMapError> {
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

fn compare(bytes: &[u8]) {
    let actual = parse_record(bytes, 19).map_err(|e| format!("{e:?}"));
    let expected = original_parse_record(bytes, 19).map_err(|e| format!("{e:?}"));
    assert_eq!(actual, expected);
}

#[test]
fn canonical_tree_reuse_preserves_unsigned_bytes_and_parser_errors() {
    for payload in [
        json!({}),
        json!({"sha256":"nested", "z":[null,true,0,-1,18446744073709551615u64],"é":"\n\"\\", "a":{"z":1,"a":2}}),
    ] {
        for kind in [
            EventType::SessionStarted,
            EventType::SessionStopped,
            EventType::TurnCompleted,
            EventType::InstructionObserved,
            EventType::AgentStarted,
            EventType::AgentStopped,
            EventType::ToolRequested,
            EventType::ToolCompleted,
            EventType::MutationObserved,
            EventType::DecisionRecorded,
            EventType::EvidenceRecorded,
            EventType::ContextCompacting,
            EventType::ContextCompacted,
            EventType::GitActionProposed,
            EventType::GitActionAuthorized,
            EventType::GitActionExecuted,
            EventType::GitActionFailed,
            EventType::AuthorityChanged,
            EventType::CaptureGap,
        ] {
            for previous in [None, Some("a".repeat(64))] {
                let event = EventEnvelope::new(
                    EVENT_SCHEMA_VERSION,
                    "event",
                    kind.clone(),
                    1,
                    "2026-09-08T00:00:00Z",
                    HostIdentity::new("test", "1").unwrap(),
                    ActorIdentity::new("actor", None).unwrap(),
                    SessionContext::new("session", None, "workspace", None, None, None).unwrap(),
                    payload.clone(),
                )
                .unwrap();
                let record = JournalRecord::new(event, previous).unwrap();
                let bytes = canonical_json(&record).unwrap();
                let mut value = serde_json::to_value(&record).unwrap();
                value.sort_all_objects();
                value.as_object_mut().unwrap().remove("sha256");
                assert_eq!(
                    serde_json::to_vec(&value).unwrap(),
                    canonical_json(&UnsignedJournalRecord {
                        sequence: record.sequence,
                        event: &record.event,
                        previous_sha256: &record.previous_sha256
                    })
                    .unwrap()
                );
                compare(&bytes);
                assert_eq!(parse_record(&bytes, 19).unwrap(), record);
                compare(&[b" ".as_slice(), bytes.as_slice()].concat());
                compare(&serde_json::to_vec_pretty(&record).unwrap());
                for (key, replacement) in [
                    ("sha256", json!("wrong")),
                    ("sequence", json!(2)),
                    ("extra", json!(true)),
                    ("previous_sha256", json!(5)),
                    ("event", json!({})),
                ] {
                    let mut value = serde_json::to_value(&record).unwrap();
                    value[key] = replacement;
                    compare(&canonical_json(&value).unwrap());
                    compare(
                        &[b" ".as_slice(), canonical_json(&value).unwrap().as_slice()].concat(),
                    );
                }
                for field in ["sequence", "event", "previous_sha256", "sha256"] {
                    let mut value = serde_json::to_value(&record).unwrap();
                    value.as_object_mut().unwrap().remove(field);
                    compare(&canonical_json(&value).unwrap());
                }
            }
        }
    }
    for bytes in [
        b"null".as_slice(),
        b"{}",
        b"{",
        b"[]",
        b"{\"sequence\":1,\"sequence\":2}",
        b"\xff",
    ] {
        compare(bytes);
    }
    compare(&vec![b' '; MAX_RECORD_BYTES + 1]);
}
