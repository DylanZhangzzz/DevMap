//! Bounded parsing on owned rows from the caller's single SQLite snapshot.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const ROWS: usize = 512;
const INPUT_BYTES: usize = 512 * 1024;
static PARALLEL: AtomicBool = AtomicBool::new(false);

struct Permit<'a>(&'a AtomicBool);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct Parsed {
    pub sequence: u64,
    pub event_sequence: u64,
    pub event_id: String,
    pub session_id: String,
    pub previous_sha256: Option<String>,
    pub sha256: String,
}
pub(super) struct Row {
    pub sequence: i64,
    pub event_id: String,
    pub bytes: usize,
    pub size: i64,
    pub parsed: Result<Parsed, DevMapError>,
}
struct Input {
    sequence: i64,
    event_id: String,
    json: String,
    size: i64,
}

fn parse(input: &Input, line: usize) -> Row {
    let parsed = parse_record(input.json.as_bytes(), line).map(|record| Parsed {
        sequence: record.sequence,
        event_sequence: record.event.sequence(),
        event_id: record.event.event_id().to_owned(),
        session_id: record.event.context().session_id().to_owned(),
        previous_sha256: record.previous_sha256,
        sha256: record.sha256,
    });
    Row {
        sequence: input.sequence,
        event_id: input.event_id.clone(),
        bytes: input.json.len(),
        size: input.size,
        parsed,
    }
}

// Only one batch in the process may create workers. Other callers parse on
// their own thread. Three helpers plus the caller; no detached workers or pool.
fn parse_batch(input: &[Input], start: usize) -> Vec<Row> {
    map_batch(input, start, &PARALLEL, true, &parse)
}

fn map_batch(
    input: &[Input],
    start: usize,
    admission: &AtomicBool,
    spawn_allowed: bool,
    parse: &(impl Fn(&Input, usize) -> Row + Sync),
) -> Vec<Row> {
    let serial = |part: &[Input], offset: usize| {
        part.iter()
            .enumerate()
            .map(|(i, row)| parse(row, start + offset + i + 1))
            .collect::<Vec<_>>()
    };
    if input.len() < 64
        || admission
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
    {
        return serial(input, 0);
    }
    let _permit = Permit(admission);
    std::thread::scope(|scope| {
        let width = input.len().div_ceil(4);
        let mut parts = Vec::new();
        for (i, part) in input.chunks(width).enumerate() {
            let offset = i * width;
            if i == 3 {
                parts.push(Ok(serial(part, offset)));
            } else {
                let spawned = if spawn_allowed {
                    std::thread::Builder::new()
                        .name("devmap-journal".into())
                        .spawn_scoped(scope, move || serial(part, offset))
                } else {
                    Err(std::io::Error::other("injected spawn refusal"))
                };
                match spawned {
                    Ok(handle) => parts.push(Err(handle)),
                    Err(_) => parts.push(Ok(serial(part, offset))),
                }
            }
        }
        let mut output = Vec::with_capacity(input.len());
        let mut panic = None;
        for part in parts {
            match part {
                Ok(rows) => output.extend(rows),
                Err(handle) => match handle.join() {
                    Ok(rows) => output.extend(rows),
                    Err(payload) => {
                        if panic.is_none() {
                            panic = Some(payload);
                        }
                    }
                },
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        output
    })
}

pub(super) fn next(rows: &mut rusqlite::Rows<'_>, start: usize) -> (Vec<Row>, Option<DevMapError>) {
    let mut input = Vec::new();
    let mut bytes = 0;
    let mut error = None;
    // SQL bounds both Strings to MAX_RECORD_BYTES. Stop after the row that
    // reaches the budget: raw input <= INPUT_BYTES + 2 * MAX_RECORD_BYTES.
    while input.len() < ROWS && bytes < INPUT_BYTES {
        let read = (|| -> Result<Option<Input>, DevMapError> {
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(Input {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                json: row.get(2)?,
                size: row.get(3)?,
            }))
        })();
        match read {
            Ok(Some(row)) => {
                bytes += row.event_id.len() + row.json.len();
                input.push(row);
            }
            Ok(None) => break,
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    // Defer a later SQL error until all earlier rows receive full ordered
    // validation. A later malformed row cannot mask an earlier chain failure.
    (parse_batch(&input, start), error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, atomic::AtomicUsize};

    fn input() -> Vec<Input> {
        (0..128)
            .map(|sequence| Input {
                sequence,
                event_id: sequence.to_string(),
                json: "{}".into(),
                size: 2,
            })
            .collect()
    }

    #[test]
    fn workers_and_both_serial_fallbacks_preserve_order_and_release_admission() {
        let input = input();
        for (busy, allowed, threads) in [(false, true, 4), (true, true, 1), (false, false, 1)] {
            let admission = AtomicBool::new(busy);
            let ids = Mutex::new(HashSet::new());
            let output = map_batch(&input, 512, &admission, allowed, &|row, line| {
                ids.lock().unwrap().insert(std::thread::current().id());
                assert_eq!(line, row.sequence as usize + 513);
                parse(row, line)
            });
            assert_eq!(ids.into_inner().unwrap().len(), threads);
            assert_eq!(
                output.iter().map(|row| row.sequence).collect::<Vec<_>>(),
                (0..128).collect::<Vec<_>>()
            );
            assert_eq!(admission.load(Ordering::Acquire), busy);
        }
    }

    #[test]
    fn caller_and_worker_panics_join_other_work_before_releasing_admission() {
        for panic_at in [0, 96] {
            let input = input();
            let admission = AtomicBool::new(false);
            let finished = AtomicUsize::new(0);
            let result = std::panic::catch_unwind(|| {
                map_batch(&input, 0, &admission, true, &|row, line| {
                    assert_ne!(row.sequence, panic_at, "injected parser panic");
                    let result = parse(row, line);
                    finished.fetch_add(1, Ordering::SeqCst);
                    result
                })
            });
            assert!(result.is_err());
            assert_eq!(finished.load(Ordering::SeqCst), 96);
            assert!(!admission.load(Ordering::Acquire));
            assert_eq!(map_batch(&input, 0, &admission, true, &parse).len(), 128);
        }
    }
}
