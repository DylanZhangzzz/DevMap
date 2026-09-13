//! Serial discovery with bounded speculative full-file hashing and a serial oracle.
use super::*;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub(super) bytes: u64,
    pub(super) files: usize,
    pub(super) directories: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: MAX_BYTES,
            files: MAX_FILES,
            directories: MAX_FILES,
        }
    }
}
#[cfg(test)]
struct Observation {
    profile_queue: bool,
    producer_wait_us: AtomicUsize,
    producer_wait_count: AtomicUsize,
    selected_worker_limit: usize,
    started: AtomicUsize,
    joined: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    fallbacks: AtomicUsize,
    hashed: AtomicUsize,
    panic_at: AtomicUsize,
    duplicate_result: AtomicBool,
    result_fault: AtomicUsize,
    result_fault_entered: AtomicBool,
    spawn_refuse_at: AtomicUsize,
    spawn_refusal_entered: AtomicBool,
    pipeline: Option<std::sync::Arc<PipelineHooks>>,
}
#[cfg(test)]
impl Default for Observation {
    fn default() -> Self {
        Self {
            profile_queue: std::env::var("DEVMAP_QUERY_PROFILE_MODE").as_deref()
                == Ok("inventory_phases"),
            producer_wait_us: AtomicUsize::new(0),
            producer_wait_count: AtomicUsize::new(0),
            selected_worker_limit: 4,
            started: AtomicUsize::new(0),
            joined: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            fallbacks: AtomicUsize::new(0),
            hashed: AtomicUsize::new(0),
            panic_at: AtomicUsize::new(usize::MAX),
            duplicate_result: AtomicBool::new(false),
            result_fault: AtomicUsize::new(0),
            result_fault_entered: AtomicBool::new(false),
            spawn_refuse_at: AtomicUsize::new(usize::MAX),
            spawn_refusal_entered: AtomicBool::new(false),
            pipeline: None,
        }
    }
}
#[cfg(test)]
impl Observation {
    fn drained(&self) {
        assert_eq!(self.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            self.started.load(Ordering::SeqCst),
            self.joined.load(Ordering::SeqCst)
        );
        assert!(matches!(self.selected_worker_limit, 4 | 8));
        assert!(self.peak.load(Ordering::SeqCst) <= self.selected_worker_limit);
    }
}

pub(super) fn run(
    workspace: &SourceWorkspace,
    origins: Vec<FrozenOrigin>,
    evaluated_at: String,
) -> Result<FrozenManifest, DevMapError> {
    with_serial_oracle(
        workspace,
        &origins,
        &evaluated_at,
        Limits::default(),
        #[cfg(test)]
        &Observation::default(),
        || {},
    )
}

struct Job {
    ordinal: usize,
    origin: usize,
    relative: String,
    bytes: u64,
}
#[derive(Default)]
struct QueueState {
    jobs: std::collections::VecDeque<Job>,
    finished: bool,
    cancelled: bool,
}
#[derive(Default)]
struct WorkQueue {
    state: std::sync::Mutex<QueueState>,
    work: std::sync::Condvar,
    room: std::sync::Condvar,
    poisoned: AtomicBool,
}
impl WorkQueue {
    fn recover<'a>(
        &self,
        result: std::sync::LockResult<std::sync::MutexGuard<'a, QueueState>>,
    ) -> std::sync::MutexGuard<'a, QueueState> {
        match result {
            Ok(state) => state,
            Err(error) => {
                self.poisoned.store(true, Ordering::SeqCst);
                let mut state = error.into_inner();
                state.cancelled = true;
                self.work.notify_all();
                self.room.notify_all();
                state
            }
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.recover(self.state.lock())
    }
    fn cancel(&self) {
        self.lock().cancelled = true;
        self.work.notify_all();
        self.room.notify_all();
    }
    fn finish(&self) {
        self.lock().finished = true;
        self.work.notify_all();
        self.room.notify_all();
    }
    fn push(&self, job: Job, #[cfg(test)] observation: &Observation) -> Result<(), DevMapError> {
        let mut state = self.lock();
        while state.jobs.len() == 4 && !state.cancelled {
            #[cfg(test)]
            let waiting = observation.profile_queue.then(std::time::Instant::now);
            #[cfg(test)]
            let queued = state.jobs.len();
            drop(state);
            #[cfg(test)]
            if let Some(pipeline) = &observation.pipeline {
                pipeline.queue_waiting(queued, job.ordinal + 1, true);
            }
            // Hooks run outside the queue lock. Rechecking the predicate after
            // reacquiring it makes notifications during the hook non-lossy.
            state = self.lock();
            while state.jobs.len() == 4 && !state.cancelled {
                state = self.recover(self.room.wait(state));
            }
            #[cfg(test)]
            if let Some(waiting) = waiting {
                observation.producer_wait_us.fetch_add(
                    usize::try_from(waiting.elapsed().as_micros()).unwrap(),
                    Ordering::Relaxed,
                );
                observation
                    .producer_wait_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            #[cfg(test)]
            let queued = state.jobs.len();
            drop(state);
            #[cfg(test)]
            if let Some(pipeline) = &observation.pipeline {
                pipeline.queue_waiting(queued, job.ordinal + 1, false);
            }
            state = self.lock();
        }
        if state.cancelled || state.finished {
            return Err(fail("parallel inventory submission cancelled"));
        }
        state.jobs.push_back(job);
        drop(state);
        self.work.notify_one();
        Ok(())
    }
    fn pop(&self, #[cfg(test)] observation: &Observation) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if state.cancelled {
                return None;
            }
            if let Some(job) = state.jobs.pop_front() {
                drop(state);
                self.room.notify_one();
                return Some(job);
            }
            if state.finished {
                return None;
            }
            drop(state);
            #[cfg(test)]
            if let Some(pipeline) = &observation.pipeline {
                pipeline.worker_waiting(true);
            }
            state = self.lock();
            while state.jobs.is_empty() && !state.finished && !state.cancelled {
                state = self.recover(self.work.wait(state));
            }
            drop(state);
            #[cfg(test)]
            if let Some(pipeline) = &observation.pipeline {
                pipeline.worker_waiting(false);
            }
            state = self.lock();
        }
    }
}
struct WorkerExit<'a> {
    queue: &'a WorkQueue,
    #[cfg(test)]
    observation: &'a Observation,
}
impl Drop for WorkerExit<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.queue.cancel();
        }
        #[cfg(test)]
        self.observation.active.fetch_sub(1, Ordering::SeqCst);
    }
}
type HashResult = (usize, Result<(u64, String), DevMapError>);
fn hash_jobs(
    queue: &WorkQueue,
    roots: &[FrozenOrigin],
    #[cfg(test)] observation: &Observation,
) -> Vec<HashResult> {
    #[cfg(test)]
    {
        observation.started.fetch_add(1, Ordering::SeqCst);
        let active = observation.active.fetch_add(1, Ordering::SeqCst) + 1;
        observation.peak.fetch_max(active, Ordering::SeqCst);
    }
    let _exit = WorkerExit {
        queue,
        #[cfg(test)]
        observation,
    };
    // Each claim owns one descriptor. Combined vectors contain at most one
    // small digest per admitted ordinal, except deliberate test corruption.
    let mut results = Vec::new();
    while let Some(job) = queue.pop(
        #[cfg(test)]
        observation,
    ) {
        let index = job.ordinal;
        #[cfg(test)]
        if let Some(pipeline) = &observation.pipeline {
            pipeline.claimed(&FrozenFile {
                origin: job.origin,
                relative: job.relative.clone(),
                bytes: job.bytes,
                sha256: String::new(),
                record_count: 0,
                outcome: String::new(),
            });
        }
        #[cfg(test)]
        if observation.panic_at.load(Ordering::SeqCst) == index {
            panic!("controlled inventory worker panic");
        }
        let path = roots[job.origin].git_dir.join("devmap").join(&job.relative);
        #[cfg(test)]
        if let Some(pipeline) = &observation.pipeline {
            pipeline.before_hash(&path);
        }
        #[cfg(test)]
        let hashed = if let Some(pipeline) = &observation.pipeline {
            pipeline.hash_job(&path, job.bytes)
        } else {
            inventory_candidate_hash(&path, job.bytes)
        };
        #[cfg(not(test))]
        let hashed = inventory_candidate_hash(&path, job.bytes);
        #[cfg(test)]
        observation.hashed.fetch_add(1, Ordering::SeqCst);
        let valid = hashed.as_ref().is_ok_and(|(bytes, _)| *bytes == job.bytes);
        #[cfg(test)]
        if let Some(pipeline) = &observation.pipeline {
            pipeline.after_hash(valid);
        }
        if !valid {
            queue.cancel();
            #[cfg(test)]
            if let Some(pipeline) = &observation.pipeline {
                pipeline.reader_cancelled();
            }
        }
        #[cfg(test)]
        if observation.duplicate_result.load(Ordering::SeqCst)
            && index == 0
            && let Ok(value) = &hashed
        {
            results.push((index, Ok(value.clone())));
        }
        #[cfg(test)]
        let index = if valid && index == 0 {
            match observation.result_fault.load(Ordering::SeqCst) {
                1 => {
                    observation
                        .result_fault_entered
                        .store(true, Ordering::SeqCst);
                    continue;
                }
                2 => {
                    observation
                        .result_fault_entered
                        .store(true, Ordering::SeqCst);
                    // Discovery is still running; this is outside every admitted cap.
                    usize::MAX
                }
                _ => index,
            }
        } else {
            index
        };
        results.push((index, hashed));
        if !valid {
            break;
        }
    }
    results
}

fn candidate(
    workspace: &SourceWorkspace,
    origins: &[FrozenOrigin],
    evaluated_at: &str,
    limits: Limits,
    #[cfg(test)] observation: &Observation,
    after_enumeration: impl FnOnce(),
) -> Result<Option<FrozenManifest>, DevMapError> {
    #[cfg(test)]
    let mut clock = crate::application::query_profile::Clock::inventory();
    #[cfg(test)]
    let worker_limit = {
        assert!(matches!(observation.selected_worker_limit, 4 | 8));
        observation.selected_worker_limit
    };
    #[cfg(not(test))]
    let worker_limit = 4;
    let queue = WorkQueue::default();
    let attempt = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_limit);
        // Catch the entire producer, including spawn and invocation-local hooks.
        // A producer panic must wake idle workers before any join can block.
        let produced = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut submit = |ordinal: usize, file: &FrozenFile| {
                if handles.is_empty() {
                    // Start only after the first descriptor passes every prefix
                    // quota and classification check. No queued open handles.
                    for _worker_index in 0..worker_limit {
                        #[cfg(test)]
                        let spawn_error = (observation.spawn_refuse_at.load(Ordering::SeqCst)
                            == _worker_index)
                            .then(|| {
                                observation
                                    .spawn_refusal_entered
                                    .store(true, Ordering::SeqCst);
                                std::io::Error::other("controlled inventory spawn refusal")
                            });
                        #[cfg(not(test))]
                        let spawn_error: Option<std::io::Error> = None;
                        let spawned = if let Some(error) = spawn_error {
                            Err(error)
                        } else {
                            std::thread::Builder::new()
                                .name("devmap-inventory".into())
                                .spawn_scoped(scope, || {
                                    hash_jobs(
                                        &queue,
                                        origins,
                                        #[cfg(test)]
                                        observation,
                                    )
                                })
                        };
                        match spawned {
                            Ok(handle) => handles.push(handle),
                            Err(_) => {
                                queue.cancel();
                                return Err(fail("parallel inventory worker spawn failed"));
                            }
                        }
                    }
                }
                queue.push(
                    Job {
                        ordinal,
                        origin: file.origin,
                        relative: file.relative.clone(),
                        bytes: file.bytes,
                    },
                    #[cfg(test)]
                    observation,
                )
            };
            let manifest = inventory_collect_emitting(
                workspace,
                origins.to_vec(),
                evaluated_at.to_owned(),
                &mut InventoryWalk {
                    hash: false,
                    limits,
                    #[cfg(test)]
                    pipeline: observation.pipeline.as_deref(),
                    sink: &mut submit,
                },
            )?;
            // Retained only for existing diagnostic callers. Workers may already
            // have hashed jobs; per-job hooks provide deterministic mutations.
            after_enumeration();
            Ok::<_, DevMapError>(manifest)
        }));
        #[cfg(test)]
        clock.mark("discovery_with_overlapping_hashes");
        if matches!(&produced, Ok(Ok(_))) {
            queue.finish();
        } else {
            queue.cancel();
        }
        // No result receiver: join every started worker even after error/panic.
        let mut outputs = Vec::with_capacity(handles.len());
        let mut panicked = false;
        for handle in handles {
            let result = handle.join();
            #[cfg(test)]
            observation.joined.fetch_add(1, Ordering::SeqCst);
            match result {
                Ok(group) => outputs.push(group),
                Err(_) => panicked = true,
            }
        }
        if panicked {
            return Err(fail("parallel inventory worker panicked"));
        }
        if produced.is_err() {
            return Err(fail("parallel inventory producer panicked"));
        }
        let cancelled = queue.lock().cancelled;
        if queue.poisoned.load(Ordering::SeqCst) {
            return Err(fail("parallel inventory queue poisoned"));
        }
        if cancelled {
            return Ok(None);
        }
        match produced {
            Ok(Ok(manifest)) => Ok(Some((manifest, outputs))),
            Ok(Err(_)) => Ok(None),
            Err(_) => unreachable!("producer panic handled after worker joins"),
        }
    })?;
    #[cfg(test)]
    clock.mark("remaining_worker_joins");
    let Some((mut manifest, outputs)) = attempt else {
        return Ok(None);
    };
    let mut seen = vec![false; manifest.files.len()];
    let mut complete = 0usize;
    for group in outputs {
        for (index, result) in group {
            let Some(file) = manifest.files.get_mut(index) else {
                return Err(fail("parallel inventory result ordinal invalid"));
            };
            if std::mem::replace(&mut seen[index], true) {
                return Err(fail("parallel inventory result ordinal duplicate"));
            }
            let (bytes, digest) =
                result.map_err(|_| fail("parallel inventory result incomplete"))?;
            if bytes != file.bytes {
                return Err(fail("parallel inventory result length mismatch"));
            }
            if digest.is_empty() {
                return Err(fail("parallel inventory result digest missing"));
            }
            file.sha256 = digest;
            complete += 1;
        }
    }
    if complete != manifest.files.len() || !seen.into_iter().all(|value| value) {
        return Err(fail("parallel inventory result missing"));
    }
    // Merge uses discovery ordinals. Canonical output order is restored only
    // after exactly one complete, valid result has filled every original slot.
    manifest
        .files
        .sort_by(|a, b| (a.origin, &a.relative).cmp(&(b.origin, &b.relative)));
    #[cfg(test)]
    {
        clock.mark("merge_and_sort");
        clock.finish();
        if observation.profile_queue {
            println!(
                "{}",
                serde_json::json!({
                    "diagnostic": "inventory-queue-wait/1",
                    "wall_us": observation.producer_wait_us.load(Ordering::Relaxed),
                    "waits": observation.producer_wait_count.load(Ordering::Relaxed),
                })
            );
        }
    }
    Ok(Some(manifest))
}

fn with_serial_oracle(
    workspace: &SourceWorkspace,
    origins: &[FrozenOrigin],
    evaluated_at: &str,
    limits: Limits,
    #[cfg(test)] observation: &Observation,
    after_enumeration: impl FnOnce(),
) -> Result<FrozenManifest, DevMapError> {
    let result = candidate(
        workspace,
        origins,
        evaluated_at,
        limits,
        #[cfg(test)]
        observation,
        after_enumeration,
    )?;
    if let Some(manifest) = result {
        return Ok(manifest);
    }
    #[cfg(test)]
    {
        observation.drained();
        if let Some(pipeline) = &observation.pipeline {
            pipeline.before_fallback(observation);
        }
        observation.fallbacks.fetch_add(1, Ordering::SeqCst);
    }
    // Candidate workers and metadata are gone. Retry once, in the original
    // serial discovery/read order; transient observations can differ. Accepted
    // bytes are capped per attempt, not total physical I/O across both attempts.
    inventory_serial(workspace, origins.to_vec(), evaluated_at.to_owned())
}

#[cfg(test)]
#[path = "inventory_parallel/tests.rs"]
mod tests;

#[cfg(test)]
pub(super) use tests::PipelineHooks;

/// Same-core inventory diagnostic. No caller-global selector or production policy.
#[cfg(test)]
pub(super) fn profile_worker_counts(
    workspace: &SourceWorkspace,
    expected: &FrozenManifest,
) -> Result<(), DevMapError> {
    let serial = inventory_serial(
        workspace,
        expected.origins.clone(),
        expected.evaluated_at.clone(),
    )?;
    validate_inventory_equality(&serial, expected)?;
    for (iteration, worker_limit) in [4, 8, 8, 4].into_iter().cycle().take(24).enumerate() {
        // First ABBA block warms both variants; the following five are retained.
        let observation = Observation {
            selected_worker_limit: worker_limit,
            ..Observation::default()
        };
        let git_before = crate::git_process::test_spawn_count();
        let start = std::time::Instant::now();
        let result = with_serial_oracle(
            workspace,
            &expected.origins,
            &expected.evaluated_at,
            Limits::default(),
            &observation,
            || {},
        );
        let wall_us = start.elapsed().as_micros();
        let git_starts = crate::git_process::test_spawn_count() - git_before;
        // Formatting, manifest comparison and assertions are outside the timer.
        println!(
            "{}",
            serde_json::json!({
                "diagnostic": "inventory-workers/1", "iteration": iteration,
                "warmup": iteration < 4, "workers": worker_limit, "wall_us": wall_us,
                "git_starts": git_starts,
                "started": observation.started.load(Ordering::SeqCst),
                "joined": observation.joined.load(Ordering::SeqCst),
                "peak": observation.peak.load(Ordering::SeqCst),
                "hashed": observation.hashed.load(Ordering::SeqCst),
                "fallbacks": observation.fallbacks.load(Ordering::SeqCst),
                "error": result.as_ref().err().map(ToString::to_string)
            })
        );
        observation.drained();
        assert_eq!(git_starts, 0);
        assert_eq!(observation.started.load(Ordering::SeqCst), worker_limit);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 0);
        assert_eq!(
            observation.hashed.load(Ordering::SeqCst),
            serial.files.len()
        );
        assert_eq!(result?, serial);
    }
    Ok(())
}
