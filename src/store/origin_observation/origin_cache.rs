//! Read-only enumeration proof owned by one InputReader. Legacy bytes are never cached here.
use super::*;
use crate::git_relationship::QueryConfiguration;

#[path = "origin_cache_admission.rs"]
mod admission;

#[derive(Default)]
pub(crate) struct ReadOriginCache {
    entry: Option<Proof>,
}
struct Proof {
    source: PathBuf,
    config: QueryConfiguration,
    evidence: Evidence,
    origins: Vec<FrozenOrigin>,
}
#[derive(PartialEq, Eq, Default)]
struct Evidence {
    directories: BTreeMap<PathBuf, String>,
    files: BTreeMap<PathBuf, Option<(String, Vec<u8>)>>,
    roots: Vec<FrozenOrigin>,
    bytes: usize,
    nodes: usize,
    path_bytes: usize,
}
fn optional<T>(result: Result<T, DevMapError>) -> Result<Option<T>, DevMapError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e @ DevMapError::GitProcess(_)) => Err(e),
        Err(_) => Ok(None),
    }
}

impl ReadOriginCache {
    pub(crate) fn is_empty(&self) -> bool {
        self.entry.is_none()
    }
    pub(crate) fn matches_query_configuration(
        &self,
        w: &SourceWorkspace,
        configuration: &QueryConfiguration,
    ) -> bool {
        self.entry.as_ref().is_some_and(|proof| {
            proof.source == w.root && proof.config.same_baseline(configuration)
        })
    }
    pub(crate) fn query_boundary(
        &self,
        w: &SourceWorkspace,
        configuration: &QueryConfiguration,
    ) -> Result<(bool, bool), DevMapError> {
        let proof = self
            .entry
            .as_ref()
            .ok_or_else(|| fail("query epoch origin proof missing"))?;
        if proof.source != w.root || !proof.config.same_baseline(configuration) {
            return Err(fail("query epoch origin proof mismatch"));
        }
        let source_valid = std::cell::Cell::new(false);
        let origin_valid = validate_proof_checks(
            || {
                let valid = configuration.recheck_pair(&proof.config)?;
                source_valid.set(valid);
                Ok(valid)
            },
            || {
                Ok(optional(parallel::candidate(w, &parallel::NoHooks))?
                    .is_some_and(|e| e == proof.evidence))
            },
            Ok(()),
        )?;
        Ok((source_valid.get(), origin_valid))
    }
    pub(crate) fn observe_in_query_epoch(
        &self,
        w: &SourceWorkspace,
        c: &Connection,
    ) -> Result<ActiveOriginReport, DevMapError> {
        let proof = self
            .entry
            .as_ref()
            .ok_or_else(|| fail("query epoch origin proof missing"))?;
        if proof.source != w.root {
            return Err(fail("query epoch origin proof mismatch"));
        }
        // An invocation-owned epoch encloses the whole projection with full
        // boundary checks. The complete observer and all legacy hashes remain.
        observe_active_origins_using(w, c, false, || Ok(proof.origins.clone()))
    }
    pub(crate) fn clear(&mut self) {
        self.entry = None;
    }
    pub(crate) fn observe(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
    ) -> Result<ActiveOriginReport, DevMapError> {
        self.observe_checked(w, c, || {})
    }
    pub(crate) fn observe_checked(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
        after_first: impl FnOnce(),
    ) -> Result<ActiveOriginReport, DevMapError> {
        let result = self.observe_inner(w, c, after_first);
        if result.is_err() {
            self.clear();
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn has_entry(&self) -> bool {
        self.entry.is_some()
    }
    fn observe_inner(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
        after_first: impl FnOnce(),
    ) -> Result<ActiveOriginReport, DevMapError> {
        if let Some(proof) = &self.entry
            && !proof.valid(w)?
        {
            self.clear();
        }
        if self.entry.is_none() {
            self.entry = optional(Proof::acquire(w))?.flatten();
        }
        let Some(proof) = &self.entry else {
            return observe_active_origins(w, c, false);
        };
        let mut after_first = Some(after_first);
        // The immutable proof was checked at entry (or just acquired inside
        // its scan sandwich). Recheck it once after the entire observation,
        // before publishing any result. Provider calls reuse that same epoch;
        // they are not independent live scans. Legacy hashes and unavailable
        // path checks inside the observer remain unchanged.
        let result = observe_active_origins_using(w, c, false, || {
            let origins = proof.origins.clone();
            if let Some(probe) = after_first.take() {
                probe();
            }
            Ok(origins)
        })?;
        if !proof.valid(w)? {
            return Err(fail("origin enumeration proof changed after observation"));
        }
        Ok(result)
    }
}
impl Proof {
    fn acquire(w: &SourceWorkspace) -> Result<Option<Self>, DevMapError> {
        let Some(config) = QueryConfiguration::acquire(w)? else {
            return Ok(None);
        };
        let before = parallel::candidate(w, &parallel::NoHooks)?;
        // Authoritative enumeration occurs inside the physical/config sandwich.
        let origins = current_origins(w)?;
        // Compare physical locations using checked canonical paths, retaining
        // Git's original path spelling in the returned observation.
        let mut physical_origins = origins.clone();
        for origin in &mut physical_origins {
            origin.workspace_path = safe::checked_canonical_directory(&origin.workspace_path)?;
        }
        let after = parallel::candidate(w, &parallel::NoHooks)?;
        if before != after || before.roots != physical_origins || !config.recheck()? {
            return Ok(None);
        }
        Ok(Some(Self {
            source: w.root.clone(),
            config,
            evidence: before,
            origins,
        }))
    }
    fn valid(&self, w: &SourceWorkspace) -> Result<bool, DevMapError> {
        if self.source != w.root {
            return Ok(false);
        }
        validate_proof_checks(
            || self.config.recheck(),
            || {
                Ok(optional(parallel::candidate(w, &parallel::NoHooks))?
                    .is_some_and(|e| e == self.evidence))
            },
            Ok(()),
        )
    }
}

/// Capture was already non-atomic; overlap changes transient observation order
/// and may perform one bounded evidence capture despite invalid configuration.
/// Configuration results retain precedence over evidence results.
/// `spawn_ready` is a private test seam: injected refusal covers the same
/// fallback branch as OS spawn failure, without exhausting system threads.
fn validate_proof_checks(
    config: impl FnOnce() -> Result<bool, DevMapError>,
    evidence: impl Fn() -> Result<bool, DevMapError> + Sync,
    spawn_ready: std::io::Result<()>,
) -> Result<bool, DevMapError> {
    std::thread::scope(|scope| {
        // Borrow an Fn: failed spawn drops its closure, but the original
        // evidence operation remains reusable for the serial fallback.
        let worker =
            spawn_ready.and_then(|()| std::thread::Builder::new().spawn_scoped(scope, &evidence));
        let Ok(worker) = worker else {
            if !config()? {
                return Ok(false);
            }
            return evidence();
        };
        let config_result = config();
        // Always join, even for configuration false/Err, before choosing a
        // result. A panicked or incomplete worker can never authorize true.
        let evidence_result = worker
            .join()
            .unwrap_or_else(|_| Err(fail("origin proof evidence worker panicked")));
        if !config_result? {
            return Ok(false);
        }
        evidence_result
    })
}

/// Independent read-only diagnostics, not additive production spans.
#[cfg(test)]
pub(super) fn profile_proof_stages(
    w: &SourceWorkspace,
    iteration: usize,
    mut report: impl FnMut(&'static str, u128, usize),
) -> Result<(), DevMapError> {
    let proof = Proof::acquire(w)?.ok_or_else(|| fail("profiling requires origin proof"))?;
    let mut components = Vec::with_capacity(4);
    let count = crate::git_process::test_spawn_count();
    let start = std::time::Instant::now();
    // The component records are nested in this single independent recheck.
    // Buffer them so output formatting/I/O does not inflate the enclosing total.
    // Never add them to the enclosing total or to other independent calls.
    let valid = proof
        .config
        .profile_recheck_components(|stage, wall_us, starts| {
            components.push((stage, wall_us, starts));
        })?;
    report(
        "origin_proof_configuration_recheck",
        start.elapsed().as_micros(),
        crate::git_process::test_spawn_count() - count,
    );
    if !valid {
        return Err(fail("profile origin configuration changed"));
    }
    for (stage, wall_us, starts) in components {
        report(stage, wall_us, starts);
    }
    // Independent paired capture attribution retains the unchanged serial oracle.
    for serial in [iteration.is_multiple_of(2), !iteration.is_multiple_of(2)] {
        let count = crate::git_process::test_spawn_count();
        let start = std::time::Instant::now();
        let mut phases = None;
        let evidence = if serial {
            Evidence::capture(w)?
        } else {
            let profile = parallel::profile_candidate(w)?;
            phases = Some((
                profile.discovery_us,
                profile.payload_us,
                profile.finishing_us,
            ));
            profile.evidence
        };
        report(
            if serial {
                "origin_proof_serial_evidence_capture"
            } else {
                "origin_proof_evidence_capture"
            },
            start.elapsed().as_micros(),
            crate::git_process::test_spawn_count() - count,
        );
        if evidence != proof.evidence {
            return Err(fail("profile origin evidence changed"));
        }
        if let Some((discovery, payload, finishing)) = phases {
            report("origin_proof_discovery", discovery, 0);
            report("origin_proof_parallel_payload", payload, 0);
            report("origin_proof_finishing", finishing, 0);
        }
    }
    for serial in [iteration.is_multiple_of(2), !iteration.is_multiple_of(2)] {
        let count = crate::git_process::test_spawn_count();
        let start = std::time::Instant::now();
        let valid = if serial {
            proof.source == w.root
                && proof.config.recheck()?
                && optional(parallel::candidate(w, &parallel::NoHooks))?
                    .is_some_and(|e| e == proof.evidence)
        } else {
            proof.valid(w)?
        };
        report(
            if serial {
                "origin_proof_serial_complete_recheck"
            } else {
                "origin_proof_complete_recheck"
            },
            start.elapsed().as_micros(),
            crate::git_process::test_spawn_count() - count,
        );
        if !valid {
            return Err(fail("profile origin proof changed"));
        }
    }
    Ok(())
}
impl Evidence {
    fn charge_path(&mut self, path: &Path) -> Result<(), DevMapError> {
        let bytes = self
            .path_bytes
            .checked_add(path.as_os_str().as_encoded_bytes().len())
            .ok_or_else(|| fail("origin proof path byte limit"))?;
        if bytes > 1024 * 1024 {
            return Err(fail("origin proof path byte limit"));
        }
        self.path_bytes = bytes;
        Ok(())
    }
    fn directory(&mut self, path: &Path) -> Result<PathBuf, DevMapError> {
        let path = safe::checked_canonical_directory(path)?;
        self.charge_path(&path)?;
        self.directories.insert(
            path.clone(),
            safe::checked_directory_identity(&path)?.stable_text(),
        );
        if self.directories.len() > 4096 {
            return Err(fail("origin proof directory limit"));
        }
        Ok(path)
    }
    fn file(&mut self, path: &Path) -> Result<(), DevMapError> {
        self.charge_path(path)?;
        self.nodes += 1;
        if self.nodes > 4096 {
            return Err(fail("origin proof file limit"));
        }
        let value = if let Some(meta) = safe::checked_metadata(path)? {
            if !meta.is_file() || meta.len() > 1024 * 1024 {
                return Err(fail("origin proof file unsupported"));
            }
            let file = safe::checked_file(path, false, false)?;
            let identity = safe::file_identity(&file)?.stable_text();
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            self.bytes += bytes.len();
            if bytes.len() > 1024 * 1024 || self.bytes > 4 * 1024 * 1024 {
                return Err(fail("origin proof byte limit"));
            }
            if safe::file_identity(&safe::checked_file(path, false, false)?)?.stable_text()
                != identity
            {
                return Err(fail("origin proof file changed"));
            }
            Some((identity, bytes))
        } else {
            None
        };
        self.files.insert(path.to_owned(), value);
        Ok(())
    }
    fn tree(&mut self, path: &Path, depth: usize) -> Result<(), DevMapError> {
        if depth > 16 {
            return Err(fail("origin proof depth limit"));
        }
        if safe::checked_metadata(path)?.is_none() {
            return self.file(path);
        }
        self.directory(path)?;
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            self.nodes += 1;
            if self.nodes > 4096 {
                return Err(fail("origin proof entry limit"));
            }
            if safe::checked_metadata(&path)?
                .ok_or_else(|| fail("origin proof disappeared"))?
                .is_dir()
            {
                self.tree(&path, depth + 1)?;
            } else {
                self.file(&path)?;
            }
        }
        Ok(())
    }
    fn origin(&mut self, common: &Path, admin: &Path, root: &Path) -> Result<(), DevMapError> {
        let physical_root = self.directory(root)?;
        let admin = self.directory(admin)?;
        let source = SourceWorkspace {
            root: root.to_owned(),
            git_dir: admin.clone(),
            git_common_dir: common.to_owned(),
            head: String::new(),
            branch: None,
        };
        validate_origin_links(&source)?;
        for name in ["HEAD", "packed-refs", "locked", "commondir", "gitdir"] {
            self.file(&admin.join(name))?;
        }
        if safe::checked_metadata(&admin.join("config.worktree"))?.is_some() {
            return Err(fail("origin proof worktree config unsupported"));
        }
        self.file(&admin.join("config.worktree"))?;
        if admin != common {
            self.file(&root.join(".git"))?;
        }
        self.tree(&admin.join("refs"), 0)?;
        self.roots.push(FrozenOrigin {
            worktree_id: worktrees::origin_id(&worktrees::repository_id(&source), &admin),
            git_dir: admin,
            workspace_path: physical_root,
            incarnation: journal::worktree_incarnation(&source)?,
        });
        Ok(())
    }
    fn capture(w: &SourceWorkspace) -> Result<Self, DevMapError> {
        let mut result = Self::default();
        let common = result.directory(&w.git_common_dir)?;
        if common.file_name().and_then(|n| n.to_str()) != Some(".git") {
            return Err(fail("origin proof layout unsupported"));
        }
        let main = common
            .parent()
            .ok_or_else(|| fail("origin proof main missing"))?;
        // Main derivation is only a candidate until compared with original scan.
        result.origin(&common, &common, main)?;
        let objects = result.directory(&common.join("objects"))?;
        let _access = fs::read_dir(objects)?;
        let administration = common.join("worktrees");
        if safe::checked_metadata(&administration)?.is_some() {
            result.directory(&administration)?;
            let mut count = 1usize;
            for entry in fs::read_dir(&administration)? {
                count += 1;
                if count > 256 {
                    return Err(fail("origin proof worktree limit"));
                }
                let admin = result.directory(&entry?.path())?;
                let backlink = pointer_text(&admin.join("gitdir"))?;
                let backlink = PathBuf::from(backlink);
                if !backlink.is_absolute()
                    || backlink.file_name().and_then(|n| n.to_str()) != Some(".git")
                {
                    return Err(fail("origin proof backlink unsupported"));
                }
                let root = backlink
                    .parent()
                    .ok_or_else(|| fail("origin proof root missing"))?;
                result.origin(&common, &admin, root)?;
            }
        } else {
            result.file(&administration)?;
        }
        result.roots.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
        Ok(result)
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn exhausted_path_budget_refuses_existing_directory_and_absent_file() {
        let owned = tempfile::tempdir().unwrap();
        let mut directories = Evidence {
            path_bytes: 1024 * 1024,
            ..Evidence::default()
        };
        assert!(directories.directory(owned.path()).is_err());
        assert!(directories.directories.is_empty());
        let mut files = Evidence {
            path_bytes: 1024 * 1024,
            ..Evidence::default()
        };
        assert!(files.file(&owned.path().join("absent")).is_err());
        assert!(files.files.is_empty());
    }
}

#[cfg(test)]
mod proof_overlap_tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    };
    use std::time::Duration;

    fn outcome(which: usize, label: &str) -> Result<bool, DevMapError> {
        match which {
            0 => Ok(false),
            1 => Ok(true),
            2 => Err(fail(label)),
            _ => unreachable!(),
        }
    }

    fn serial_oracle(config: usize, evidence: usize) -> Result<bool, DevMapError> {
        if !outcome(config, "configuration error")? {
            return Ok(false);
        }
        outcome(evidence, "evidence error")
    }

    fn comparable(value: Result<bool, DevMapError>) -> Result<bool, String> {
        value.map_err(|error| error.to_string())
    }

    #[test]
    fn stable_boolean_and_error_results_match_serial_oracle_and_join() {
        for config in 0..3 {
            for evidence in 0..3 {
                let finished = AtomicUsize::new(0);
                let actual = validate_proof_checks(
                    || outcome(config, "configuration error"),
                    || {
                        finished.fetch_add(1, Ordering::SeqCst);
                        outcome(evidence, "evidence error")
                    },
                    Ok(()),
                );
                assert_eq!(
                    comparable(actual),
                    comparable(serial_oracle(config, evidence))
                );
                // False and configuration error must also await the worker.
                assert_eq!(finished.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[test]
    fn independent_checks_overlap_using_bounded_handshake() {
        let (evidence_started, wait_evidence) = sync_channel(1);
        let (config_started, wait_config) = sync_channel(1);
        let wait_config = Mutex::new(wait_config);
        let finished = AtomicUsize::new(0);
        let caller = std::thread::current().id();
        let result = validate_proof_checks(
            || {
                let received = wait_evidence.recv_timeout(Duration::from_secs(5));
                // Release even on timeout. One send per capacity-one channel
                // cannot block cleanup. No speed threshold is asserted.
                let _ = config_started.send(());
                received.map_err(|_| fail("evidence did not overlap configuration"))?;
                Ok(true)
            },
            || {
                assert_ne!(std::thread::current().id(), caller);
                evidence_started
                    .send(())
                    .map_err(|_| fail("configuration receiver gone"))?;
                wait_config
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("configuration did not release evidence"))?;
                finished.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            },
            Ok(()),
        );
        assert_eq!(comparable(result), Ok(true));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn injected_spawn_refusal_uses_original_serial_short_circuit() {
        for config in 0..3 {
            for evidence in 0..3 {
                let calls = AtomicUsize::new(0);
                let caller = std::thread::current().id();
                let actual = validate_proof_checks(
                    || outcome(config, "configuration error"),
                    || {
                        assert_eq!(std::thread::current().id(), caller);
                        calls.fetch_add(1, Ordering::SeqCst);
                        outcome(evidence, "evidence error")
                    },
                    Err(std::io::Error::other("injected pre-spawn refusal")),
                );
                assert_eq!(
                    comparable(actual),
                    comparable(serial_oracle(config, evidence))
                );
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(config == 1));
            }
        }
    }

    #[test]
    fn optional_evidence_preserves_ordinary_and_git_error_classification() {
        fn evidence(which: usize) -> Result<bool, DevMapError> {
            let captured = match which {
                0 => Ok(false),
                1 => Ok(true),
                2 => Err(fail("ordinary evidence capture failure")),
                3 => Err(DevMapError::GitProcess(
                    crate::git_process::GitProcessError::Deadline,
                )),
                _ => unreachable!(),
            };
            Ok(optional(captured)?.is_some_and(|matches| matches))
        }
        for config in 0..3 {
            for capture in 0..4 {
                let oracle = match outcome(config, "configuration error") {
                    Ok(true) => evidence(capture),
                    other => other,
                };
                let actual = validate_proof_checks(
                    || outcome(config, "configuration error"),
                    || evidence(capture),
                    Ok(()),
                );
                assert_eq!(comparable(actual), comparable(oracle));
            }
        }
    }

    #[test]
    fn caller_configuration_panic_still_joins_owned_worker() {
        struct ReleaseOnUnwind(std::sync::mpsc::SyncSender<()>);
        impl Drop for ReleaseOnUnwind {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        struct Finished<'a>(&'a AtomicUsize);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let finished = AtomicUsize::new(0);
        let caller = std::thread::current().id();
        let (started, wait_started) = sync_channel(1);
        let (release, wait_release) = sync_channel(1);
        let wait_release = Mutex::new(wait_release);
        let result = std::panic::catch_unwind(|| {
            validate_proof_checks(
                || {
                    // The worker cannot finish until configuration unwinds.
                    // Capacity one and a single send keep drop cleanup bounded.
                    let _release_on_unwind = ReleaseOnUnwind(release);
                    wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
                    panic!("caller configuration panic");
                },
                || {
                    let _finished = Finished(&finished);
                    assert_ne!(std::thread::current().id(), caller);
                    started.send(()).unwrap();
                    wait_release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    Ok(true)
                },
                Ok(()),
            )
        });
        let panic = result.expect_err("configuration panic must propagate after scope cleanup");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"caller configuration panic")
        );
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn owned_worker_panic_is_joined_and_never_authorizes_success() {
        struct Finished<'a>(&'a AtomicUsize);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        for config in 0..3 {
            let finished = AtomicUsize::new(0);
            let caller = std::thread::current().id();
            let (release, wait_release) = sync_channel(1);
            let wait_release = Mutex::new(wait_release);
            let result = std::panic::catch_unwind(|| {
                validate_proof_checks(
                    || {
                        release.send(()).unwrap();
                        outcome(config, "configuration error")
                    },
                    || {
                        let _finished = Finished(&finished);
                        assert_ne!(std::thread::current().id(), caller);
                        wait_release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        panic!("owned proof worker panic");
                    },
                    Ok(()),
                )
            })
            .expect("worker panic must be converted after joining, not unwind the caller");
            assert_eq!(finished.load(Ordering::SeqCst), 1);
            let expected = match config {
                0 => Ok(false),
                1 => Err(fail("origin proof evidence worker panicked")),
                2 => Err(fail("configuration error")),
                _ => unreachable!(),
            };
            assert_eq!(comparable(result), comparable(expected));
        }
    }
}

#[path = "origin_cache_parallel.rs"]
mod parallel;

#[cfg(test)]
#[path = "origin_cache_parallel_red_tests.rs"]
mod parallel_evidence_tests;
