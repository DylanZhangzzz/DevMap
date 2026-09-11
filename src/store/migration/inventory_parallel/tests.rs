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
fn growth_and_shrink_after_enumeration_join_before_one_serial_oracle() {
    for replacement in [b"quota-original-grown".as_slice(), b"x".as_slice()] {
        let fixture = Fixture::new();
        let observation = Observation::default();
        let mut changed = false;
        let actual = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            Limits::default(),
            &observation,
            || {
                fs::write(&fixture.changed_file, replacement).unwrap();
                changed = true;
            },
        )
        .unwrap();
        assert!(changed, "actual post-enumeration mutation must execute");
        observation.drained();
        assert!(observation.started.load(Ordering::SeqCst) > 0);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
        assert_eq!(actual, fixture.serial());
    }
}

#[test]
fn small_speculative_quota_declines_to_single_serial_oracle() {
    let fixture = Fixture::new();
    let expected = fixture.serial();
    let observation = Observation::default();
    let actual = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits {
            bytes: 7,
            ..Limits::default()
        },
        &observation,
        || panic!("over-budget candidate must decline before callback"),
    )
    .unwrap();
    assert_eq!(actual, expected);
    observation.drained();
    assert_eq!(observation.started.load(Ordering::SeqCst), 0);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
}

#[test]
fn checked_file_error_joins_before_original_serial_error() {
    let fixture = Fixture::new();
    let observation = Observation::default();
    let mut changed = false;
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {
            fs::hard_link(
                &fixture.changed_file,
                fixture._owned.path().join("retained-alias"),
            )
            .unwrap();
            changed = true;
        },
    );
    assert!(
        changed,
        "hardlink change must actually precede worker reads"
    );
    let expected =
        inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into()).unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), expected.to_string());
    observation.drained();
    assert!(observation.started.load(Ordering::SeqCst) > 0);
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
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
fn exact_aggregate_and_global_entry_limits_are_checked_before_workers() {
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
    for limits in [
        Limits {
            bytes: bytes - 1,
            ..exact
        },
        Limits {
            files: exact.files - 1,
            ..exact
        },
        Limits {
            directories: exact.directories - 1,
            ..exact
        },
    ] {
        let observation = Observation::default();
        let actual = with_serial_oracle(
            &fixture.workspace,
            &fixture.origins,
            STAMP,
            limits,
            &observation,
            || panic!("over-budget discovery cannot start workers"),
        )
        .unwrap();
        assert_eq!(actual, expected);
        observation.drained();
        assert_eq!(observation.started.load(Ordering::SeqCst), 0);
        assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn ordinary_errors_in_multiple_origins_use_the_serial_oracle() {
    let fixture = Fixture::new();
    let observation = Observation::default();
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {
            for (index, origin) in fixture.origins.iter().enumerate() {
                fs::hard_link(
                    origin.git_dir.join("devmap/sessions/s3/events.ndjson"),
                    fixture
                        ._owned
                        .path()
                        .join(format!("retained-alias-{index}")),
                )
                .unwrap();
            }
        },
    );
    let direct =
        inventory_serial(&fixture.workspace, fixture.origins.clone(), STAMP.into()).unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), direct.to_string());
    observation.drained();
    assert_eq!(observation.fallbacks.load(Ordering::SeqCst), 1);
}

#[cfg(windows)]
#[test]
fn owned_windows_junction_after_enumeration_refuses_and_preserves_target() {
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
    let observation = Observation::default();
    let mut replaced = false;
    let result = with_serial_oracle(
        &fixture.workspace,
        &fixture.origins,
        STAMP,
        Limits::default(),
        &observation,
        || {
            // Both endpoints are inside this newly owned fixture. Preserve the
            // original directory; never move/delete an externally resolved tree.
            fs::rename(directory, &retained).unwrap();
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                // cmd's mklink interprets forward-slash absolute paths as
                // switches. Normalize only these already verified owned paths.
                .arg(directory.to_str().unwrap().replace('/', "\\"))
                .arg(target.to_str().unwrap().replace('/', "\\"))
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            replaced = true;
        },
    );
    assert!(
        replaced,
        "real owned Windows junction must be created after enumeration"
    );
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
