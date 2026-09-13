//! Owned-fixture diagnostic only. Parallel parsing is not a journal verifier.
use super::*;
use sha2::Digest;

pub(crate) fn run(c: &rusqlite::Connection, expected: u64) -> Result<(), DevMapError> {
    const MAX_BATCH: usize = 4 * 1024 * 1024;
    const MAX_ROWS: usize = 512;
    let tx = c.unchecked_transaction()?;
    let generation: i64 = tx.query_row(
        "SELECT generation FROM store_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let mut oracle = None;
    for (iteration, workers) in [1usize, 4, 4, 1].into_iter().enumerate() {
        let start = std::time::Instant::now();
        let mut stmt = tx.prepare("SELECT CASE WHEN length(CAST(record_json AS BLOB))<=?1 THEN record_json END FROM journal_records ORDER BY session_id,sequence")?;
        let mut rows = stmt.query([MAX_RECORD_BYTES as i64])?;
        let mut batch = Vec::<String>::new();
        let mut batch_bytes = 0usize;
        let mut peak = 0usize;
        let mut count = 0u64;
        let mut batches = 0usize;
        let mut joined = 0usize;
        let mut digest = sha2::Sha256::new();
        {
            let mut consume = |batch: &[String]| -> Result<(), DevMapError> {
                if batch.is_empty() {
                    return Ok(());
                }
                let parse = |part: &[String]| -> Result<Vec<String>, DevMapError> {
                    part.iter()
                        .enumerate()
                        .map(|(i, json)| {
                            parse_record(json.as_bytes(), i + 1).map(|record| record.sha256)
                        })
                        .collect()
                };
                let parts = if workers == 1 {
                    vec![parse(batch)?]
                } else {
                    std::thread::scope(|scope| -> Result<Vec<Vec<String>>, DevMapError> {
                        let handles: Vec<_> = batch
                            .chunks(batch.len().div_ceil(workers))
                            .map(|part| scope.spawn(move || parse(part)))
                            .collect();
                        let mut results = Vec::new();
                        for handle in handles {
                            let result = handle.join().unwrap();
                            joined += 1;
                            results.push(result?);
                        }
                        Ok(results)
                    })?
                };
                for value in parts.into_iter().flatten() {
                    digest.update(value.as_bytes());
                    count += 1;
                }
                batches += 1;
                Ok(())
            };
            while let Some(row) = rows.next()? {
                let json: String = row.get(0)?;
                if json.len() > MAX_BATCH {
                    return Err(corruption("diagnostic batch record exceeds limit"));
                }
                // One bounded look-ahead record may coexist with the current batch.
                peak = peak.max(batch_bytes + json.len());
                if !batch.is_empty()
                    && (batch.len() == MAX_ROWS || batch_bytes + json.len() > MAX_BATCH)
                {
                    consume(&batch)?;
                    batch.clear();
                    batch_bytes = 0;
                }
                batch_bytes += json.len();
                batch.push(json);
            }
            consume(&batch)?;
        }
        let elapsed = start.elapsed().as_micros();
        assert_eq!(count, expected);
        assert!(peak <= MAX_BATCH + MAX_RECORD_BYTES);
        let digest = format!("{:x}", digest.finalize());
        if let Some(oracle) = &oracle {
            assert_eq!(&digest, oracle);
        } else {
            oracle = Some(digest.clone());
        }
        println!(
            "{}",
            serde_json::json!({"diagnostic":"record-parse-batches/1","iteration":iteration,"workers":workers,"records":count,"batches":batches,"joined":joined,"peak_input_bytes":peak,"digest":digest,"wall_us":elapsed})
        );
    }
    let after: i64 = tx.query_row(
        "SELECT generation FROM store_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(after, generation);
    tx.commit()?;
    Ok(())
}
