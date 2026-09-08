use super::*;
use crate::events::{ActorIdentity, EVENT_SCHEMA_VERSION, EventType, HostIdentity, SessionContext};
use rusqlite::{Connection, params};
use serde_json::json;

fn fixture(count: usize) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(include_str!("store/schema.sql")).unwrap();
    c.execute("INSERT INTO store_meta(singleton,schema_version,repository_id,common_dir) VALUES(1,1,'repo','origin')", []).unwrap();
    let worktree = format!("wt-{}", sha256_hex(b"repo\0origin"));
    c.execute(
        "INSERT INTO worktree_registry VALUES(?1,'incarnation','origin','workspace',NULL)",
        [&worktree],
    )
    .unwrap();
    c.execute(
        "INSERT INTO journal_sessions VALUES('session',?1,'incarnation','origin')",
        [&worktree],
    )
    .unwrap();
    let mut previous = None;
    let mut bytes = 0;
    for sequence in 1..=count {
        let event = EventEnvelope::new(
            EVENT_SCHEMA_VERSION,
            format!("event-{sequence}"),
            EventType::CaptureGap,
            sequence as u64,
            "2026-09-08T00:00:00Z",
            HostIdentity::new("test", "1").unwrap(),
            ActorIdentity::new("actor", None).unwrap(),
            SessionContext::new("session", None, "workspace", None, None, None).unwrap(),
            json!({"reason":"independent fixture"}),
        )
        .unwrap();
        let record = JournalRecord::new(event, previous).unwrap();
        let encoded = canonical_json(&record).unwrap();
        c.execute(
            "INSERT INTO journal_records VALUES('session',?1,?2,?3,?4)",
            params![
                sequence as i64,
                record.event.event_id(),
                String::from_utf8(encoded.clone()).unwrap(),
                encoded.len() as i64
            ],
        )
        .unwrap();
        bytes += encoded.len() + 1;
        previous = Some(record.sha256);
    }
    c.execute(
        "INSERT INTO journal_heads VALUES('session',?1,?2,?3)",
        params![count as i64, previous, bytes as i64],
    )
    .unwrap();
    c
}

fn legacy_summary(c: &Connection, id: &str) -> Result<JournalSummary, DevMapError> {
    let present = sql_session_exists(c, id)?;
    let records = sql_records(c, id)?;
    Ok(JournalSummary {
        session_id: id.into(),
        records: records.len() as u64,
        last_sequence: records.last().map(|r| r.sequence),
        last_sha256: records.last().map(|r| r.sha256.clone()),
        integrity: if present {
            JournalIntegrity::Verified
        } else {
            JournalIntegrity::Missing
        },
    })
}

#[test]
fn streaming_summary_matches_full_verifier_for_missing_empty_and_chains() {
    for count in [0, 1, 37] {
        let c = fixture(count);
        for id in ["session", "absent"] {
            assert_eq!(
                sql_summary(&c, id).unwrap(),
                legacy_summary(&c, id).unwrap()
            );
        }
        assert_eq!(
            sql_summary(&c, "session").unwrap().integrity,
            JournalIntegrity::Verified
        );
        assert_eq!(
            sql_summary(&c, "absent").unwrap().integrity,
            JournalIntegrity::Missing
        );
    }
    let c = fixture(0);
    assert!(sql_summary(&c, "../invalid").is_err());
    assert!(sql_records(&c, "../invalid").is_err());
}

#[test]
fn streaming_summary_rejects_independent_sql_metadata_and_extent_tampering() {
    for sql in [
        "UPDATE journal_records SET byte_length=byte_length+1 WHERE sequence=1",
        "UPDATE journal_records SET event_id='other' WHERE sequence=1",
        "UPDATE journal_records SET sequence=9 WHERE sequence=2",
        "UPDATE journal_records SET record_json=' '||record_json,byte_length=byte_length+1 WHERE sequence=1",
        "UPDATE journal_heads SET record_count=record_count+1",
        "UPDATE journal_heads SET byte_length=byte_length+1",
        "UPDATE journal_heads SET last_sha256=replace(last_sha256,substr(last_sha256,1,1),'z')",
        "DELETE FROM journal_heads",
        "UPDATE journal_sessions SET origin_path='other'",
        "UPDATE worktree_registry SET git_dir='other'",
        "UPDATE store_meta SET repository_id='other'",
        "UPDATE journal_sessions SET origin_path=''",
        "UPDATE worktree_registry SET git_dir=''",
    ] {
        let c = fixture(2);
        c.execute(sql, [])
            .unwrap_or_else(|error| panic!("tamper setup failed for {sql}: {error}"));
        assert!(
            sql_records(&c, "session").is_err(),
            "full verifier accepted: {sql}"
        );
        assert!(
            sql_summary(&c, "session").is_err(),
            "summary accepted: {sql}"
        );
    }
}

#[test]
fn streaming_summary_rejects_canonical_rehashed_chain_and_identity_corruption() {
    for field in [
        "record_sequence",
        "event_sequence",
        "session",
        "previous",
        "hash",
        "duplicate_event",
    ] {
        let c = fixture(2);
        let original: String = c
            .query_row(
                "SELECT record_json FROM journal_records WHERE sequence=2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
        match field {
            "record_sequence" => value["sequence"] = json!(3),
            "event_sequence" => value["event"]["sequence"] = json!(3),
            "session" => value["event"]["context"]["session_id"] = json!("other"),
            "previous" => value["previous_sha256"] = json!("0".repeat(64)),
            "hash" => value["sha256"] = json!("0".repeat(64)),
            "duplicate_event" => value["event"]["event_id"] = json!("event-1"),
            _ => unreachable!(),
        }
        let mut record: JournalRecord = serde_json::from_value(value).unwrap();
        if field != "hash" {
            record.sha256 = record.expected_sha256().unwrap();
        }
        let bytes = canonical_json(&record).unwrap();
        c.execute(
            "UPDATE journal_records SET record_json=?1,byte_length=?2 WHERE sequence=2",
            params![
                String::from_utf8(bytes.clone()).unwrap(),
                bytes.len() as i64
            ],
        )
        .unwrap();
        if field == "record_sequence" {
            // Keep SQL identity consistent to exercise contiguous sequence.
            c.execute("UPDATE journal_records SET sequence=3 WHERE sequence=2", [])
                .unwrap();
        }
        assert!(
            sql_records(&c, "session").is_err(),
            "full verifier accepted {field}"
        );
        assert!(
            sql_summary(&c, "session").is_err(),
            "summary accepted {field}"
        );
    }
}

#[test]
fn streaming_summary_rejects_oversized_sql_record_without_loading_it() {
    let c = fixture(1);
    let json = serde_json::to_string(&"x".repeat(MAX_RECORD_BYTES + 1)).unwrap();
    c.execute(
        "UPDATE journal_records SET record_json=?1,byte_length=?2",
        params![json, json.len() as i64],
    )
    .unwrap();
    assert!(sql_records(&c, "session").is_err());
    assert!(sql_summary(&c, "session").is_err());
}

#[test]
fn streaming_summary_bounds_indexed_event_id_before_owned_allocation() {
    let c = fixture(1);
    c.execute(
        "UPDATE journal_records SET event_id=?1",
        ["x".repeat(MAX_RECORD_BYTES + 1)],
    )
    .unwrap();
    // The small canonical record remains intact; only its independently indexed
    // ID is oversized. SQL returns NULL before constructing an owned String.
    assert!(matches!(
        sql_summary(&c, "session"),
        Err(DevMapError::Sqlite(rusqlite::Error::InvalidColumnType(..)))
    ));
    assert!(sql_records(&c, "session").is_err());
}
