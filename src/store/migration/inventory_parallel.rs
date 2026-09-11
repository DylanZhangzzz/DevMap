//! Serial discovery with bounded speculative full-file hashing and a serial oracle.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    started: AtomicUsize,
    joined: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    fallbacks: AtomicUsize,
    hashed: AtomicUsize,
    panic_at: AtomicUsize,
    duplicate_result: AtomicBool,
}
#[cfg(test)]
impl Default for Observation {
    fn default() -> Self {
        Self {
            started: AtomicUsize::new(0),
            joined: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            fallbacks: AtomicUsize::new(0),
            hashed: AtomicUsize::new(0),
            panic_at: AtomicUsize::new(usize::MAX),
            duplicate_result: AtomicBool::new(false),
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
        assert!(self.peak.load(Ordering::SeqCst) <= 4);
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

fn candidate(
    workspace: &SourceWorkspace,
    origins: &[FrozenOrigin],
    evaluated_at: &str,
    limits: Limits,
    #[cfg(test)] observation: &Observation,
    after_enumeration: impl FnOnce(),
) -> Result<Option<FrozenManifest>, DevMapError> {
    // The same serial walker owns classification, directories and metadata caps.
    // Any ordinary discovery failure is resolved by one original serial attempt.
    let mut manifest = match inventory_collect(
        workspace,
        origins.to_vec(),
        evaluated_at.to_owned(),
        false,
        limits,
    ) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(None),
    };
    after_enumeration();
    if manifest.files.is_empty() {
        return Ok(Some(manifest));
    }
    let next = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    struct Exit<'a> {
        cancelled: &'a AtomicBool,
        #[cfg(test)]
        observation: &'a Observation,
    }
    impl Drop for Exit<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.cancelled.store(true, Ordering::SeqCst);
            }
            #[cfg(test)]
            self.observation.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let files = &manifest.files;
    let roots = &manifest.origins;
    let outputs = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(4);
        let mut spawn_failed = false;
        for _ in 0..4.min(files.len()) {
            let spawned = std::thread::Builder::new()
                .name("devmap-inventory".into())
                .spawn_scoped(scope, || {
                    #[cfg(test)]
                    {
                        observation.started.fetch_add(1, Ordering::SeqCst);
                        let active = observation.active.fetch_add(1, Ordering::SeqCst) + 1;
                        observation.peak.fetch_max(active, Ordering::SeqCst);
                    }
                    let _exit = Exit {
                        cancelled: &cancelled,
                        #[cfg(test)]
                        observation,
                    };
                    // Each ordinal is claimed once. All local result vectors together
                    // contain at most files.len() small digest records, never payloads.
                    let mut results = Vec::new();
                    while !cancelled.load(Ordering::SeqCst) {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        let Some(file) = files.get(index) else {
                            break;
                        };
                        #[cfg(test)]
                        if observation.panic_at.load(Ordering::SeqCst) == index {
                            panic!("controlled inventory worker panic");
                        }
                        let path = roots[file.origin]
                            .git_dir
                            .join("devmap")
                            .join(&file.relative);
                        // <=4 payload readers and <=4x64 KiB streaming buffers;
                        // security checks may open additional temporary handles.
                        // Metadata quotas sum <=512 MiB before spawning.
                        let hashed = inventory_hash(&path, file.bytes);
                        #[cfg(test)]
                        observation.hashed.fetch_add(1, Ordering::SeqCst);
                        let valid = hashed.as_ref().is_ok_and(|(bytes, _)| *bytes == file.bytes);
                        if !valid {
                            cancelled.store(true, Ordering::SeqCst);
                        }
                        #[cfg(test)]
                        if observation.duplicate_result.load(Ordering::SeqCst)
                            && index == 0
                            && let Ok(value) = &hashed
                        {
                            // Duplicate one genuinely successful result without
                            // changing its ordinal, length or digest.
                            results.push((index, Ok(value.clone())));
                        }
                        results.push((index, hashed));
                        if !valid {
                            break;
                        }
                    }
                    results
                });
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(_) => {
                    spawn_failed = true;
                    cancelled.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
        // No result channel: worker panic cannot strand a receiver waiting for
        // a missing result. Explicitly join EVERY handle before deciding outcome.
        let mut joined = Vec::with_capacity(handles.len());
        let mut panicked = false;
        for handle in handles {
            let result = handle.join();
            #[cfg(test)]
            observation.joined.fetch_add(1, Ordering::SeqCst);
            match result {
                Ok(results) => joined.push(results),
                Err(_) => panicked = true,
            }
        }
        if panicked {
            return Err(fail("parallel inventory worker panicked"));
        }
        if spawn_failed || cancelled.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(joined))
    })?;
    let Some(outputs) = outputs else {
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
            file.sha256 = digest;
            complete += 1;
        }
    }
    if complete != manifest.files.len() || !seen.into_iter().all(|value| value) {
        return Err(fail("parallel inventory result missing"));
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
