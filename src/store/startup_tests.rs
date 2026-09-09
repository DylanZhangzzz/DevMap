use super::*;

fn source() -> (tempfile::TempDir, SourceWorkspace) {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let workspace = SourceGitInspector::open(dir.path())
        .unwrap()
        .workspace()
        .unwrap();
    (dir, workspace)
}

#[test]
fn startup_empty_proof_preserves_zero_byte_and_unknown_artifacts() {
    for relative in [
        "sessions/s/events.ndjson",
        "sessions/s",
        "route-plans.jsonl",
        "task-binding-watermarks.json",
        "unknown/pending",
    ] {
        let (_repo, w) = source();
        let path = w.git_common_dir.join("devmap").join(relative);
        if relative == "sessions/s" {
            fs::create_dir_all(&path).unwrap();
        } else {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, []).unwrap();
        }
        assert!(matches!(
            prepare_first_write(&w).unwrap(),
            WriteBackend::LegacyPreserved(_)
        ));
        assert!(path.exists());
        assert!(!w.git_common_dir.join("devmap/devmap.db").exists());
        assert!(
            !w.git_common_dir
                .join("devmap/backend-transition/attempt.json")
                .exists()
        );
    }
}

#[test]
fn startup_unowned_shadow_is_not_an_empty_repository() {
    let (_repo, w) = source();
    let store = RepositoryStore::open(&w).unwrap();
    assert_eq!(store.generation().unwrap(), 0);
    assert!(
        prepare_first_write(&w)
            .unwrap_err()
            .to_string()
            .contains("unowned shadow")
    );
    assert!(!is_active(store.connection()).unwrap());
    assert!(
        !w.git_common_dir
            .join("devmap/backend-transition/attempt.json")
            .exists()
    );
}

fn imported() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    SourceWorkspace,
    FrozenManifest,
    RepositoryStore,
) {
    let (repo, w) = source();
    let backups = tempfile::tempdir().unwrap();
    let snapshot = backups.path().join("snapshot");
    let manifest = freeze(&w, &snapshot, OffsetDateTime::now_utc()).unwrap();
    import_shadow(&w, &snapshot).unwrap();
    let store = RepositoryStore::open(&w).unwrap();
    validate_shadow(&store, &manifest).unwrap();
    (repo, backups, w, manifest, store)
}

#[test]
fn startup_recovery_rejects_extra_registry_row() {
    let (_repo, _backup, _w, manifest, store) = imported();
    store.connection().execute("INSERT INTO worktree_registry VALUES('extra','incarnation','extra-git','extra-root',NULL)", []).unwrap();
    assert_eq!(store.generation().unwrap(), 0);
    assert!(
        validate_shadow(&store, &manifest)
            .unwrap_err()
            .to_string()
            .contains("imported worktree registry inventory mismatch")
    );
    let retained: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM worktree_registry WHERE worktree_id='extra'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1);
}

#[test]
fn startup_recovery_rejects_extra_provenance_and_changed_snapshot_hash() {
    let (_repo, _backup, _w, manifest, store) = imported();
    store
        .connection()
        .execute(
            "INSERT INTO migration_sources VALUES('unowned','hash',0,'unknown','{}')",
            [],
        )
        .unwrap();
    assert!(validate_shadow(&store, &manifest).is_err());
    store
        .connection()
        .execute(
            "DELETE FROM migration_sources WHERE source_path='unowned'",
            [],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "UPDATE migration_sources SET source_hash='changed' WHERE source_path='@snapshot'",
            [],
        )
        .unwrap();
    assert!(
        validate_shadow(&store, &manifest)
            .unwrap_err()
            .to_string()
            .contains("provenance mismatch")
    );
}

#[test]
fn startup_fenced_pristine_shadow_requires_imported_provenance() {
    let (_repo, w) = source();
    let backup = tempfile::tempdir().unwrap();
    let manifest = freeze(
        &w,
        &backup.path().join("snapshot"),
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    let store = RepositoryStore::open(&w).unwrap();
    validate_shadow(&store, &manifest).unwrap();
    write_activation_fence(&w, &manifest).unwrap();
    assert!(
        validate_shadow(&store, &manifest)
            .unwrap_err()
            .to_string()
            .contains("requires imported shadow provenance")
    );
    assert!(!is_active(store.connection()).unwrap());
}

#[test]
fn startup_transition_blocks_missing_database_writer_and_legacy_openers() {
    let (_repo, w) = source();
    let guard = transition::Guard::acquire(&w).unwrap();
    let (sent, received) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for kind in 0..3 {
        let w = w.clone();
        let sent = sent.clone();
        workers.push(std::thread::spawn(move || {
            let result = match kind {
                0 => crate::store::domain_write(&w, |_| {
                    sent.send(kind).unwrap();
                    Ok(())
                }),
                1 => journal::JournalStore::open(&w, "blocked").map(|_| {
                    sent.send(kind).unwrap();
                }),
                _ => presence::PresenceStore::open(&w).map(|_| {
                    sent.send(kind).unwrap();
                }),
            };
            result.unwrap();
        }));
    }
    assert!(
        received
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_err()
    );
    assert!(!w.git_common_dir.join("devmap/sessions").exists());
    assert!(!w.git_common_dir.join("devmap/presence").exists());
    drop(guard);
    for worker in workers {
        worker.join().unwrap();
    }
    let mut kinds = vec![
        received.recv().unwrap(),
        received.recv().unwrap(),
        received.recv().unwrap(),
    ];
    kinds.sort();
    assert_eq!(kinds, [0, 1, 2]);
}

#[test]
fn startup_unknown_transition_child_is_not_ignored() {
    let (_repo, w) = source();
    let guard = transition::Guard::acquire(&w).unwrap();
    fs::write(guard.directory.join("unknown"), b"retain").unwrap();
    drop(guard);
    assert!(
        prepare_first_write(&w)
            .unwrap_err()
            .to_string()
            .contains("unknown backend transition")
    );
    assert_eq!(
        fs::read(w.git_common_dir.join("devmap/backend-transition/unknown")).unwrap(),
        b"retain"
    );
}

#[cfg(windows)]
#[test]
fn startup_default_state_selection_is_durable_and_respects_redirected_state() {
    let profile = tempfile::tempdir().unwrap();
    let conventional = profile.path().join("AppData/Local");
    let selected =
        windows_state_base(Some(conventional), Some(profile.path().to_path_buf())).unwrap();
    assert_eq!(selected, profile.path().join(".devmap-state"));
    assert!(!selected.exists(), "selection alone created state");
    let redirected = profile.path().join("explicit-state");
    assert_eq!(
        windows_state_base(Some(redirected.clone()), Some(profile.path().to_path_buf())).unwrap(),
        redirected
    );
}

#[cfg(windows)]
#[test]
fn startup_real_default_state_selection_is_read_only() {
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap();
    let selected =
        windows_state_base(Some(profile.join("AppData/Local")), Some(profile.clone())).unwrap();
    assert_eq!(selected, profile.join(".devmap-state"));
    platform::validate_chain(&profile).unwrap();
    eprintln!(
        "read-only verified default durable state selection: {}",
        selected.display()
    );
}

/// Run the real startup entry point with process-local durable-state settings.
/// No environment variable in the concurrently running parent test process is changed.
fn isolated_resume_case(name: &str, check: impl FnOnce()) {
    const MARKER: &str = "DEVMAP_STARTUP_RESUME_TEST";
    if std::env::var(MARKER).as_deref() == Ok(name) {
        check();
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let state = fixture.path().join("durable-state");
    fs::create_dir(&state).unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &format!("{module}::{name}"), "--nocapture"])
        .env(MARKER, name)
        .env("LOCALAPPDATA", &state)
        .env("XDG_STATE_HOME", &state)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(command.spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.take(1024 * 1024).read_to_string(&mut text).unwrap();
        text
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.take(1024 * 1024).read_to_string(&mut text).unwrap();
        text
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let output = output.join().unwrap();
    let errors = errors.join().unwrap();
    assert!(
        status.is_some_and(|s| s.success()),
        "isolated startup test failed/timed out: {output}\n{errors}"
    );
    assert!(
        output.contains("1 passed"),
        "exact child filter did not execute one test: {output}"
    );
}

enum OwnedSnapshotStage {
    PointerOnly,
    PartialShadow,
    Complete,
}

fn owned_attempt(w: &SourceWorkspace, stage: OwnedSnapshotStage) -> (Attempt, PathBuf) {
    let guard = transition::Guard::acquire(w).unwrap();
    let now = OffsetDateTime::now_utc();
    let manifest = inventory(w, origins(w).unwrap(), now.format(&Rfc3339).unwrap()).unwrap();
    require_empty(&manifest).unwrap();
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).unwrap();
    let attempt = Attempt {
        format: "devmap-empty-startup/1".into(),
        nonce: nonce.iter().map(|b| format!("{b:02x}")).collect(),
        state_root: backup_root(w, true).unwrap(),
        manifest,
    };
    let directory = attempt_directory(&attempt);
    platform::private_dir(&directory).unwrap();
    let bytes = serde_json::to_vec(&attempt).unwrap();
    write_private(&directory.join("owner.json"), &bytes).unwrap();
    write_private(&guard.directory.join("attempt.json"), &bytes).unwrap();
    let snapshot = validate_attempt(w, &attempt).unwrap();
    match stage {
        OwnedSnapshotStage::PointerOnly => {}
        OwnedSnapshotStage::PartialShadow => {
            drop(RepositoryStore::open_guarded(w, &guard).unwrap());
            platform::private_dir(&snapshot).unwrap();
            write_private(
                &snapshot.join("retained.partial"),
                b"incomplete frozen snapshot bytes",
            )
            .unwrap();
        }
        OwnedSnapshotStage::Complete => {
            assert_eq!(
                freeze_guarded(w, &snapshot, now, &guard).unwrap(),
                attempt.manifest
            );
        }
    }
    (attempt, snapshot)
}

fn retained_tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                output.insert(relative, None);
                visit(root, &path, output);
            } else {
                output.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

type StartupSqlRows = BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>;

fn logical_sql(w: &SourceWorkspace) -> Option<StartupSqlRows> {
    let store = RepositoryStore::open_existing(w).unwrap()?;
    store.connection().execute_batch("BEGIN").unwrap();
    let mut output = BTreeMap::new();
    for table in [
        "store_meta",
        "worktree_registry",
        "journal_sessions",
        "journal_records",
        "route_records",
        "presence_records",
        "binding_records",
        "binding_watermarks",
        "migration_sources",
        "presence_projection",
        "journal_heads",
    ] {
        let mut statement = store
            .connection()
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        output.insert(table.into(), rows);
    }
    Some(output)
}

fn assert_owned_resume(imported: bool, fenced: bool) {
    let (_repo, w) = source();
    let (attempt, snapshot) = owned_attempt(&w, OwnedSnapshotStage::Complete);
    let pointer = w
        .git_common_dir
        .join("devmap/backend-transition/attempt.json");
    let pointer_before = fs::read(&pointer).unwrap();
    let backup_before = retained_tree(&attempt.state_root);
    if imported {
        import_shadow(&w, &snapshot).unwrap();
    }
    if fenced {
        let _guard = transition::Guard::acquire(&w).unwrap();
        write_activation_fence(&w, &attempt.manifest).unwrap();
    }
    let before = RepositoryStore::open_existing(&w).unwrap().unwrap();
    assert!(!is_active(before.connection()).unwrap());
    assert_eq!(before.generation().unwrap(), 0);
    let provenance: i64 = before
        .connection()
        .query_row("SELECT count(*) FROM migration_sources", [], |r| r.get(0))
        .unwrap();
    assert_eq!(provenance, i64::from(imported));
    drop(before);

    assert_eq!(prepare_first_write(&w).unwrap(), WriteBackend::ActiveSql);
    let store = RepositoryStore::open_existing(&w).unwrap().unwrap();
    assert!(is_active(store.connection()).unwrap());
    assert_eq!(store.generation().unwrap(), 0);
    let activation = activation(store.connection()).unwrap().unwrap();
    assert_eq!(activation.manifest, attempt.manifest);
    assert_eq!(activation.snapshot_path, snapshot);
    assert_eq!(activation.generation, 0);
    let provenance: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM migration_sources", [], |r| r.get(0))
        .unwrap();
    assert_eq!(provenance, 2);
    for table in [
        "journal_sessions",
        "journal_records",
        "route_records",
        "presence_records",
        "binding_records",
        "binding_watermarks",
        "presence_projection",
        "journal_heads",
    ] {
        let count: i64 = store
            .connection()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "setup created a domain row in {table}");
    }
    drop(store);
    let sql_before_retry = logical_sql(&w);
    let fence_path = w.git_common_dir.join("devmap").join(FENCE);
    let fence = fs::read(&fence_path).unwrap();
    assert_eq!(prepare_first_write(&w).unwrap(), WriteBackend::ActiveSql);
    assert_eq!(logical_sql(&w), sql_before_retry);
    assert_eq!(fs::read(&fence_path).unwrap(), fence);
    assert_eq!(fs::read(&pointer).unwrap(), pointer_before);
    assert_eq!(retained_tree(&attempt.state_root), backup_before);
    assert!(!w.git_common_dir.join("devmap/sessions").exists());
}

#[test]
fn startup_owned_pristine_complete_snapshot_resumes_and_retries_unchanged() {
    isolated_resume_case(
        "startup_owned_pristine_complete_snapshot_resumes_and_retries_unchanged",
        || assert_owned_resume(false, false),
    );
}

#[test]
fn startup_owned_imported_snapshot_resumes_and_retries_unchanged() {
    isolated_resume_case(
        "startup_owned_imported_snapshot_resumes_and_retries_unchanged",
        || assert_owned_resume(true, false),
    );
}

#[test]
fn startup_owned_imported_fenced_snapshot_resumes_and_retries_unchanged() {
    isolated_resume_case(
        "startup_owned_imported_fenced_snapshot_resumes_and_retries_unchanged",
        || assert_owned_resume(true, true),
    );
}

#[test]
fn startup_owned_incomplete_attempts_refuse_recovery_and_retain_artifacts() {
    isolated_resume_case(
        "startup_owned_incomplete_attempts_refuse_recovery_and_retain_artifacts",
        || {
            for (stage, missing_database) in [
                (OwnedSnapshotStage::PointerOnly, true),
                (OwnedSnapshotStage::PartialShadow, false),
            ] {
                let (_repo, w) = source();
                let (attempt, snapshot) = owned_attempt(&w, stage);
                let pointer = w
                    .git_common_dir
                    .join("devmap/backend-transition/attempt.json");
                let pointer_before = fs::read(&pointer).unwrap();
                let backup_before = retained_tree(&attempt.state_root);
                let sql_before = logical_sql(&w);
                validate_attempt(&w, &attempt).unwrap();
                assert!(!snapshot.join("manifest.json").exists());
                assert_eq!(sql_before.is_none(), missing_database);
                for _ in 0..2 {
                    let error = prepare_first_write(&w).unwrap_err();
                    if missing_database {
                        assert!(
                            error.to_string().contains("attempt has no database"),
                            "{error}"
                        );
                    } else {
                        assert!(
                            matches!(error, DevMapError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
                            "{error}"
                        );
                    }
                    assert_eq!(logical_sql(&w), sql_before);
                    assert_eq!(fs::read(&pointer).unwrap(), pointer_before);
                    assert_eq!(retained_tree(&attempt.state_root), backup_before);
                    assert!(!w.git_common_dir.join("devmap").join(FENCE).exists());
                    assert!(!w.git_common_dir.join("devmap/sessions").exists());
                }
            }
        },
    );
}
