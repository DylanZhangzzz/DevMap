use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc::sync_channel,
};
use std::time::Duration;

use super::parallel::{Hooks, NoHooks, candidate};

struct Fixture {
    _owned: tempfile::TempDir,
    sources: Vec<SourceWorkspace>,
}
impl Fixture {
    fn new(linked: usize) -> Self {
        fn git(root: &Path, args: &[&str]) {
            let mut command = std::process::Command::new("git");
            command.arg("-C").arg(root).args(args);
            // Child-local isolation: never mutate the test runner environment.
            for (key, _) in std::env::vars_os() {
                if key
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("GIT_")
                {
                    command.env_remove(key);
                }
            }
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "");
            let output = crate::git_process::output(&mut command).unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let owned = tempfile::tempdir().unwrap();
        let main = owned.path().join("main");
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
        let mut roots = vec![main.clone()];
        for index in 0..linked {
            let root = owned.path().join(format!("linked-{index}"));
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-qb",
                    &format!("linked-{index}"),
                    root.to_str().unwrap(),
                ],
            );
            roots.push(root);
        }
        let sources = roots
            .iter()
            .map(|root| {
                crate::git::SourceGitInspector::open(root)
                    .unwrap()
                    .workspace_allow_unborn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for (index, source) in sources.iter().enumerate() {
            let refs = source.git_dir.join("refs/private/deeper");
            fs::create_dir_all(&refs).unwrap();
            fs::write(refs.join("binary-proof"), [0, 255, index as u8, 13, 10]).unwrap();
        }
        Self {
            _owned: owned,
            sources,
        }
    }
}

fn same(actual: &Evidence, expected: &Evidence) {
    assert!(
        actual == expected,
        "complete Evidence differs (maps, identities, bytes, roots, incarnation, all counters)"
    );
}

#[test]
fn stable_main_linked_nested_refs_match_complete_serial_evidence() {
    for linked in [0, 2] {
        let fixture = Fixture::new(linked);
        for source in &fixture.sources {
            same(
                &candidate(source, &NoHooks).unwrap(),
                &Evidence::capture(source).unwrap(),
            );
        }
    }
}

#[derive(Default)]
struct Audit {
    total: AtomicUsize,
    started: AtomicUsize,
    finished: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    ordinals: Mutex<BTreeSet<usize>>,
    admissions: Mutex<Vec<(usize, usize, usize, usize)>>,
}
impl Hooks for Audit {
    fn tasks_ready(&self, total: usize) {
        assert!(total <= 256);
        assert_eq!(
            self.started.load(Ordering::SeqCst),
            0,
            "complete task list precedes payloads"
        );
        self.total.store(total, Ordering::SeqCst);
    }
    fn before_origin(&self, ordinal: usize) -> Result<(), DevMapError> {
        assert!(ordinal < self.total.load(Ordering::SeqCst));
        assert!(self.ordinals.lock().unwrap().insert(ordinal));
        self.started.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        assert!(active <= 4, "more than four origin payloads");
        Ok(())
    }
    fn after_origin(&self, _ordinal: usize) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.finished.fetch_add(1, Ordering::SeqCst);
    }
    fn admitted(&self, bytes: usize, nodes: usize, paths: usize, directories: usize) {
        assert!(
            bytes <= 4 * 1024 * 1024
                && nodes <= 4096
                && paths <= 1024 * 1024
                && directories <= 4096
        );
        self.admissions
            .lock()
            .unwrap()
            .push((bytes, nodes, paths, directories));
    }
}
impl Audit {
    fn joined(&self) {
        assert!(
            self.started.load(Ordering::SeqCst) > 0,
            "candidate never entered instrumented payload"
        );
        assert_eq!(self.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            self.started.load(Ordering::SeqCst),
            self.finished.load(Ordering::SeqCst)
        );
    }
}

#[test]
fn complete_list_unique_ordinals_and_duplicate_charges_match_serial() {
    let fixture = Fixture::new(4);
    let audit = Audit::default();
    let actual = candidate(&fixture.sources[0], &audit).unwrap();
    audit.joined();
    assert_eq!(audit.total.load(Ordering::SeqCst), 5);
    assert_eq!(audit.started.load(Ordering::SeqCst), 5);
    same(&actual, &Evidence::capture(&fixture.sources[0]).unwrap());
    // Common/admin directories are visited repeatedly. Unique map cardinality
    // must not replace call-based path/node/byte charges.
    let admissions = audit.admissions.lock().unwrap();
    assert_eq!(
        admissions.last().copied(),
        Some((
            actual.bytes,
            actual.nodes,
            actual.path_bytes,
            actual.directories.len()
        ))
    );
}

#[test]
fn controlled_payload_handshake_proves_overlap_and_cleanup() {
    struct Handshake {
        audit: Audit,
        tx: std::sync::mpsc::SyncSender<()>,
        rx: Mutex<std::sync::mpsc::Receiver<()>>,
        entered: std::sync::mpsc::SyncSender<()>,
        wait_entered: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl Hooks for Handshake {
        fn tasks_ready(&self, total: usize) {
            self.audit.tasks_ready(total);
        }
        fn before_origin(&self, ordinal: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(ordinal)?;
            if ordinal == 0 {
                self.entered
                    .send(())
                    .map_err(|_| fail("second payload missing"))?;
                self.rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("origin payloads did not overlap"))?;
            } else if ordinal == 1 {
                self.wait_entered
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("first payload did not overlap"))?;
                // One send to capacity one is bounded even if ordinal 0 timed out.
                self.tx
                    .send(())
                    .map_err(|_| fail("overlap receiver missing"))?;
            }
            Ok(())
        }
        fn after_origin(&self, ordinal: usize) {
            self.audit.after_origin(ordinal);
        }
    }
    let fixture = Fixture::new(2);
    let (tx, rx) = sync_channel(1);
    let (entered, wait_entered) = sync_channel(1);
    let hooks = Handshake {
        audit: Audit::default(),
        tx,
        rx: Mutex::new(rx),
        entered,
        wait_entered: Mutex::new(wait_entered),
    };
    let actual = candidate(&fixture.sources[0], &hooks);
    hooks.audit.joined();
    assert!(
        hooks.audit.peak.load(Ordering::SeqCst) >= 2,
        "payload overlap was not observed"
    );
    same(
        &actual.unwrap(),
        &Evidence::capture(&fixture.sources[0]).unwrap(),
    );
}

#[test]
fn aggregate_actual_bytes_and_nodes_are_global_not_per_worker() {
    for bytes_case in [true, false] {
        let fixture = Fixture::new(2);
        for source in &fixture.sources {
            if bytes_case {
                // Each payload is below 4 MiB; aggregate exceeds 4 MiB.
                fs::write(source.git_dir.join("packed-refs"), vec![b'x'; 1024 * 1024]).unwrap();
                fs::write(
                    source.git_dir.join("refs/private/deeper/large"),
                    vec![b'y'; 512 * 1024],
                )
                .unwrap();
            } else {
                // A tree file charges an entry AND a file node: > 4200 globally.
                for index in 0..700 {
                    fs::write(
                        source.git_dir.join(format!("refs/private/deeper/n{index}")),
                        [],
                    )
                    .unwrap();
                }
            }
        }
        let audit = Audit::default();
        let actual = candidate(&fixture.sources[0], &audit);
        audit.joined();
        assert!(
            actual.is_err(),
            "per-worker limits incorrectly authorize an oversized proof"
        );
        assert!(Evidence::capture(&fixture.sources[0]).is_err());
        assert!(!audit.admissions.lock().unwrap().is_empty());
        assert!(
            optional(actual).unwrap().is_none(),
            "ordinary quota failure must retain optional fallback classification"
        );
    }
}

#[test]
fn per_file_limit_and_worktree_config_still_refuse_evidence() {
    for config_case in [true, false] {
        let fixture = Fixture::new(1);
        let admin = &fixture.sources[1].git_dir;
        if config_case {
            fs::write(admin.join("config.worktree"), []).unwrap();
        } else {
            fs::write(admin.join("packed-refs"), vec![0; 1024 * 1024 + 1]).unwrap();
        }
        assert!(
            optional(candidate(&fixture.sources[0], &NoHooks))
                .unwrap()
                .is_none()
        );
        assert!(Evidence::capture(&fixture.sources[0]).is_err());
    }
}

#[test]
fn spawn_refusal_after_started_worker_joins_and_cannot_hide_panic() {
    struct Refusal {
        audit: Audit,
        fallback: AtomicUsize,
        started: std::sync::mpsc::SyncSender<()>,
        wait_started: Mutex<std::sync::mpsc::Receiver<()>>,
        release: std::sync::mpsc::SyncSender<()>,
        wait_release: Mutex<std::sync::mpsc::Receiver<()>>,
        handshake: AtomicUsize,
        panicked: AtomicUsize,
        panic: bool,
    }
    impl Hooks for Refusal {
        fn tasks_ready(&self, n: usize) {
            self.audit.tasks_ready(n);
        }
        fn spawn_ready(&self, slot: usize) -> std::io::Result<()> {
            if slot != 1 {
                return Ok(());
            }
            let started = self
                .wait_started
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            // Release before reporting any refusal, including handshake timeout.
            // A single capacity-one send cannot strand a late-started worker.
            let _ = self.release.send(());
            started.map_err(|_| std::io::Error::other("first worker never entered payload"))?;
            self.handshake.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("injected second spawn refusal"))
        }
        fn before_origin(&self, n: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(n)?;
            if n == 0 {
                self.started
                    .send(())
                    .map_err(|_| fail("spawn coordinator missing"))?;
                self.wait_release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("spawn refusal never released first worker"))?;
                if self.panic {
                    self.panicked.fetch_add(1, Ordering::SeqCst);
                    panic!("first payload panic concurrent with spawn refusal");
                }
            }
            Ok(())
        }
        fn after_origin(&self, n: usize) {
            self.audit.after_origin(n);
        }
        fn serial_fallback(&self) {
            assert_eq!(
                self.audit.active.load(Ordering::SeqCst),
                0,
                "fallback before joins"
            );
            assert!(
                self.audit.started.load(Ordering::SeqCst) >= 1,
                "no owned payload was exercised"
            );
            assert_eq!(
                self.audit.started.load(Ordering::SeqCst),
                self.audit.finished.load(Ordering::SeqCst)
            );
            self.fallback.fetch_add(1, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new(4);
    for panic in [false, true] {
        let (started, wait_started) = sync_channel(1);
        let (release, wait_release) = sync_channel(1);
        let hooks = Refusal {
            audit: Audit::default(),
            fallback: AtomicUsize::new(0),
            started,
            wait_started: Mutex::new(wait_started),
            release,
            wait_release: Mutex::new(wait_release),
            handshake: AtomicUsize::new(0),
            panicked: AtomicUsize::new(0),
            panic,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            candidate(&fixture.sources[0], &hooks)
        }))
        .expect("spawn refusal must join and convert the owned worker panic");
        hooks.audit.joined();
        assert_eq!(
            hooks.handshake.load(Ordering::SeqCst),
            1,
            "second spawn did not observe first payload start"
        );
        assert_eq!(
            hooks.panicked.load(Ordering::SeqCst),
            usize::from(panic),
            "panic branch never reached the injected panic"
        );
        if panic {
            assert!(
                result.is_err(),
                "spawn refusal must not turn an owned worker panic into success"
            );
            assert_eq!(
                hooks.fallback.load(Ordering::SeqCst),
                0,
                "panic must take precedence over serial replay"
            );
        } else {
            assert_eq!(hooks.fallback.load(Ordering::SeqCst), 1);
            same(
                &result.unwrap(),
                &Evidence::capture(&fixture.sources[0]).unwrap(),
            );
        }
    }
}

#[test]
fn ordinary_failure_and_worker_panic_join_and_never_publish_success() {
    struct Fault {
        audit: Audit,
        panic: bool,
    }
    impl Hooks for Fault {
        fn tasks_ready(&self, n: usize) {
            self.audit.tasks_ready(n);
        }
        fn before_origin(&self, n: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(n)?;
            if n == 1 {
                if self.panic {
                    panic!("injected origin payload panic");
                }
                return Err(fail("injected ordinary origin failure"));
            }
            Ok(())
        }
        fn after_origin(&self, n: usize) {
            self.audit.after_origin(n);
        }
    }
    let fixture = Fixture::new(3);
    for panic in [false, true] {
        let hooks = Fault {
            audit: Audit::default(),
            panic,
        };
        let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            candidate(&fixture.sources[0], &hooks)
        }))
        .expect("owned worker panic must be converted after joining");
        hooks.audit.joined();
        assert!(actual.is_err(), "incomplete payload authorized success");
        assert!(optional(actual).unwrap().is_none());
    }
}

#[test]
fn conflicting_merge_replays_serial_after_all_workers_join() {
    struct Conflict {
        audit: Audit,
        common: PathBuf,
        fallback: AtomicUsize,
    }
    impl Hooks for Conflict {
        fn tasks_ready(&self, n: usize) {
            self.audit.tasks_ready(n);
        }
        fn before_origin(&self, n: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(n)
        }
        fn after_origin(&self, n: usize) {
            self.audit.after_origin(n);
        }
        fn before_merge(&self, ordinal: usize, fragment: &mut Evidence) {
            assert_eq!(self.audit.active.load(Ordering::SeqCst), 0);
            if ordinal == 0 {
                fragment
                    .directories
                    .insert(self.common.clone(), "injected conflicting identity".into());
            }
        }
        fn serial_fallback(&self) {
            assert_eq!(self.audit.active.load(Ordering::SeqCst), 0);
            self.fallback.fetch_add(1, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new(2);
    let source = &fixture.sources[0];
    let hooks = Conflict {
        audit: Audit::default(),
        common: safe::checked_canonical_directory(&source.git_common_dir).unwrap(),
        fallback: AtomicUsize::new(0),
    };
    let actual = candidate(source, &hooks).unwrap();
    hooks.audit.joined();
    assert_eq!(hooks.fallback.load(Ordering::SeqCst), 1);
    same(&actual, &Evidence::capture(source).unwrap());
}

#[test]
fn missing_duplicate_or_out_of_range_completion_cannot_publish_partial_proof() {
    struct BadOrdinal {
        total: AtomicUsize,
        mode: usize,
    }
    impl Hooks for BadOrdinal {
        fn tasks_ready(&self, total: usize) {
            self.total.store(total, Ordering::SeqCst);
        }
        fn completion_ordinal(&self, ordinal: usize) -> usize {
            if ordinal != 1 {
                return ordinal;
            }
            match self.mode {
                0 => 0,
                1 => self.total.load(Ordering::SeqCst),
                _ => ordinal,
            }
        }
        fn omit_completion(&self, ordinal: usize) -> bool {
            self.mode == 2 && ordinal == 1
        }
    }
    let fixture = Fixture::new(2);
    for mode in 0..3 {
        assert!(
            candidate(
                &fixture.sources[0],
                &BadOrdinal {
                    total: AtomicUsize::new(0),
                    mode
                }
            )
            .is_err(),
            "malformed completion mode {mode} returned a partial proof"
        );
    }
}

#[test]
fn global_path_bytes_refuse_before_global_node_limit() {
    let fixture = Fixture::new(1);
    for source in &fixture.sources {
        // Checked canonical prefixes permit long fixture paths on Windows.
        let mut directory = safe::checked_canonical_directory(&source.git_dir)
            .unwrap()
            .join("refs");
        for _ in 0..6 {
            directory = directory.join("p".repeat(100));
        }
        fs::create_dir_all(&directory).unwrap();
        for index in 0..900 {
            fs::write(directory.join(format!("n{index}")), []).unwrap();
        }
    }
    let audit = Audit::default();
    let result = candidate(&fixture.sources[0], &audit);
    audit.joined();
    let error = match result {
        Ok(_) => panic!("global path quota accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("path byte limit"), "{error}");
    let last = audit.admissions.lock().unwrap().last().copied().unwrap();
    assert!(
        last.1 < 4096,
        "fixture must exercise path admission before node admission"
    );
}

#[test]
fn nested_proof_overlap_has_four_payload_workers_and_six_total_threads() {
    struct Nested {
        audit: Audit,
        ids: Mutex<Vec<std::thread::ThreadId>>,
        started: std::sync::mpsc::SyncSender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl Nested {
        fn record_thread(&self) {
            let id = std::thread::current().id();
            let mut ids = self.ids.lock().unwrap();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    impl Hooks for Nested {
        fn tasks_ready(&self, total: usize) {
            self.audit.tasks_ready(total);
            self.record_thread();
        }
        fn before_origin(&self, ordinal: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(ordinal)?;
            self.record_thread();
            if ordinal < 4 {
                self.started
                    .send(())
                    .map_err(|_| fail("configuration receiver missing"))?;
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("configuration did not release all payloads"))?;
            }
            Ok(())
        }
        fn after_origin(&self, ordinal: usize) {
            self.audit.after_origin(ordinal);
        }
    }
    let fixture = Fixture::new(4);
    let (started, wait_started) = sync_channel(4);
    let (release, wait_release) = sync_channel(4);
    let hooks = Nested {
        audit: Audit::default(),
        ids: Mutex::new(Vec::new()),
        started,
        release: Mutex::new(wait_release),
    };
    let result = validate_proof_checks(
        || {
            hooks.record_thread();
            let all_started =
                (0..4).all(|_| wait_started.recv_timeout(Duration::from_secs(5)).is_ok());
            // Always release every potential waiter before returning false/error.
            for _ in 0..4 {
                let _ = release.send(());
            }
            if !all_started {
                return Err(fail(
                    "four payload workers were not available during configuration",
                ));
            }
            Ok(true)
        },
        || {
            candidate(&fixture.sources[0], &hooks)
                .map(|evidence| evidence == Evidence::capture(&fixture.sources[0]).unwrap())
        },
        Ok(()),
    );
    hooks.audit.joined();
    assert!(result.unwrap());
    assert_eq!(hooks.audit.peak.load(Ordering::SeqCst), 4);
    assert_eq!(
        hooks.ids.lock().unwrap().len(),
        6,
        "caller + coordinator + four payload workers"
    );
}

// These controls intentionally keep preflight separate from origin payloads.
#[derive(Default)]
struct PreflightAudit {
    started: AtomicUsize,
    fallback: AtomicUsize,
}
impl Hooks for PreflightAudit {
    fn before_origin(&self, _: usize) -> Result<(), DevMapError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn serial_fallback(&self) {
        self.fallback.fetch_add(1, Ordering::SeqCst);
    }
}
fn preflight_refuses(source: &SourceWorkspace, message: &str) {
    let hooks = PreflightAudit::default();
    let error = match candidate(source, &hooks) {
        Ok(_) => panic!("preflight accepted {message}"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), fail(message).to_string());
    assert_eq!(
        hooks.started.load(Ordering::SeqCst),
        0,
        "payload entered before preflight refusal"
    );
    assert_eq!(
        hooks.fallback.load(Ordering::SeqCst),
        0,
        "preflight refusal replayed serial payloads"
    );
}

#[test]
fn malformed_backlink_and_layout_refuse_before_any_payload() {
    for backlink in ["relative/.git", "/absolute/not-a-git-entry"] {
        let fixture = Fixture::new(1);
        fs::write(fixture.sources[1].git_dir.join("gitdir"), backlink).unwrap();
        // Serial currently reads this first; discovery-first must refuse the
        // backlink without reaching any main payload file.
        fs::write(
            fixture.sources[0].git_dir.join("packed-refs"),
            vec![0; 1024 * 1024 + 1],
        )
        .unwrap();
        preflight_refuses(&fixture.sources[0], "origin proof backlink unsupported");
    }
    let fixture = Fixture::new(0);
    let unsupported = fixture._owned.path().join("not-dot-git");
    fs::create_dir(&unsupported).unwrap();
    let source = SourceWorkspace {
        root: fixture.sources[0].root.clone(),
        git_dir: unsupported.clone(),
        git_common_dir: unsupported,
        head: String::new(),
        branch: None,
    };
    preflight_refuses(&source, "origin proof layout unsupported");
}

#[test]
fn discovery_of_257_origins_refuses_exact_limit_before_any_payload() {
    let fixture = Fixture::new(0);
    let common = safe::checked_canonical_directory(&fixture.sources[0].git_common_dir).unwrap();
    let administration = common.join("worktrees");
    fs::create_dir(&administration).unwrap();
    // Bounded discovery-only admin entries: complete absolute .git backlinks.
    // These are not 256 valid real linked worktrees or an acceptance substitute.
    for index in 0..256 {
        let admin = administration.join(format!("synthetic-{index:03}"));
        fs::create_dir(&admin).unwrap();
        let backlink = fixture
            ._owned
            .path()
            .join(format!("synthetic-root-{index:03}"))
            .join(".git");
        assert!(backlink.is_absolute());
        fs::write(admin.join("gitdir"), backlink.to_str().unwrap()).unwrap();
    }
    assert_eq!(fs::read_dir(&administration).unwrap().count(), 256);
    fs::write(common.join("packed-refs"), vec![0; 1024 * 1024 + 1]).unwrap();
    preflight_refuses(&fixture.sources[0], "origin proof worktree limit");
}

#[test]
fn merge_file_identity_bytes_kinds_and_equal_duplicates_obey_replay_contract() {
    struct Merge {
        audit: Audit,
        file: PathBuf,
        file_value: Option<(String, Vec<u8>)>,
        directory: PathBuf,
        directory_value: String,
        mode: usize,
        injected: AtomicUsize,
        fallback: AtomicUsize,
    }
    impl Hooks for Merge {
        fn tasks_ready(&self, n: usize) {
            self.audit.tasks_ready(n);
        }
        fn before_origin(&self, n: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(n)
        }
        fn after_origin(&self, n: usize) {
            self.audit.after_origin(n);
        }
        fn before_merge(&self, ordinal: usize, fragment: &mut Evidence) {
            self.audit.joined();
            if ordinal > 1 {
                return;
            }
            self.injected.fetch_add(1, Ordering::SeqCst);
            // Both fragments supply the exact same actual file and directory
            // keys. Keep charges unchanged: faults affect proof values only.
            fragment
                .files
                .insert(self.file.clone(), self.file_value.clone());
            fragment
                .directories
                .insert(self.directory.clone(), self.directory_value.clone());
            if ordinal == 1 {
                match self.mode {
                    0 => fragment
                        .files
                        .get_mut(&self.file)
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .0
                        .push_str("-different"),
                    1 => fragment
                        .files
                        .get_mut(&self.file)
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .1
                        .push(255),
                    2 => {
                        fragment.files.remove(&self.file);
                        fragment
                            .directories
                            .insert(self.file.clone(), "file-became-directory".into());
                    }
                    3 => {
                        fragment.directories.remove(&self.directory);
                        fragment
                            .files
                            .insert(self.directory.clone(), self.file_value.clone());
                    }
                    4 => {} // Equal duplicates must be accepted without replay.
                    _ => unreachable!(),
                }
            }
        }
        fn serial_fallback(&self) {
            self.audit.joined();
            assert_eq!(
                self.injected.load(Ordering::SeqCst),
                2,
                "conflicting fragment was not exercised"
            );
            self.fallback.fetch_add(1, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new(2);
    let source = &fixture.sources[0];
    let oracle = Evidence::capture(source).unwrap();
    let common = safe::checked_canonical_directory(&source.git_common_dir).unwrap();
    let file = common.join("HEAD");
    let file_value = oracle.files.get(&file).unwrap().clone();
    assert!(file_value.is_some());
    for mode in 0..5 {
        let hooks = Merge {
            audit: Audit::default(),
            file: file.clone(),
            file_value: file_value.clone(),
            directory: common.clone(),
            directory_value: oracle.directories[&common].clone(),
            mode,
            injected: AtomicUsize::new(0),
            fallback: AtomicUsize::new(0),
        };
        let actual = candidate(source, &hooks).unwrap();
        hooks.audit.joined();
        assert_eq!(hooks.injected.load(Ordering::SeqCst), 2);
        assert_eq!(
            hooks.fallback.load(Ordering::SeqCst),
            usize::from(mode != 4),
            "merge mode {mode}"
        );
        same(&actual, &oracle);
        same(&actual, &Evidence::capture(source).unwrap());
    }
}

#[test]
fn spawn_refusal_cannot_replay_away_a_genuine_owned_payload_error() {
    struct RefusalError {
        audit: Audit,
        started: std::sync::mpsc::SyncSender<()>,
        wait_started: Mutex<std::sync::mpsc::Receiver<()>>,
        release: std::sync::mpsc::SyncSender<()>,
        wait_release: Mutex<std::sync::mpsc::Receiver<()>>,
        handshake: AtomicUsize,
        error_reached: AtomicUsize,
        fallback: AtomicUsize,
    }
    impl Hooks for RefusalError {
        fn tasks_ready(&self, n: usize) {
            self.audit.tasks_ready(n);
        }
        fn spawn_ready(&self, slot: usize) -> std::io::Result<()> {
            if slot != 1 {
                return Ok(());
            }
            let entered = self
                .wait_started
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            let _ = self.release.send(());
            entered.map_err(|_| std::io::Error::other("first payload never entered"))?;
            self.handshake.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("injected second spawn refusal"))
        }
        fn before_origin(&self, n: usize) -> Result<(), DevMapError> {
            self.audit.before_origin(n)?;
            if n == 0 {
                self.started
                    .send(())
                    .map_err(|_| fail("coordinator missing"))?;
                self.wait_release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| fail("release timed out"))?;
                self.error_reached.fetch_add(1, Ordering::SeqCst);
                return Err(fail("genuine owned payload error during spawn refusal"));
            }
            Ok(())
        }
        fn after_origin(&self, n: usize) {
            self.audit.after_origin(n);
        }
        fn serial_fallback(&self) {
            self.audit.joined();
            self.fallback.fetch_add(1, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new(4);
    let (started, wait_started) = sync_channel(1);
    let (release, wait_release) = sync_channel(1);
    let hooks = RefusalError {
        audit: Audit::default(),
        started,
        wait_started: Mutex::new(wait_started),
        release,
        wait_release: Mutex::new(wait_release),
        handshake: AtomicUsize::new(0),
        error_reached: AtomicUsize::new(0),
        fallback: AtomicUsize::new(0),
    };
    let actual = candidate(&fixture.sources[0], &hooks);
    hooks.audit.joined();
    assert_eq!(hooks.handshake.load(Ordering::SeqCst), 1);
    assert_eq!(
        hooks.error_reached.load(Ordering::SeqCst),
        1,
        "timeout cannot stand in for genuine payload error"
    );
    assert!(
        actual.is_err(),
        "spawn refusal hid a genuine payload failure"
    );
    assert_eq!(hooks.fallback.load(Ordering::SeqCst), 0);
}
