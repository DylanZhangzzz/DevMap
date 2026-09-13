use super::*;
const STAMP: &str = "2026-09-11T10:00:00Z";

fn git(root: &Path, args: &[&str]) {
    let output = crate::git_process::output(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct Fixture {
    _owned: tempfile::TempDir,
    workspace: SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    changed_file: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let owned = tempfile::tempdir().unwrap();
        let main = owned.path().join("main");
        let linked = owned.path().join("linked");
        fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        git(
            &main,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "base",
            ],
        );
        git(
            &main,
            &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
        );
        let workspace = SourceGitInspector::open(&main)
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let mut origins = Vec::new();
        for root in [&main, &linked] {
            let source = SourceGitInspector::open(root)
                .unwrap()
                .workspace_allow_unborn()
                .unwrap();
            for (index, bytes) in [
                Vec::new(),
                vec![0, 255, 0, 13, 10],
                "完整字节🙂\n".repeat(10000).into_bytes(),
                b"quota-original".to_vec(),
            ]
            .into_iter()
            .enumerate()
            {
                // Inventory validates complete bytes and known paths, not event parsing.
                let directory = source.git_dir.join(format!("devmap/sessions/s{index}"));
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("events.ndjson"), bytes).unwrap();
            }
            origins.push(FrozenOrigin {
                worktree_id: worktrees::origin_id(
                    &worktrees::repository_id(&source),
                    &source.git_dir,
                ),
                incarnation: journal::worktree_incarnation(&source).unwrap(),
                git_dir: source.git_dir,
                workspace_path: source.root,
            });
        }
        origins.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
        let changed_file = workspace.git_dir.join("devmap/sessions/s3/events.ndjson");
        Self {
            _owned: owned,
            workspace,
            origins,
            changed_file,
        }
    }
    fn serial(&self) -> FrozenManifest {
        inventory_serial(&self.workspace, self.origins.clone(), STAMP.into()).unwrap()
    }
}

#[test]
fn real_multi_origin_candidate_matches_complete_serial_manifest_and_joins_four_workers() {
    let fixture = Fixture::new();
    let expected = fixture.serial();
    assert_eq!(expected.origins.len(), 2);
    assert_eq!(expected.files.len(), 8);
    let observation = Observation::default();
    let actual = candidate(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    )
    .unwrap()
    .expect("bounded parallel candidate must be produced for ordinary multi-origin inventory");
    assert_eq!(actual, expected);
    observation.drained();
    assert_eq!(observation.started.load(Ordering::SeqCst), 4);
    assert_eq!(
        observation.hashed.load(Ordering::SeqCst),
        expected.files.len()
    );
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.serial(), expected);
}

#[test]
fn growth_and_shrink_at_target_job_join_before_one_serial_oracle() {
    for replacement in [b"quota-original-grown".as_slice(), b"x".as_slice()] {
        let fixture = Fixture::new();
        let bytes = replacement.to_vec();
        let (hooks, observation) =
            historical_observation(&fixture, vec![fixture.changed_file.clone()], move |path| {
                fs::write(path, &bytes).unwrap()
            });
        let actual = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {},
        )
        .unwrap();
        assert_historical_target_failures(&hooks, &observation);
        assert!(observation.started.load(Ordering::SeqCst) > 0);
        assert_eq!(actual, fixture.serial());
    }
}

#[test]
fn small_speculative_quota_reads_only_admissible_prefix_before_one_oracle() {
    assert_historical_late_prefix(Limits {
        bytes: 7,
        files: 2,
        directories: 4,
    });
}

#[test]
fn checked_target_job_error_joins_before_original_serial_error() {
    let fixture = Fixture::new();
    let alias = fixture._owned.path().join("retained-alias");
    assert_owned_destination(&fixture, &alias);
    let (hooks, observation) =
        historical_observation(&fixture, vec![fixture.changed_file.clone()], move |path| {
            fs::hard_link(path, &alias).unwrap()
        });
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    let expected =
        inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into()).unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), expected.to_string());
    assert_historical_target_failures(&hooks, &observation);
    assert!(observation.started.load(Ordering::SeqCst) > 0);
}

#[test]
fn worker_panic_joins_every_started_thread_without_serial_retry() {
    let fixture = Fixture::new();
    let observation = Observation::default();
    observation.panic_at.store(0, Ordering::SeqCst);
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "repository store: parallel inventory worker panicked"
    );
    observation.drained();
    assert_eq!(observation.started.load(Ordering::SeqCst), 4);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    // Cancellation belongs only to the failed operation.
    let fresh = Observation::default();
    let result = candidate(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &fresh,
        || {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(result, fixture.serial());
    fresh.drained();
    assert_eq!(fresh.hashed.load(Ordering::SeqCst), result.files.len());
}

#[test]
fn duplicate_ordinal_never_returns_a_partial_or_blank_digest_manifest() {
    let fixture = Fixture::new();
    let observation = Observation::default();
    observation.duplicate_result.store(true, Ordering::SeqCst);
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "repository store: parallel inventory result ordinal duplicate"
    );
    observation.drained();
    assert_eq!(
        observation.hashed.load(Ordering::SeqCst),
        fixture.serial().files.len()
    );
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
}

#[test]
fn exact_aggregate_and_late_prefix_entry_limits_preserve_admission() {
    let fixture = Fixture::new();
    let expected = fixture.serial();
    let bytes = expected.files.iter().map(|file| file.bytes).sum();
    let exact = Limits {
        bytes,
        files: expected.files.len(),
        directories: expected.directories.len(),
    };
    let observation = Observation::default();
    let actual = candidate(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        exact,
        &observation,
        || {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual, expected);
    observation.drained();
    // Intentional contract replacement: later refusal may follow a completed
    // admissible read. First-inadmissible zero-I/O tests remain unchanged.
    for limits in [
        Limits {
            bytes: 7,
            files: 2,
            directories: 4,
        },
        Limits {
            bytes: 8,
            files: 1,
            directories: 4,
        },
        Limits {
            bytes: 8,
            files: 2,
            directories: 2,
        },
    ] {
        assert_historical_late_prefix(limits);
    }
}

#[test]
fn both_origin_target_jobs_refuse_hardlinks_before_one_serial_oracle() {
    let fixture = Fixture::new();
    let targets: Vec<_> = fixture
        .origins
        .iter()
        .map(|origin| origin.git_dir.join("devmap/sessions/s3/events.ndjson"))
        .collect();
    let aliases: Vec<_> = targets
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let alias = fixture
                ._owned
                .path()
                .join(format!("retained-alias-{index}"));
            assert_owned_destination(&fixture, &alias);
            (path.clone(), alias)
        })
        .collect();
    // historical_observation holds BOTH selected jobs before either action.
    let (hooks, observation) = historical_observation(&fixture, targets, move |path| {
        let alias = &aliases.iter().find(|(target, _)| target == path).unwrap().1;
        fs::hard_link(path, alias).unwrap();
    });
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    let direct =
        inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into()).unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), direct.to_string());
    assert_historical_target_failures(&hooks, &observation);
}

#[cfg(windows)]
#[test]
fn owned_windows_junction_before_target_checked_open_refuses_and_preserves_target() {
    use std::os::windows::process::CommandExt;
    let fixture = Fixture::new();
    let owned = fs::canonicalize(fixture._owned.path()).unwrap();
    let directory = fixture.changed_file.parent().unwrap();
    assert!(fs::canonicalize(directory).unwrap().starts_with(&owned));
    let target = fixture._owned.path().join("junction-target");
    let retained = fixture._owned.path().join("retained-session-directory");
    fs::create_dir(&target).unwrap();
    assert!(fs::canonicalize(&target).unwrap().starts_with(&owned));
    assert!(
        fs::canonicalize(retained.parent().unwrap())
            .unwrap()
            .starts_with(&owned)
    );
    let original_bytes = fs::read(&fixture.changed_file).unwrap();
    // Equal length ensures the quota cannot accidentally detect a followed
    // junction; refusal must come from the checked path/open rules.
    let target_bytes = vec![b'Z'; original_bytes.len()];
    fs::write(target.join("events.ndjson"), &target_bytes).unwrap();
    let moved_directory = directory.to_owned();
    let moved_retained = retained.clone();
    let moved_target = target.clone();
    let (hooks, observation) =
        historical_observation(&fixture, vec![fixture.changed_file.clone()], move |_path| {
            // All endpoints were canonicalized inside this owned fixture above.
            // Preserve the original directory, then mutate only this captured job.
            fs::rename(&moved_directory, &moved_retained).unwrap();
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(moved_directory.to_str().unwrap().replace('/', "\\"))
                .arg(moved_target.to_str().unwrap().replace('/', "\\"))
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        });
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    assert_historical_target_failures(&hooks, &observation);
    let direct =
        inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into()).unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), direct.to_string());
    observation.drained();
    assert_eq!(observation.started.load(Ordering::SeqCst), 4);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(target.join("events.ndjson")).unwrap(),
        target_bytes
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(
        fs::read(retained.join("events.ndjson")).unwrap(),
        original_bytes
    );
}

// First pipeline RED slice. Synchronization is owned by this invocation.
// The runner below delegates to the existing all-metadata-first candidate.
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct PipelineState {
    admitted: usize,
    checked_hashes: usize,
    producer_released_by_hash: bool,
    held_workers: usize,
    release_workers: bool,
    worker_gate_expired: bool,
    queued: usize,
    producer_waiting: bool,
    waiting_workers: usize,
    producer_fault_entered: bool,
    producer_gate_expired: bool,
    arm_worker_fault: bool,
    worker_fault_entered: bool,
    actual_worker_error: bool,
    fallback_drained: Vec<bool>,
}

pub(in crate::store::migration) struct PipelineHooks {
    state: Mutex<PipelineState>,
    changed: Condvar,
    pause_first_admission: bool,
    hold_workers: bool,
    fault: Option<FaultPlan>,
    admission: Option<AdmissionPlan>,
    historical: Option<HistoricalMutation>,
}

impl PipelineHooks {
    fn new(pause_first_admission: bool, hold_workers: bool) -> Self {
        Self {
            state: Mutex::new(PipelineState::default()),
            changed: Condvar::new(),
            pause_first_admission,
            hold_workers,
            fault: None,
            admission: None,
            historical: None,
        }
    }

    // GREEN: invoke this after successful job submission, before continuing
    // enumeration. The RED barrier has only appended metadata at this point.
    pub(in crate::store::migration) fn admitted(&self, count: usize) -> Result<(), DevMapError> {
        let mut state = self.state.lock().unwrap();
        state.admitted = count;
        self.changed.notify_all();
        if self.pause_first_admission && count == 1 {
            // A completed real checked hash is the ONLY successful release.
            // The finite timeout records failure and permits normal cleanup.
            let (next, _) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(2), |s| s.checked_hashes == 0)
                .unwrap();
            state = next;
            state.producer_released_by_hash = state.checked_hashes > 0;
        }
        drop(state);
        self.producer_fault(count)?;
        self.after_admission_measurement(count);
        Ok(())
    }

    pub(super) fn before_hash(&self, path: &Path) {
        self.mutate_historical_target(path);
        self.mutate_admission_job(path);
        self.grow_concurrent_job(path);
        if !self.hold_workers {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.held_workers += 1;
        self.changed.notify_all();
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |s| !s.release_workers)
            .unwrap();
        if !state.release_workers {
            state.worker_gate_expired = true;
            state.release_workers = true;
            self.changed.notify_all();
        }
        let inject = state.arm_worker_fault && !state.worker_fault_entered;
        if inject {
            state.worker_fault_entered = true;
            self.changed.notify_all();
        }
        drop(state); // A deliberate panic must not poison the test control lock.
        if inject {
            let fault = self.fault.as_ref().unwrap();
            match fault.kind {
                FaultKind::WorkerError => {
                    // Actual checked-open failure; the serial oracle sees the
                    // same stable hardlink refusal on the next full traversal.
                    fs::hard_link(path, &fault.alias).unwrap();
                }
                FaultKind::WorkerPanic | FaultKind::ProducerErrorAndWorkerPanic => {
                    panic!("controlled inventory worker panic at full queue");
                }
                _ => unreachable!(),
            }
        }
    }

    pub(super) fn after_hash(&self, valid: bool) {
        let mut state = self.state.lock().unwrap();
        if valid {
            state.checked_hashes += 1;
        } else if state.worker_fault_entered {
            state.actual_worker_error = true;
        }
        self.changed.notify_all();
    }

    // Future GREEN observation point: actual queue-full wait state, outside
    // the queue lock. RED deliberately has no queue to report. Never fabricate
    // pending work inside the compatibility runner to satisfy this test.
    // admitted is the actual pending discovery ordinal + 1. The ninth job
    // has no post-submission callback until backpressure releases it.
    #[allow(dead_code)]
    pub(super) fn queue_waiting(&self, queued: usize, admitted: usize, waiting: bool) {
        let mut state = self.state.lock().unwrap();
        state.queued = queued;
        state.admitted = admitted;
        state.producer_waiting = waiting;
        self.changed.notify_all();
    }
}

fn pipeline_fixture() -> Fixture {
    let fixture = Fixture::new();
    // All possible first payloads are nonempty. Twelve jobs exceed the
    // four running + four queued + one transient producer descriptor window.
    for origin in &fixture.origins {
        fs::write(
            origin.git_dir.join("devmap/sessions/s0/events.ndjson"),
            b"nonempty checked payload",
        )
        .unwrap();
    }
    for index in 0..4 {
        let directory = fixture.origins[0]
            .git_dir
            .join(format!("devmap/sessions/pipeline{index}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("events.ndjson"), vec![index as u8; 130_001]).unwrap();
    }
    fixture
}

fn pipeline_red_candidate(
    fixture: &Fixture,
    observation: &Observation,
) -> Result<Option<FrozenManifest>, DevMapError> {
    // Current barrier candidate, not a test queue or separate implementation.
    // GREEN must route this through the actual pipeline.
    candidate(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        observation,
        || {},
    )
}

#[test]
fn pipeline_checked_full_hash_releases_producer_before_second_admission() {
    // Catches restoration of the metadata-before-hashing barrier.
    let fixture = pipeline_fixture();
    let expected = fixture.serial();
    let hooks = Arc::new(PipelineHooks::new(true, false));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let actual = pipeline_red_candidate(&fixture, &observation);
    observation.drained();
    assert_eq!(actual.unwrap().unwrap(), expected);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    let state = hooks.state.lock().unwrap();
    assert_eq!(state.checked_hashes, 12);
    assert!(
        state.producer_released_by_hash,
        "producer reached its finite timeout: no real checked full hash completed before second admission"
    );
}

#[test]
fn pipeline_full_queue_blocks_producer_then_releases_and_joins_workers() {
    // Catches an unbounded queue, absent overlap, or missing producer wakeup
    // after the four held payload readers are released.
    let fixture = pipeline_fixture();
    let expected = fixture.serial();
    let hooks = Arc::new(PipelineHooks::new(false, true));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (actual, saw_full_queue, snapshot) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| pipeline_red_candidate(&fixture, &observation));
        let state = hooks.state.lock().unwrap();
        let (mut state, _) = hooks
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |s| {
                !(s.held_workers == 4 && s.producer_waiting && s.admitted == 9)
            })
            .unwrap();
        let saw_full_queue =
            state.held_workers == 4 && state.producer_waiting && state.admitted == 9;
        let snapshot = (state.queued, state.admitted, state.checked_hashes);
        // Cleanup precedes every assertion, including the intended RED failure.
        state.release_workers = true;
        hooks.changed.notify_all();
        drop(state);
        (handle.join(), saw_full_queue, snapshot)
    });
    observation.drained();
    assert_eq!(actual.unwrap().unwrap().unwrap(), expected);
    assert_eq!(observation.started.load(Ordering::SeqCst), 4);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    let state = hooks.state.lock().unwrap();
    assert!(!state.worker_gate_expired, "worker safety deadline fired");
    assert_eq!(state.checked_hashes, 12);
    assert!(
        saw_full_queue,
        "four held workers never overlapped a producer blocked on the bounded job queue"
    );
    assert_eq!(
        snapshot,
        (4, 9, 0),
        "four queued, four in flight, one producer-held; no hash bypasses the gate"
    );
}

#[test]
fn pipeline_first_inadmissible_payload_still_starts_no_payload_io() {
    let fixture = pipeline_fixture();
    // No global zero-worker promise for LATE failures is inferred here.
    // Every file is nonempty, so zero bytes rejects the first discovered file.
    for limits in [
        Limits {
            bytes: 0,
            ..Limits::default()
        },
        Limits {
            files: 0,
            ..Limits::default()
        },
        Limits {
            directories: 0,
            ..Limits::default()
        },
    ] {
        let observation = Observation::default();
        let result = candidate(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || panic!("first admission must decline"),
        );
        observation.drained();
        assert!(result.unwrap().is_none());
        assert_eq!(observation.started.load(Ordering::SeqCst), 0);
        assert_eq!(observation.hashed.load(Ordering::SeqCst), 0);
    }
}

#[derive(Clone, Copy)]
enum FaultKind {
    WorkerPanic,
    ProducerPanic,
    ProducerError,
    ProducerErrorAndWorkerPanic,
    WorkerError,
}

struct FaultPlan {
    kind: FaultKind,
    alias: PathBuf,
}

impl PipelineHooks {
    fn fault(kind: FaultKind, fixture: &Fixture) -> Self {
        let mut hooks = Self::new(false, !matches!(kind, FaultKind::ProducerPanic));
        hooks.fault = Some(FaultPlan {
            kind,
            alias: fixture._owned.path().join("pipeline-retained-hardlink"),
        });
        hooks
    }

    fn producer_fault(&self, count: usize) -> Result<(), DevMapError> {
        let Some(fault) = &self.fault else {
            return Ok(());
        };
        if count != 1
            || !matches!(
                fault.kind,
                FaultKind::ProducerPanic
                    | FaultKind::ProducerError
                    | FaultKind::ProducerErrorAndWorkerPanic
            )
        {
            return Ok(());
        }
        let state = self.state.lock().unwrap();
        let ready = |s: &PipelineState| match fault.kind {
            // First real job finished; at least one worker is truly waiting
            // for more work. Merely starting a thread does not satisfy this.
            FaultKind::ProducerPanic => s.checked_hashes > 0 && s.waiting_workers > 0,
            _ => s.held_workers > 0,
        };
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |s| !ready(s))
            .unwrap();
        if !ready(&state) {
            state.producer_gate_expired = true;
            self.changed.notify_all();
            return Ok(()); // Barrier RED misses the prerequisite; assert later.
        }
        state.producer_fault_entered = true;
        self.changed.notify_all();
        drop(state);
        match fault.kind {
            FaultKind::ProducerPanic => panic!("controlled inventory producer panic"),
            // One-shot injected producer failure travels through the real
            // collector error path; the fresh serial oracle is uninstrumented.
            _ => Err(fail(
                "controlled inventory producer failure after admission",
            )),
        }
    }

    // GREEN only: balanced entry/exit observations around a real empty-queue
    // worker wait, outside the queue lock. The barrier has no such wait.
    #[allow(dead_code)]
    pub(super) fn worker_waiting(&self, waiting: bool) {
        let mut state = self.state.lock().unwrap();
        if waiting {
            state.waiting_workers += 1;
        } else {
            state.waiting_workers = state.waiting_workers.checked_sub(1).unwrap();
        }
        self.changed.notify_all();
    }

    pub(super) fn before_fallback(&self, observation: &Observation) {
        self.state.lock().unwrap().fallback_drained.push(
            observation.active.load(Ordering::SeqCst) == 0
                && observation.started.load(Ordering::SeqCst)
                    == observation.joined.load(Ordering::SeqCst),
        );
    }
}

type FaultOutcome = std::thread::Result<Result<FrozenManifest, DevMapError>>;

fn run_pipeline_fault(
    fixture: &Fixture,
    observation: &Observation,
    hooks: &PipelineHooks,
) -> (FaultOutcome, bool) {
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_serial_oracle(
                    &fixture.workspace,
                    &fixture.origins,
                    STAMP,
                    Limits::default(),
                    observation,
                    || {},
                )
            }))
        });
        let kind = hooks.fault.as_ref().unwrap().kind;
        let ready = |s: &PipelineState| match kind {
            FaultKind::WorkerPanic | FaultKind::WorkerError => {
                s.held_workers == 4 && s.producer_waiting && s.queued == 4 && s.admitted == 9
            }
            _ => s.producer_fault_entered,
        };
        let state = hooks.state.lock().unwrap();
        let (mut state, _) = hooks
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |s| {
                !ready(s) && !s.producer_gate_expired
            })
            .unwrap();
        let reached = ready(&state);
        // Missing scheduling prerequisites NEVER arm a fault. Otherwise an old
        // barrier could pass a lifecycle test without exercising the lifecycle.
        state.arm_worker_fault = reached
            && matches!(
                kind,
                FaultKind::WorkerPanic
                    | FaultKind::WorkerError
                    | FaultKind::ProducerErrorAndWorkerPanic
            );
        state.release_workers = true;
        hooks.changed.notify_all();
        drop(state);
        // Finite hook waits cannot bound a broken production queue. Root runs
        // this test binary in an owned Windows Job with a hard process deadline.
        (handle.join().unwrap(), reached)
    })
}

fn assert_pipeline_fault_drained(observation: &Observation, hooks: &PipelineHooks) {
    observation.drained();
    let state = hooks.state.lock().unwrap();
    assert!(
        !state.worker_gate_expired,
        "test worker safety deadline fired"
    );
    assert_eq!(
        state.waiting_workers, 0,
        "workers left an empty-queue wait unclosed"
    );
}

#[test]
fn pipeline_fault_worker_panic_wakes_full_queue_producer_without_fallback() {
    let fixture = pipeline_fixture();
    let hooks = Arc::new(PipelineHooks::fault(FaultKind::WorkerPanic, &fixture));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (result, reached) = run_pipeline_fault(&fixture, &observation, &hooks);
    assert_pipeline_fault_drained(&observation, &hooks);
    assert!(
        reached,
        "worker panic prerequisite: producer never blocked on full queue"
    );
    let state = hooks.state.lock().unwrap();
    assert!(
        state.worker_fault_entered,
        "worker panic injection was not entered"
    );
    assert!(state.fallback_drained.is_empty());
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    assert_eq!(
        result.unwrap().unwrap_err().to_string(),
        "repository store: parallel inventory worker panicked"
    );
}

#[test]
fn pipeline_fault_producer_panic_drains_workers_waiting_for_more_jobs() {
    let fixture = pipeline_fixture();
    let hooks = Arc::new(PipelineHooks::fault(FaultKind::ProducerPanic, &fixture));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (result, reached) = run_pipeline_fault(&fixture, &observation, &hooks);
    assert_pipeline_fault_drained(&observation, &hooks);
    assert!(
        reached,
        "producer panic prerequisite: no completed real hash and waiting worker"
    );
    let state = hooks.state.lock().unwrap();
    assert!(state.producer_fault_entered);
    assert!(state.fallback_drained.is_empty());
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    // The design permits resumed unwind or a specifically controlled fatal
    // conversion, never success or the ordinary fallback path.
    match result {
        Err(_) => {}
        Ok(Err(error)) => assert_eq!(
            error.to_string(),
            "repository store: parallel inventory producer panicked"
        ),
        Ok(Ok(_)) => panic!("producer panic published a successful manifest"),
    }
}

#[test]
fn pipeline_fault_late_producer_error_drains_before_oracle_and_worker_panic_dominates() {
    let mut cases = Vec::new();
    for panic_worker in [false, true] {
        let fixture = pipeline_fixture();
        let expected = fixture.serial();
        let kind = if panic_worker {
            FaultKind::ProducerErrorAndWorkerPanic
        } else {
            FaultKind::ProducerError
        };
        let hooks = Arc::new(PipelineHooks::fault(kind, &fixture));
        let observation = Observation {
            pipeline: Some(hooks.clone()),
            ..Observation::default()
        };
        let (result, reached) = run_pipeline_fault(&fixture, &observation, &hooks);
        cases.push((panic_worker, expected, hooks, observation, result, reached));
    }
    // Run and clean up BOTH variants before asserting the barrier RED result.
    for (panic_worker, expected, hooks, observation, result, reached) in cases {
        assert_pipeline_fault_drained(&observation, &hooks);
        assert!(
            reached,
            "producer error prerequisite: no live held payload worker"
        );
        let state = hooks.state.lock().unwrap();
        assert!(state.producer_fault_entered);
        if panic_worker {
            assert!(
                state.worker_fault_entered,
                "concurrent worker panic was not entered"
            );
            assert!(state.fallback_drained.is_empty());
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
            assert_eq!(
                result.unwrap().unwrap_err().to_string(),
                "repository store: parallel inventory worker panicked"
            );
        } else {
            assert_eq!(state.fallback_drained, vec![true]);
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
            assert_eq!(result.unwrap().unwrap(), expected);
        }
    }
}

#[test]
fn pipeline_fault_checked_worker_error_wakes_full_queue_before_one_serial_error() {
    let fixture = pipeline_fixture();
    let hooks = Arc::new(PipelineHooks::fault(FaultKind::WorkerError, &fixture));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (result, reached) = run_pipeline_fault(&fixture, &observation, &hooks);
    assert_pipeline_fault_drained(&observation, &hooks);
    assert!(
        reached,
        "worker error prerequisite: producer never blocked on full queue"
    );
    let state = hooks.state.lock().unwrap();
    assert!(
        state.worker_fault_entered,
        "real hardlink mutation was not entered"
    );
    assert!(
        state.actual_worker_error,
        "actual checked full hash did not reject the hardlink"
    );
    assert_eq!(state.fallback_drained, vec![true]);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    let expected = inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into())
        .unwrap_err()
        .to_string();
    assert_eq!(
        expected,
        "repository store: hard-linked legacy artifact refused"
    );
    assert_eq!(result.unwrap().unwrap_err().to_string(), expected);
}

// Slice 03: actual payload read accounting; no queue/submission simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct JobKey {
    discovery_ordinal: usize,
    origin: usize,
    relative: String,
    expected_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReadCounts {
    opened: u64,
    returned: u64,
    accepted: u64,
    probes: u64,
    eof: u64,
}

#[derive(Clone, Debug)]
struct AdmissionJob {
    key: JobKey,
    path: PathBuf,
    reserved_total: u64,
    counts: ReadCounts,
    finished: bool,
    valid: bool,
    mutations: usize,
}

#[derive(Default)]
struct AdmissionState {
    jobs: Vec<AdmissionJob>,
    prefix_released_by_hash: bool,
    claims: Vec<JobKey>,
    growth_keys: Vec<JobKey>,
    reader_cancelled: bool,
    claims_after_cancel: usize,
}

enum AdmissionMode {
    Count,
    Prefix,
    Mutate(Vec<u8>),
    GrowFour,
}
struct AdmissionPlan {
    mode: AdmissionMode,
    state: Mutex<AdmissionState>,
    changed: Condvar,
}
struct JobReader<'a> {
    plan: &'a AdmissionPlan,
    ordinal: usize,
}
impl InventoryReadObserver for JobReader<'_> {
    fn returned(&mut self, bytes: usize) {
        self.plan.state.lock().unwrap().jobs[self.ordinal]
            .counts
            .returned += bytes as u64;
    }
    fn accepted(&mut self, bytes: usize) {
        self.plan.state.lock().unwrap().jobs[self.ordinal]
            .counts
            .accepted += bytes as u64;
    }
    fn probe(&mut self, bytes: usize) {
        self.plan.state.lock().unwrap().jobs[self.ordinal]
            .counts
            .probes += bytes as u64;
    }
    fn eof(&mut self) {
        self.plan.state.lock().unwrap().jobs[self.ordinal]
            .counts
            .eof += 1;
    }
}

impl PipelineHooks {
    fn measuring(mode: AdmissionMode) -> Self {
        let mut hooks = Self::new(false, false);
        hooks.admission = Some(AdmissionPlan {
            mode,
            state: Mutex::new(AdmissionState::default()),
            changed: Condvar::new(),
        });
        hooks
    }

    pub(in crate::store::migration) fn reserved(
        &self,
        ordinal: usize,
        file: &FrozenFile,
        total: u64,
        path: &Path,
    ) {
        let Some(plan) = &self.admission else {
            return;
        };
        let mut state = plan.state.lock().unwrap();
        assert_eq!(ordinal, state.jobs.len());
        state.jobs.push(AdmissionJob {
            key: JobKey {
                discovery_ordinal: ordinal,
                origin: file.origin,
                relative: file.relative.clone(),
                expected_bytes: file.bytes,
            },
            path: path.to_owned(),
            reserved_total: total,
            counts: ReadCounts::default(),
            finished: false,
            valid: false,
            mutations: 0,
        });
        // This is an actual reservation/descriptor append, NOT an enqueue event.
    }

    fn after_admission_measurement(&self, count: usize) {
        let Some(plan) = &self.admission else {
            return;
        };
        if count != 1 || !matches!(&plan.mode, AdmissionMode::Prefix) {
            return;
        }
        let state = plan.state.lock().unwrap();
        let (mut state, _) = plan
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |s| {
                !s.jobs[0].finished || !s.jobs[0].valid
            })
            .unwrap();
        state.prefix_released_by_hash = state.jobs[0].finished && state.jobs[0].valid;
        // Timeout only enables cleanup; it never represents a completed read.
    }

    fn mutate_admission_job(&self, path: &Path) {
        let Some(plan) = &self.admission else {
            return;
        };
        let AdmissionMode::Mutate(replacement) = &plan.mode else {
            return;
        };
        let ordinal = {
            let state = plan.state.lock().unwrap();
            let job = state.jobs.iter().find(|job| job.path == path).unwrap();
            assert_eq!(
                job.counts.opened, 0,
                "mutation must precede checked payload open"
            );
            assert_eq!(job.mutations, 0, "one mutation per actual job identity");
            job.key.discovery_ordinal
        };
        fs::write(path, replacement).unwrap();
        plan.state.lock().unwrap().jobs[ordinal].mutations += 1;
    }

    pub(super) fn hash_job(
        &self,
        path: &Path,
        expected_bytes: u64,
    ) -> Result<(u64, String), DevMapError> {
        let Some(plan) = &self.admission else {
            // No observation allocation for existing fault/overlap controls.
            return inventory_candidate_hash(path, expected_bytes);
        };
        let ordinal = {
            let state = plan.state.lock().unwrap();
            let job = state.jobs.iter().find(|job| job.path == path).unwrap();
            assert_eq!(job.key.expected_bytes, expected_bytes);
            job.key.discovery_ordinal
        };
        let result =
            inventory_hash_observed(path, expected_bytes, JobReader { plan, ordinal }, || {
                plan.state.lock().unwrap().jobs[ordinal].counts.opened += 1;
            });
        let mut state = plan.state.lock().unwrap();
        state.jobs[ordinal].finished = true;
        state.jobs[ordinal].valid = result.as_ref().is_ok_and(|(n, _)| *n == expected_bytes);
        plan.changed.notify_all();
        result
    }

    fn admission_snapshot(&self) -> (Vec<AdmissionJob>, bool) {
        let state = self.admission.as_ref().unwrap().state.lock().unwrap();
        (state.jobs.clone(), state.prefix_released_by_hash)
    }
}

fn admission_fixture(payloads: &[Vec<u8>]) -> Fixture {
    assert!(!payloads.is_empty() && payloads.len() <= 2);
    let mut fixture = Fixture::new();
    let owned = fs::canonicalize(fixture._owned.path()).unwrap();
    for (index, origin) in fixture.origins.iter().enumerate() {
        // Replace only fixture-owned sample sessions, never a repository supplied
        // by the user. Both roots were created by Fixture::new in this TempDir.
        let sessions = origin.git_dir.join("devmap/sessions");
        assert!(fs::canonicalize(&sessions).unwrap().starts_with(&owned));
        fs::remove_dir_all(&sessions).unwrap();
        if let Some(bytes) = payloads.get(index) {
            fs::create_dir_all(sessions.join("only")).unwrap();
            fs::write(sessions.join("only/events.ndjson"), bytes).unwrap();
        }
    }
    fixture.origins.truncate(payloads.len());
    fixture.changed_file = fixture.origins[0]
        .git_dir
        .join("devmap/sessions/only/events.ndjson");
    fixture
}

fn measured_observation(mode: AdmissionMode) -> (Arc<PipelineHooks>, Observation) {
    let hooks = Arc::new(PipelineHooks::measuring(mode));
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    (hooks, observation)
}

#[test]
fn pipeline_admission_first_refusal_has_zero_actual_payload_bytes() {
    let fixture = admission_fixture(&[b"abcd".to_vec(), b"efgh".to_vec()]);
    for limits in [
        Limits {
            bytes: 3,
            ..Limits::default()
        },
        Limits {
            files: 0,
            ..Limits::default()
        },
        Limits {
            directories: 0,
            ..Limits::default()
        },
    ] {
        let (hooks, observation) = measured_observation(AdmissionMode::Count);
        let result = candidate(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || panic!("first refusal must precede enumeration callback"),
        );
        observation.drained();
        assert!(result.unwrap().is_none());
        assert!(hooks.admission_snapshot().0.is_empty());
        assert_eq!(observation.started.load(Ordering::SeqCst), 0);
        assert_eq!(observation.hashed.load(Ordering::SeqCst), 0);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        // No admitted key and no hash call means no observed payload open/read.
    }
}

#[test]
fn pipeline_admission_exact_and_late_prefix_limits_preserve_read_budget() {
    let mut cases = Vec::new();
    for (limits, late) in [
        (
            Limits {
                bytes: 8,
                files: 2,
                directories: 4,
            },
            false,
        ),
        (
            Limits {
                bytes: 7,
                files: 2,
                directories: 4,
            },
            true,
        ),
        (
            Limits {
                bytes: 8,
                files: 1,
                directories: 4,
            },
            true,
        ),
        (
            Limits {
                bytes: 8,
                files: 2,
                directories: 2,
            },
            true,
        ),
    ] {
        let fixture = admission_fixture(&[b"abcd".to_vec(), b"efgh".to_vec()]);
        let expected = fixture.serial();
        let mode = if late {
            AdmissionMode::Prefix
        } else {
            AdmissionMode::Count
        };
        let (hooks, observation) = measured_observation(mode);
        let result = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || {},
        );
        cases.push((late, expected, hooks, observation, result));
    }
    // Exercise every quota axis before any intended barrier RED assertion.
    for (late, expected, hooks, observation, result) in cases {
        observation.drained();
        assert_eq!(result.unwrap(), expected);
        let (jobs, reached) = hooks.admission_snapshot();
        if late {
            assert!(
                reached,
                "late-limit producer advanced without first real checked hash completion"
            );
            assert_eq!(jobs.len(), 1, "inadmissible second payload was reserved");
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
            assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
        } else {
            assert_eq!(jobs.len(), 2);
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        }
        for (index, job) in jobs.iter().enumerate() {
            assert_eq!(
                job.key,
                JobKey {
                    discovery_ordinal: index,
                    origin: index,
                    relative: "sessions/only/events.ndjson".into(),
                    expected_bytes: 4
                }
            );
            assert_eq!(job.reserved_total, 4 * (index as u64 + 1));
            assert_eq!(
                job.counts,
                ReadCounts {
                    opened: 1,
                    returned: 4,
                    accepted: 4,
                    probes: 0,
                    eof: 1
                }
            );
            assert!(job.finished && job.valid);
        }
    }
}

#[test]
fn pipeline_admission_failed_mutations_count_real_returned_accepted_and_probe_bytes() {
    for (original, replacement, want) in [
        (
            Vec::new(),
            vec![9],
            ReadCounts {
                opened: 1,
                returned: 1,
                accepted: 0,
                probes: 1,
                eof: 0,
            },
        ),
        (
            vec![1; 4],
            vec![2; 5],
            ReadCounts {
                opened: 1,
                returned: 5,
                accepted: 4,
                probes: 1,
                eof: 0,
            },
        ),
        (
            vec![1; 4],
            vec![2],
            ReadCounts {
                opened: 1,
                returned: 1,
                accepted: 1,
                probes: 0,
                eof: 1,
            },
        ),
    ] {
        let quota = original.len() as u64;
        let fixture = admission_fixture(&[original]);
        let (hooks, observation) = measured_observation(AdmissionMode::Mutate(replacement));
        let result = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {},
        );
        observation.drained();
        assert_eq!(result.unwrap(), fixture.serial());
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
        let jobs = hooks.admission_snapshot().0;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].key.discovery_ordinal, 0);
        assert_eq!(jobs[0].key.origin, 0);
        assert_eq!(jobs[0].key.relative, "sessions/only/events.ndjson");
        assert_eq!(jobs[0].key.expected_bytes, quota);
        assert_eq!(
            jobs[0].reserved_total, quota,
            "shrink must not refund reserved bytes"
        );
        assert_eq!(jobs[0].mutations, 1);
        assert!(jobs[0].finished && !jobs[0].valid);
        assert_eq!(jobs[0].counts, want);
        assert_eq!(
            jobs[0].counts.returned,
            jobs[0].counts.accepted + jobs[0].counts.probes
        );
    }
}

#[test]
fn pipeline_admission_exact_eof_same_length_edit_and_multichunk_hash_are_complete() {
    use sha2::{Digest, Sha256};
    for (original, replacement) in [
        (Vec::new(), None),
        (b"abcd".to_vec(), Some(b"wxyz".to_vec())),
        (vec![7; 65_537], None),
    ] {
        let bytes = replacement.as_ref().unwrap_or(&original).clone();
        // Independent fixture hash, not a call back into the candidate reader.
        let expected_hash = format!("{:x}", Sha256::digest(&bytes));
        let expected_mutations = usize::from(replacement.is_some());
        let fixture = admission_fixture(&[original]);
        let mode = replacement.map_or(AdmissionMode::Count, AdmissionMode::Mutate);
        let (hooks, observation) = measured_observation(mode);
        let result = candidate(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {},
        )
        .unwrap()
        .unwrap();
        observation.drained();
        assert_eq!(result, fixture.serial());
        assert_eq!(result.files[0].sha256, expected_hash);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        let jobs = hooks.admission_snapshot().0;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].mutations, expected_mutations);
        assert!(jobs[0].finished && jobs[0].valid);
        let count = bytes.len() as u64;
        assert_eq!(
            jobs[0].counts,
            ReadCounts {
                opened: 1,
                returned: count,
                accepted: count,
                probes: 0,
                eof: 1
            }
        );
    }
}

impl PipelineHooks {
    pub(super) fn claimed(&self, file: &FrozenFile) {
        let Some(plan) = &self.admission else {
            return;
        };
        let mut state = plan.state.lock().unwrap();
        // Current barrier ordinals are sorted indices; bind the actual claim
        // back to the captured discovery key by origin/path, not that index.
        let key = state
            .jobs
            .iter()
            .find(|job| job.key.origin == file.origin && job.key.relative == file.relative)
            .unwrap()
            .key
            .clone();
        if state.reader_cancelled {
            state.claims_after_cancel += 1;
        }
        state.claims.push(key);
    }

    pub(super) fn reader_cancelled(&self) {
        let Some(plan) = &self.admission else {
            return;
        };
        plan.state.lock().unwrap().reader_cancelled = true;
    }

    fn grow_concurrent_job(&self, path: &Path) {
        let Some(plan) = &self.admission else {
            return;
        };
        if !matches!(&plan.mode, AdmissionMode::GrowFour) {
            return;
        }
        let ordinal = {
            let mut state = plan.state.lock().unwrap();
            if state.growth_keys.len() == 4 {
                return;
            }
            let key = state
                .jobs
                .iter()
                .find(|job| job.path == path)
                .unwrap()
                .key
                .clone();
            assert_eq!(key.expected_bytes, 4);
            let ordinal = key.discovery_ordinal;
            assert_eq!(state.jobs[ordinal].counts.opened, 0);
            assert_eq!(state.jobs[ordinal].mutations, 0);
            state.growth_keys.push(key);
            ordinal
        };
        let mut bytes = fs::read(path).unwrap();
        assert_eq!(bytes.len(), 4);
        bytes.push(9);
        fs::write(path, bytes).unwrap();
        plan.state.lock().unwrap().jobs[ordinal].mutations += 1;
    }
}

#[test]
fn pipeline_final_four_inflight_growth_reads_only_four_unhashed_probe_bytes() {
    let fixture = admission_fixture(&[vec![1; 4]]);
    for index in 0..11 {
        let directory = fixture.origins[0]
            .git_dir
            .join(format!("devmap/sessions/grow{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("events.ndjson"), [1; 4]).unwrap();
    }
    let mut hooks = PipelineHooks::measuring(AdmissionMode::GrowFour);
    hooks.hold_workers = true;
    let hooks = Arc::new(hooks);
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (result, reached) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            with_serial_oracle(
                &fixture.workspace,
                &fixture.origins,
                STAMP,
                Limits::default(),
                &observation,
                || {},
            )
        });
        let state = hooks.state.lock().unwrap();
        let (mut state, _) = hooks
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |s| {
                !(s.held_workers == 4 && s.producer_waiting && s.queued == 4 && s.admitted == 9)
            })
            .unwrap();
        let reached = state.held_workers == 4
            && state.producer_waiting
            && state.queued == 4
            && state.admitted == 9;
        // Every worker gate is released before join/assert, including barrier RED.
        state.release_workers = true;
        hooks.changed.notify_all();
        drop(state);
        (handle.join().unwrap(), reached)
    });
    observation.drained();
    assert!(!hooks.state.lock().unwrap().worker_gate_expired);
    assert_eq!(result.unwrap(), fixture.serial());
    assert!(
        reached,
        "four growth readers never overlapped a producer blocked on the actual full queue"
    );
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
    let state = hooks.admission.as_ref().unwrap().state.lock().unwrap();
    assert_eq!(state.growth_keys.len(), 4);
    assert_eq!(
        state.claims.len(),
        4,
        "a queued/later job was claimed after held readers failed"
    );
    assert!(state.reader_cancelled);
    assert_eq!(state.claims_after_cancel, 0);
    for key in &state.growth_keys {
        assert!(state.claims.contains(key));
    }
    for job in &state.jobs {
        if state.growth_keys.contains(&job.key) {
            assert_eq!(job.mutations, 1);
            assert!(job.finished && !job.valid);
            assert_eq!(
                job.counts,
                ReadCounts {
                    opened: 1,
                    returned: 5,
                    accepted: 4,
                    probes: 1,
                    eof: 0
                }
            );
        } else {
            assert_eq!(job.mutations, 0);
            assert_eq!(
                job.counts,
                ReadCounts::default(),
                "later descriptor reached payload I/O"
            );
            assert!(!job.finished);
        }
    }
    assert_eq!(
        state
            .jobs
            .iter()
            .map(|job| job.counts.returned)
            .sum::<u64>(),
        20
    );
    assert_eq!(
        state
            .jobs
            .iter()
            .map(|job| job.counts.accepted)
            .sum::<u64>(),
        16
    );
    assert_eq!(
        state.jobs.iter().map(|job| job.counts.probes).sum::<u64>(),
        4
    );
}

#[test]
fn pipeline_final_missing_and_invalid_ordinals_after_real_hashes_are_fatal() {
    for (fault, expected_error) in [
        (1, "repository store: parallel inventory result missing"),
        (
            2,
            "repository store: parallel inventory result ordinal invalid",
        ),
    ] {
        let fixture = Fixture::new();
        let expected = fixture.serial();
        let hooks = Arc::new(PipelineHooks::new(false, false));
        let observation = Observation {
            pipeline: Some(hooks.clone()),
            ..Observation::default()
        };
        observation.result_fault.store(fault, Ordering::SeqCst);
        let result = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {},
        );
        observation.drained();
        assert!(observation.result_fault_entered.load(Ordering::SeqCst));
        assert_eq!(
            observation.hashed.load(Ordering::SeqCst),
            expected.files.len()
        );
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        assert!(hooks.state.lock().unwrap().fallback_drained.is_empty());
        assert_eq!(result.unwrap_err().to_string(), expected_error);
        assert_eq!(fixture.serial(), expected);
    }
}

#[test]
fn pipeline_final_partial_spawn_refusal_joins_before_one_oracle_and_is_invocation_local() {
    for refused_index in [1, 3] {
        let fixture = Fixture::new();
        let expected = fixture.serial();
        let hooks = Arc::new(PipelineHooks::new(false, false));
        let observation = Observation {
            pipeline: Some(hooks.clone()),
            ..Observation::default()
        };
        observation
            .spawn_refuse_at
            .store(refused_index, Ordering::SeqCst);
        let result = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {},
        );
        observation.drained();
        assert!(observation.spawn_refusal_entered.load(Ordering::SeqCst));
        assert_eq!(observation.started.load(Ordering::SeqCst), refused_index);
        assert_eq!(observation.joined.load(Ordering::SeqCst), refused_index);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
        assert_eq!(result.unwrap(), expected);
        let fresh = Observation::default();
        let actual = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &fresh,
            || {},
        )
        .unwrap();
        fresh.drained();
        assert_eq!(actual, expected);
        assert!(!fresh.spawn_refusal_entered.load(Ordering::SeqCst));
        assert_eq!(fresh.fallbacks.load(Ordering::SeqCst), 0);
        assert_eq!(fresh.hashed.load(Ordering::SeqCst), expected.files.len());
    }
}

struct HistoricalTarget {
    path: PathBuf,
    origin: usize,
    relative: String,
    expected_bytes: u64,
}
struct HistoricalState {
    reached: Vec<Option<JobKey>>,
    completed: Vec<usize>,
    expired: bool,
}
struct HistoricalMutation {
    targets: Vec<HistoricalTarget>,
    action: Box<dyn Fn(&Path) + Send + Sync>,
    state: Mutex<HistoricalState>,
    changed: Condvar,
}

fn assert_owned_destination(fixture: &Fixture, destination: &Path) {
    let owned = fs::canonicalize(fixture._owned.path()).unwrap();
    assert!(
        fs::canonicalize(destination.parent().unwrap())
            .unwrap()
            .starts_with(owned)
    );
    assert!(!destination.exists());
}

fn historical_observation(
    fixture: &Fixture,
    paths: Vec<PathBuf>,
    action: impl Fn(&Path) + Send + Sync + 'static,
) -> (Arc<PipelineHooks>, Observation) {
    let owned = fs::canonicalize(fixture._owned.path()).unwrap();
    let targets: Vec<_> = paths
        .into_iter()
        .map(|path| {
            assert!(fs::canonicalize(&path).unwrap().starts_with(&owned));
            let (origin, relative) = fixture
                .origins
                .iter()
                .enumerate()
                .find_map(|(index, root)| {
                    path.strip_prefix(root.git_dir.join("devmap"))
                        .ok()
                        .map(|p| (index, p.to_str().unwrap().replace('\\', "/")))
                })
                .unwrap();
            let expected_bytes = fs::metadata(&path).unwrap().len();
            HistoricalTarget {
                path,
                origin,
                relative,
                expected_bytes,
            }
        })
        .collect();
    let count = targets.len();
    let mut hooks = PipelineHooks::measuring(AdmissionMode::Count);
    hooks.historical = Some(HistoricalMutation {
        targets,
        action: Box::new(action),
        state: Mutex::new(HistoricalState {
            reached: vec![None; count],
            completed: vec![0; count],
            expired: false,
        }),
        changed: Condvar::new(),
    });
    let hooks = Arc::new(hooks);
    let observation = Observation {
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    (hooks, observation)
}

impl PipelineHooks {
    fn mutate_historical_target(&self, path: &Path) {
        let Some(plan) = &self.historical else {
            return;
        };
        let Some(index) = plan.targets.iter().position(|target| target.path == path) else {
            return;
        };
        let target = &plan.targets[index];
        let key = {
            let state = self.admission.as_ref().unwrap().state.lock().unwrap();
            let job = state.jobs.iter().find(|job| job.path == path).unwrap();
            assert_eq!(
                (
                    job.key.origin,
                    job.key.relative.as_str(),
                    job.key.expected_bytes
                ),
                (
                    target.origin,
                    target.relative.as_str(),
                    target.expected_bytes
                )
            );
            assert_eq!(job.counts.opened, 0);
            assert!(!job.finished);
            job.key.clone()
        };
        let mut state = plan.state.lock().unwrap();
        assert!(
            state.reached[index].is_none(),
            "target job must enter hook exactly once"
        );
        state.reached[index] = Some(key);
        plan.changed.notify_all();
        let (mut state, _) = plan
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |s| {
                !s.expired && s.reached.iter().any(Option::is_none)
            })
            .unwrap();
        if state.reached.iter().any(Option::is_none) {
            state.expired = true;
        }
        let execute = !state.expired;
        plan.changed.notify_all();
        drop(state); // Never hold observation/control locks during mutation I/O.
        if execute {
            (plan.action)(path);
            plan.state.lock().unwrap().completed[index] += 1;
        }
    }
}

fn assert_historical_target_failures(hooks: &PipelineHooks, observation: &Observation) {
    observation.drained();
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
    let historical = hooks.historical.as_ref().unwrap().state.lock().unwrap();
    assert!(
        !historical.expired,
        "not all selected jobs reached their before-open gate"
    );
    assert!(historical.completed.iter().all(|count| *count == 1));
    let admission = hooks.admission.as_ref().unwrap().state.lock().unwrap();
    for key in &historical.reached {
        let key = key
            .as_ref()
            .expect("actual target-key gate was not entered");
        let job = admission.jobs.iter().find(|job| &job.key == key).unwrap();
        assert!(
            job.finished && !job.valid,
            "mutated target must actually fail its checked hash"
        );
        assert_eq!(
            admission
                .claims
                .iter()
                .filter(|claimed| *claimed == key)
                .count(),
            1
        );
    }
}

fn assert_historical_late_prefix(limits: Limits) {
    let fixture = admission_fixture(&[b"abcd".to_vec(), b"efgh".to_vec()]);
    let expected = fixture.serial();
    let (hooks, observation) = measured_observation(AdmissionMode::Prefix);
    let actual = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        limits,
        &observation,
        || {},
    )
    .unwrap();
    observation.drained();
    assert_eq!(actual, expected);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
    let (jobs, reached) = hooks.admission_snapshot();
    assert!(
        reached,
        "first real payload must finish before the later quota refusal"
    );
    assert_eq!(jobs.len(), 1, "inadmissible second payload was reserved");
    assert_eq!(
        jobs[0].key,
        JobKey {
            discovery_ordinal: 0,
            origin: 0,
            relative: "sessions/only/events.ndjson".into(),
            expected_bytes: 4
        }
    );
    assert_eq!(jobs[0].reserved_total, 4);
    assert!(jobs[0].reserved_total <= limits.bytes);
    assert_eq!(
        jobs[0].counts,
        ReadCounts {
            opened: 1,
            returned: 4,
            accepted: 4,
            probes: 0,
            eof: 1
        }
    );
    assert!(jobs[0].finished && jobs[0].valid);
    let state = hooks.admission.as_ref().unwrap().state.lock().unwrap();
    assert_eq!(
        state.claims,
        vec![jobs[0].key.clone()],
        "refused job reached actual dispatch/claim"
    );
}

// Only test builds can select eight workers. Production remains hard four;
// both counts use the same hashing algorithm and bounded queue.
fn eight_fixture() -> Fixture {
    let fixture = pipeline_fixture();
    for index in 4..8 {
        let directory = fixture.origins[0]
            .git_dir
            .join(format!("devmap/sessions/pipeline{index}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("events.ndjson"), vec![index as u8; 130_001]).unwrap();
    }
    assert_eq!(fixture.serial().files.len(), 16);
    fixture
}

struct ReleaseEight<'a>(&'a PipelineHooks);
impl Drop for ReleaseEight<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.release_workers = true;
        self.0.changed.notify_all();
    }
}

fn run_eight_held(
    fixture: &Fixture,
    observation: &Observation,
    hooks: &PipelineHooks,
) -> (FaultOutcome, bool, (usize, usize, usize)) {
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_serial_oracle(
                    &fixture.workspace,
                    &fixture.origins,
                    STAMP,
                    Limits::default(),
                    observation,
                    || {},
                )
            }))
        });
        // Unwind releases before scope joins, including a failed control wait.
        let release = ReleaseEight(hooks);
        let state = hooks.state.lock().unwrap();
        let (mut state, _) = hooks
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |s| {
                !(s.held_workers == 8 && s.producer_waiting && s.queued == 4 && s.admitted == 13)
            })
            .unwrap();
        let reached = state.held_workers == 8
            && state.producer_waiting
            && state.queued == 4
            && state.admitted == 13;
        let snapshot = (state.queued, state.admitted, state.checked_hashes);
        state.arm_worker_fault = reached && hooks.fault.is_some();
        drop(state);
        drop(release);
        (handle.join().unwrap(), reached, snapshot)
    })
}

#[test]
fn eight_workers_hold_real_reads_with_four_queued_and_full_manifest_parity() {
    let fixture = eight_fixture();
    let expected = fixture.serial();
    let mut hooks = PipelineHooks::measuring(AdmissionMode::Count);
    hooks.hold_workers = true;
    let hooks = Arc::new(hooks);
    let observation = Observation {
        selected_worker_limit: 8,
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    let (result, reached, snapshot) = run_eight_held(&fixture, &observation, &hooks);
    assert_pipeline_fault_drained(&observation, &hooks);
    assert_eq!(result.unwrap().unwrap(), expected);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
    let (jobs, _) = hooks.admission_snapshot();
    assert_eq!(jobs.len(), expected.files.len());
    for job in &jobs {
        assert!(job.finished && job.valid);
        assert_eq!(
            job.counts,
            ReadCounts {
                opened: 1,
                returned: job.key.expected_bytes,
                accepted: job.key.expected_bytes,
                probes: 0,
                eof: 1
            }
        );
    }
    assert!(
        reached,
        "eight actual held workers never blocked the thirteenth producer descriptor"
    );
    assert_eq!(snapshot, (4, 13, 0));
    assert_eq!(observation.started.load(Ordering::SeqCst), 8);
    assert_eq!(observation.joined.load(Ordering::SeqCst), 8);
}

#[test]
fn eight_workers_full_queue_error_and_panic_drain_before_fallback_or_fatal() {
    let mut cases = Vec::new();
    for kind in [FaultKind::WorkerError, FaultKind::WorkerPanic] {
        let fixture = eight_fixture();
        let hooks = Arc::new(PipelineHooks::fault(kind, &fixture));
        let observation = Observation {
            selected_worker_limit: 8,
            pipeline: Some(hooks.clone()),
            ..Observation::default()
        };
        let (result, reached, snapshot) = run_eight_held(&fixture, &observation, &hooks);
        // Run and clean both cases before any intended RED prerequisite failure.
        cases.push((fixture, hooks, observation, result, reached, snapshot, kind));
    }
    for (fixture, hooks, observation, result, reached, snapshot, kind) in cases {
        assert_pipeline_fault_drained(&observation, &hooks);
        assert!(
            reached,
            "eight-worker full-queue fault prerequisite not reached"
        );
        assert_eq!(snapshot, (4, 13, 0));
        assert_eq!(observation.started.load(Ordering::SeqCst), 8);
        let state = hooks.state.lock().unwrap();
        assert!(state.worker_fault_entered);
        match kind {
            FaultKind::WorkerError => {
                assert!(state.actual_worker_error);
                assert_eq!(state.fallback_drained, vec![true]);
                assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
                let expected =
                    inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into())
                        .unwrap_err();
                assert_eq!(
                    expected.to_string(),
                    "repository store: hard-linked legacy artifact refused"
                );
                assert_eq!(
                    result.unwrap().unwrap_err().to_string(),
                    expected.to_string()
                );
            }
            FaultKind::WorkerPanic => {
                assert!(state.fallback_drained.is_empty());
                assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
                assert_eq!(
                    result.unwrap().unwrap_err().to_string(),
                    "repository store: parallel inventory worker panicked"
                );
            }
            _ => unreachable!(),
        }
        drop(state);
        if matches!(kind, FaultKind::WorkerError) {
            fs::remove_file(&hooks.fault.as_ref().unwrap().alias).unwrap();
        }
        let fresh = Observation {
            selected_worker_limit: 8,
            ..Observation::default()
        };
        let result = pipeline_red_candidate(&fixture, &fresh).unwrap().unwrap();
        fresh.drained();
        assert_eq!(result, fixture.serial());
        assert_eq!(fresh.started.load(Ordering::SeqCst), 8);
        assert_eq!(fresh.fallbacks.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn eight_workers_partial_spawn_refusal_joins_six_and_fresh_call_succeeds() {
    let fixture = eight_fixture();
    let expected = fixture.serial();
    let hooks = Arc::new(PipelineHooks::new(false, false));
    let observation = Observation {
        selected_worker_limit: 8,
        pipeline: Some(hooks.clone()),
        ..Observation::default()
    };
    observation.spawn_refuse_at.store(6, Ordering::SeqCst);
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {},
    );
    observation.drained();
    let fresh = Observation {
        selected_worker_limit: 8,
        ..Observation::default()
    };
    let recovered = pipeline_red_candidate(&fixture, &fresh).unwrap().unwrap();
    fresh.drained();
    assert_eq!(result.unwrap(), expected);
    assert_eq!(recovered, expected);
    assert!(
        observation.spawn_refusal_entered.load(Ordering::SeqCst),
        "spawn index six was never attempted"
    );
    assert_eq!(observation.started.load(Ordering::SeqCst), 6);
    assert_eq!(observation.joined.load(Ordering::SeqCst), 6);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
    assert_eq!(fresh.started.load(Ordering::SeqCst), 8);
    assert_eq!(fresh.fallbacks.load(Ordering::SeqCst), 0);
    assert_eq!(fresh.hashed.load(Ordering::SeqCst), expected.files.len());
    assert!(!fresh.spawn_refusal_entered.load(Ordering::SeqCst));
}

#[test]
fn eight_workers_preserve_global_first_exact_and_late_prefix_quotas() {
    let fixture = admission_fixture(&[b"abcd".to_vec(), b"efgh".to_vec()]);
    for limits in [
        Limits {
            bytes: 3,
            ..Limits::default()
        },
        Limits {
            files: 0,
            ..Limits::default()
        },
        Limits {
            directories: 0,
            ..Limits::default()
        },
    ] {
        let (hooks, mut observation) = measured_observation(AdmissionMode::Count);
        observation.selected_worker_limit = 8;
        let result = candidate(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || panic!("first refusal must precede enumeration callback"),
        );
        observation.drained();
        assert!(result.unwrap().is_none());
        assert!(hooks.admission_snapshot().0.is_empty());
        assert_eq!(observation.started.load(Ordering::SeqCst), 0);
        assert_eq!(observation.hashed.load(Ordering::SeqCst), 0);
    }
    let mut cases = Vec::new();
    for (limits, late) in [
        (
            Limits {
                bytes: 8,
                files: 2,
                directories: 4,
            },
            false,
        ),
        (
            Limits {
                bytes: 7,
                files: 2,
                directories: 4,
            },
            true,
        ),
        (
            Limits {
                bytes: 8,
                files: 1,
                directories: 4,
            },
            true,
        ),
        (
            Limits {
                bytes: 8,
                files: 2,
                directories: 2,
            },
            true,
        ),
    ] {
        let fixture = admission_fixture(&[b"abcd".to_vec(), b"efgh".to_vec()]);
        let expected = fixture.serial();
        let mode = if late {
            AdmissionMode::Prefix
        } else {
            AdmissionMode::Count
        };
        let (hooks, mut observation) = measured_observation(mode);
        observation.selected_worker_limit = 8;
        let result = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || {},
        );
        cases.push((late, expected, hooks, observation, result));
    }
    // Exercise every quota axis before any intended barrier RED assertion.
    for (late, expected, hooks, observation, result) in cases {
        observation.drained();
        assert_eq!(result.unwrap(), expected);
        let (jobs, reached) = hooks.admission_snapshot();
        if late {
            assert!(
                reached,
                "late-limit producer advanced without first real checked hash completion"
            );
            assert_eq!(jobs.len(), 1, "inadmissible second payload was reserved");
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
            assert_eq!(hooks.state.lock().unwrap().fallback_drained, vec![true]);
        } else {
            assert_eq!(jobs.len(), 2);
            assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        }
        for (index, job) in jobs.iter().enumerate() {
            assert_eq!(
                job.key,
                JobKey {
                    discovery_ordinal: index,
                    origin: index,
                    relative: "sessions/only/events.ndjson".into(),
                    expected_bytes: 4
                }
            );
            assert_eq!(job.reserved_total, 4 * (index as u64 + 1));
            assert_eq!(
                job.counts,
                ReadCounts {
                    opened: 1,
                    returned: 4,
                    accepted: 4,
                    probes: 0,
                    eof: 1
                }
            );
            assert!(job.finished && job.valid);
        }
    }
}
