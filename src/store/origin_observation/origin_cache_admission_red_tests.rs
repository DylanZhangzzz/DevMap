use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::sync_channel,
};
use std::time::Duration;

fn commit_bytes(a: &Admission, mut bytes: usize) {
    while bytes != 0 {
        let n = bytes.min(SCRATCH_BYTES);
        a.reserve_chunk(n)
            .expect("in-budget exact chunk admitted")
            .commit_before_append()
            .unwrap();
        bytes -= n;
        let s = a.snapshot();
        assert!(s.totals.bytes + s.reserved <= MAX_BYTES);
    }
}
#[test]
fn admission_global_exact_and_one_over_and_chunk_misuse() {
    let a = Admission::new();
    assert_eq!(a.finish().unwrap(), AdmissionTotals::default());
    assert!(a.reserve_chunk(0).is_err());
    assert!(a.reserve_chunk(SCRATCH_BYTES + 1).is_err());
    assert_eq!(a.snapshot(), Snapshot::default());
    commit_bytes(&a, MAX_BYTES);
    assert_eq!(a.finish().unwrap().bytes, MAX_BYTES);
    let before = a.snapshot();
    assert!(a.reserve_chunk(1).is_err());
    assert_eq!(a.snapshot(), before);
}
#[test]
fn admission_per_file_exact_one_over_and_global_refusal_never_append() {
    let a = Admission::new();
    let mut bytes = Vec::new();
    for _ in 0..MAX_FILE_BYTES / SCRATCH_BYTES {
        append_checked(&mut bytes, &[9; SCRATCH_BYTES], &a).unwrap();
    }
    assert_eq!(bytes, vec![9; MAX_FILE_BYTES]);
    let before = a.snapshot();
    assert!(append_checked(&mut bytes, &[10], &a).is_err());
    assert_eq!(bytes, vec![9; MAX_FILE_BYTES]);
    assert_eq!(a.snapshot(), before);
    commit_bytes(&a, MAX_BYTES - MAX_FILE_BYTES);
    let before = a.snapshot();
    let mut next = Vec::new();
    assert!(append_checked(&mut next, &[11; SCRATCH_BYTES], &a).is_err());
    assert!(next.is_empty());
    assert_eq!(a.snapshot(), before);
}
#[test]
fn admission_nodes_exact_one_over_and_tree_file_double_charge() {
    let a = Admission::new();
    a.charge_node().unwrap(); // tree entry
    a.charge_node().unwrap(); // visit to that file; collector wiring checked separately
    assert_eq!(a.finish().unwrap().nodes, 2);
    for _ in 2..MAX_NODES {
        a.charge_node().unwrap();
    }
    assert_eq!(a.finish().unwrap().nodes, MAX_NODES);
    let before = a.snapshot();
    assert!(a.charge_node().is_err());
    assert_eq!(a.snapshot(), before);
}
#[test]
fn admission_encoded_path_exact_one_over_and_repeated_call_charge() {
    let a = Admission::new();
    let p = PathBuf::from("repeat");
    let n = p.as_os_str().as_encoded_bytes().len();
    a.charge_path(&p).unwrap();
    a.charge_path(&p).unwrap();
    assert_eq!(a.finish().unwrap().path_bytes, n * 2);
    // Synthetic quota key only, never submitted to a filesystem API.
    a.charge_path(Path::new(&"a".repeat(MAX_PATH_BYTES - 2 * n)))
        .unwrap();
    assert_eq!(a.finish().unwrap().path_bytes, MAX_PATH_BYTES);
    let before = a.snapshot();
    assert!(a.charge_path(Path::new("b")).is_err());
    assert_eq!(a.snapshot(), before);
}
#[test]
fn admission_directory_exact_one_over_unique_keys_separate_from_visits() {
    let a = Admission::new();
    // Synthetic already-checked canonical keys isolate unique-directory quota.
    let keys = (0..MAX_DIRECTORIES)
        .map(|i| PathBuf::from(format!("/quota/{i}")))
        .collect::<Vec<_>>();
    for key in &keys {
        a.admit_directory(key).unwrap();
    }
    assert_eq!(a.finish().unwrap().directories, MAX_DIRECTORIES);
    let before = a.snapshot();
    assert!(a.admit_directory(Path::new("/quota/overflow")).is_err());
    assert_eq!(a.snapshot(), before);
    for _ in 0..2 {
        a.charge_path(&keys[0]).unwrap();
        a.admit_directory(&keys[0]).unwrap();
    }
    let totals = a.finish().unwrap();
    assert_eq!(totals.directories, MAX_DIRECTORIES);
    assert_eq!(
        totals.path_bytes,
        2 * keys[0].as_os_str().as_encoded_bytes().len()
    );
    assert_eq!(totals.nodes, 0);
}
#[test]
fn admission_outstanding_reservation_counts_under_bounded_handshake() {
    let a = Admission::new();
    commit_bytes(&a, MAX_BYTES - SCRATCH_BYTES - 1);
    std::thread::scope(|scope| {
        let (ready, waiting) = sync_channel(1);
        let (release, released) = sync_channel(1);
        let a = &a;
        let worker = scope.spawn(move || {
            let token = a.reserve_chunk(SCRATCH_BYTES).unwrap();
            ready.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(token);
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(a.snapshot().reserved, SCRATCH_BYTES);
        let final_byte = a.reserve_chunk(1).unwrap();
        assert_eq!(a.snapshot().totals.bytes + a.snapshot().reserved, MAX_BYTES);
        let before = a.snapshot();
        assert!(a.reserve_chunk(1).is_err());
        assert_eq!(a.snapshot(), before);
        final_byte.commit_before_append().unwrap();
        assert!(
            a.finish().is_err(),
            "outstanding token prevents publication"
        );
        release.send(()).unwrap();
        worker.join().unwrap();
    });
    assert_eq!(a.snapshot().reserved, 0);
    commit_bytes(&a, SCRATCH_BYTES);
    assert_eq!(a.finish().unwrap().bytes, MAX_BYTES);
}
#[test]
fn admission_drop_and_ordinary_error_refund_only_uncommitted_tokens() {
    let a = Admission::new();
    let token = a.reserve_chunk(23).unwrap();
    assert_eq!(a.snapshot().reserved, 23);
    assert!(a.finish().is_err());
    drop(token);
    assert_eq!(a.finish().unwrap().bytes, 0);
    let reached_error = AtomicBool::new(false);
    let ordinary_error = || -> Result<(), DevMapError> {
        let _token = a.reserve_chunk(31)?;
        assert_eq!(a.snapshot().reserved, 31);
        reached_error.store(true, Ordering::SeqCst);
        Err(fail("injected failure before commit"))
    };
    assert!(ordinary_error().is_err());
    assert!(
        reached_error.load(Ordering::SeqCst),
        "must reach injected ordinary error after reservation"
    );
    assert_eq!(a.snapshot().reserved, 0);
    assert_eq!(a.finish().unwrap().bytes, 0);
    commit_bytes(&a, 41);
    assert_eq!(a.finish().unwrap().bytes, 41);
}
#[test]
fn admission_unwind_before_commit_refunds_after_commit_remains_charged() {
    let a = Admission::new();
    a.finish().unwrap(); // success control prevents stub-only rejection from passing
    let reached_before = AtomicBool::new(false);
    let reached_after = AtomicBool::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _token = a.reserve_chunk(17).unwrap();
        assert_eq!(a.snapshot().reserved, 17);
        reached_before.store(true, Ordering::SeqCst);
        panic!("before commit");
    }));
    assert!(result.is_err());
    assert!(
        reached_before.load(Ordering::SeqCst),
        "must reach injected precommit panic"
    );
    assert_eq!(a.snapshot().reserved, 0);
    assert_eq!(a.finish().unwrap().bytes, 0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        a.reserve_chunk(19).unwrap().commit_before_append().unwrap();
        assert_eq!(a.snapshot().totals.bytes, 19);
        reached_after.store(true, Ordering::SeqCst);
        panic!("after commit before vector append");
    }));
    assert!(result.is_err());
    assert!(
        reached_after.load(Ordering::SeqCst),
        "must reach injected postcommit panic"
    );
    assert_eq!(a.snapshot().reserved, 0);
    assert_eq!(a.snapshot().totals.bytes, 19);
    a.cancel(); // production worker unwind guard must do this; integration requirement
    assert!(a.finish().is_err());
}
#[test]
fn admission_cancellation_rejects_mutation_and_commit_then_refunds() {
    let a = Admission::new();
    commit_bytes(&a, 11);
    let token = a.reserve_chunk(13).unwrap();
    a.cancel();
    assert!(token.commit_before_append().is_err());
    assert_eq!(a.snapshot().reserved, 0);
    assert_eq!(a.snapshot().totals.bytes, 11);
    assert!(a.snapshot().cancelled);
    assert!(a.reserve_chunk(1).is_err());
    assert!(a.charge_node().is_err());
    assert!(a.charge_path(Path::new("x")).is_err());
    assert!(a.admit_directory(Path::new("/x")).is_err());
    assert!(a.finish().is_err());
}
#[test]
fn admission_poisoned_drop_never_panics_or_publishes() {
    let a = Admission::new();
    let token = a.reserve_chunk(29).unwrap();
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = a.state.lock().unwrap();
        panic!("poison");
    }));
    assert!(poisoned.is_err());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(token))).is_ok());
    assert_eq!(a.snapshot().reserved, 0);
    assert!(a.snapshot().cancelled);
    assert!(a.finish().is_err());
    assert!(a.reserve_chunk(1).is_err());
}
#[test]
fn admission_duplicate_file_visits_keep_committed_historical_bytes() {
    let a = Admission::new();
    let mut files = BTreeMap::new();
    for _ in 0..2 {
        let bytes = read_admitted(
            &mut io::Cursor::new(vec![8; 12345]),
            &a,
            &mut [0; SCRATCH_BYTES],
        )
        .unwrap();
        files.insert(PathBuf::from("same-file"), bytes);
    }
    assert_eq!(files.len(), 1);
    assert_eq!(a.finish().unwrap().bytes, 24690);
    drop(files);
    assert_eq!(a.finish().unwrap().bytes, 24690);
}
struct Stream<'a> {
    admission: &'a Admission<'a>,
    remaining: usize,
    max_return: usize,
    delivered: usize,
    requests: Vec<usize>,
    error_at_end: bool,
}
impl Read for Stream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let s = self.admission.snapshot();
        assert_eq!(s.reserved, 0, "resolve current token before next read");
        assert_eq!(
            s.totals.bytes, self.delivered,
            "every prior read committed actual bytes only"
        );
        assert!(!buffer.is_empty() && buffer.len() <= SCRATCH_BYTES);
        assert!(buffer.len() <= MAX_FILE_BYTES.saturating_sub(self.delivered) + 1);
        self.requests.push(buffer.len());
        if self.remaining == 0 && self.error_at_end {
            return Err(io::Error::other("injected I/O error"));
        }
        let n = self.remaining.min(self.max_return).min(buffer.len());
        buffer[..n].fill(7);
        self.remaining -= n;
        self.delivered += n;
        Ok(n)
    }
}
#[test]
fn admission_read_short_actual_chunks_exact_eof_and_growth_sentinel() {
    for (len, short, success) in [
        (0, 3, true),
        (37, 3, true),
        (MAX_FILE_BYTES, SCRATCH_BYTES, true),
        (MAX_FILE_BYTES + 1, SCRATCH_BYTES, false),
    ] {
        let a = Admission::new();
        let mut stream = Stream {
            admission: &a,
            remaining: len,
            max_return: short,
            delivered: 0,
            requests: Vec::new(),
            error_at_end: false,
        };
        let result = read_admitted(&mut stream, &a, &mut [0; SCRATCH_BYTES]);
        if success {
            assert_eq!(result.unwrap(), vec![7; len]);
        } else {
            assert!(result.is_err());
        }
        assert_eq!(stream.remaining, 0);
        assert!(!stream.requests.is_empty());
        if len >= MAX_FILE_BYTES {
            assert_eq!(stream.requests.last(), Some(&1));
        }
        assert_eq!(a.snapshot().totals.bytes, len.min(MAX_FILE_BYTES));
        assert_eq!(a.snapshot().reserved, 0);
    }
}
#[test]
fn admission_read_error_after_committed_short_chunks_returns_no_partial_success() {
    let a = Admission::new();
    let mut stream = Stream {
        admission: &a,
        remaining: 11,
        max_return: 3,
        delivered: 0,
        requests: Vec::new(),
        error_at_end: true,
    };
    let result = read_admitted(&mut stream, &a, &mut [0; SCRATCH_BYTES]);
    assert!(result.is_err());
    assert_eq!(stream.delivered, 11);
    assert_eq!(a.snapshot().totals.bytes, 11);
    assert_eq!(a.snapshot().reserved, 0);
}
#[test]
fn admission_read_interrupted_retries_before_bytes_and_eof() {
    struct InterruptedThenBytes {
        step: usize,
    }
    impl Read for InterruptedThenBytes {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.step += 1;
            match self.step {
                1 | 3 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                2 => {
                    buffer[..3].copy_from_slice(b"abc");
                    Ok(3)
                }
                4 => Ok(0),
                _ => panic!("unexpected extra read after EOF"),
            }
        }
    }
    let a = Admission::new();
    let mut reader = InterruptedThenBytes { step: 0 };
    let bytes = read_admitted(&mut reader, &a, &mut [0; SCRATCH_BYTES]).unwrap();
    assert_eq!(bytes, b"abc");
    assert_eq!(reader.step, 4);
    assert_eq!(a.finish().unwrap().bytes, 3);
    assert_eq!(a.snapshot().reserved, 0);
}
#[test]
fn admission_read_exact_global_eof_then_next_actual_byte_refused() {
    let a = Admission::new();
    for _ in 0..4 {
        let result = read_admitted(
            &mut io::Cursor::new(vec![4; MAX_FILE_BYTES]),
            &a,
            &mut [0; SCRATCH_BYTES],
        )
        .unwrap();
        assert_eq!(result, vec![4; MAX_FILE_BYTES]);
    }
    assert_eq!(a.finish().unwrap().bytes, MAX_BYTES);
    assert!(
        read_admitted(
            &mut io::Cursor::new(Vec::<u8>::new()),
            &a,
            &mut [0; SCRATCH_BYTES]
        )
        .unwrap()
        .is_empty()
    );
    let before = a.snapshot();
    assert!(read_admitted(&mut io::Cursor::new(vec![5]), &a, &mut [0; SCRATCH_BYTES]).is_err());
    assert_eq!(a.snapshot(), before);
}
