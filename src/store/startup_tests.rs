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
            .contains("unowned records")
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
