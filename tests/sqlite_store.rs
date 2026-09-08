use devmap::{
    git::{SourceGitInspector, SourceWorkspace},
    store::RepositoryStore,
};
use std::{fs, path::Path, process::Command};
fn repo() -> (tempfile::TempDir, SourceWorkspace) {
    let d = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(d.path())
            .status()
            .unwrap()
            .success()
    );
    let w = SourceGitInspector::open(d.path())
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    (d, w)
}
#[test]
fn missing_read_only_does_not_create() {
    let (_d, w) = repo();
    assert!(RepositoryStore::open_existing(&w).unwrap().is_none());
    assert!(!w.git_common_dir.join("devmap").exists());
}
#[test]
fn persistent_shadow_and_durable_configuration() {
    let (_d, w) = repo();
    let s = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    assert_eq!(
        c.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        c.query_row("SELECT backend_state FROM store_meta", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "shadow"
    );
    c.execute("UPDATE store_meta SET generation=7", []).unwrap();
    drop(c);
    drop(s);
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        7
    );
}
#[test]
fn shared_common_directory() {
    let (d, w) = repo();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "initial"
            ])
            .status()
            .unwrap()
            .success()
    );
    let linked_path = d.path().join("linked");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(["worktree", "add", "-q", "-b", "linked"])
            .arg(&linked_path)
            .status()
            .unwrap()
            .success()
    );
    let linked = SourceGitInspector::open(&linked_path)
        .unwrap()
        .workspace()
        .unwrap();
    let a = RepositoryStore::open(&w).unwrap();
    let b = RepositoryStore::open(&linked).unwrap();
    assert_eq!(a.path(), b.path());
    assert!(!linked.git_dir.join("devmap").exists());
}
#[test]
fn rejects_identity_version_and_corruption_without_repair() {
    for sql in [
        "UPDATE store_meta SET repository_id='foreign'",
        "UPDATE store_meta SET schema_version=999",
    ] {
        let (_d, w) = repo();
        let s = RepositoryStore::open(&w).unwrap();
        let p = s.path().to_owned();
        drop(s);
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute(sql, []).unwrap();
        drop(c);
        let bytes = fs::read(&p).unwrap();
        assert!(RepositoryStore::open(&w).is_err());
        assert!(RepositoryStore::open_existing(&w).is_err());
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
    let (_d, w) = repo();
    fs::create_dir(w.git_common_dir.join("devmap")).unwrap();
    let p = w.git_common_dir.join("devmap/devmap.db");
    fs::write(&p, b"not sqlite").unwrap();
    assert!(RepositoryStore::open(&w).is_err());
    assert_eq!(fs::read(p).unwrap(), b"not sqlite");
}
#[test]
fn backup_is_consistent_and_never_overwrites() {
    let (d, w) = repo();
    let s = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    c.execute("UPDATE store_meta SET generation=19", [])
        .unwrap();
    let dest = d.path().join("snapshot.db");
    s.backup_to(&dest).unwrap();
    let restored = rusqlite::Connection::open(&dest).unwrap();
    assert_eq!(
        restored
            .query_row("SELECT generation FROM store_meta", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        19
    );
    assert!(s.backup_to(&dest).is_err());
    assert!(
        s.backup_to(Path::new("missing-parent/snapshot.db"))
            .is_err()
    );
    s.integrity_check().unwrap();
}

#[test]
fn hardlinked_database_and_sidecars_are_rejected() {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let (d, w) = repo();
        let s = RepositoryStore::open(&w).unwrap();
        let p = s.path().to_owned();
        drop(s);
        let candidate = std::path::PathBuf::from(format!("{}{suffix}", p.display()));
        if suffix != "" {
            fs::write(&candidate, b"sentinel").unwrap();
        }
        fs::hard_link(&candidate, d.path().join("alias")).unwrap();
        assert!(RepositoryStore::open(&w).is_err());
        assert!(RepositoryStore::open_existing(&w).is_err());
    }
}
#[test]
fn read_only_open_keeps_main_database_unchanged_and_observes_later_commit() {
    let (_d, w) = repo();
    let s = RepositoryStore::open(&w).unwrap();
    let p = s.path().to_owned();
    drop(s);
    let before = fs::read(&p).unwrap();
    let reader = RepositoryStore::open_existing(&w).unwrap().unwrap();
    reader.integrity_check().unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
    assert_eq!(reader.generation().unwrap(), 0);
    let writer = rusqlite::Connection::open(&p).unwrap();
    writer
        .execute("UPDATE store_meta SET generation=2", [])
        .unwrap();
    assert_eq!(reader.generation().unwrap(), 2);
}
#[test]
fn orphan_sidecar_is_not_adopted_by_new_store_or_backup() {
    let (d, w) = repo();
    fs::create_dir(w.git_common_dir.join("devmap")).unwrap();
    let wal = w.git_common_dir.join("devmap/devmap.db-wal");
    fs::write(&wal, b"orphan").unwrap();
    assert!(RepositoryStore::open(&w).is_err());
    assert!(!w.git_common_dir.join("devmap/devmap.db").exists());
    assert_eq!(fs::read(&wal).unwrap(), b"orphan");
    fs::remove_file(wal).unwrap();
    let s = RepositoryStore::open(&w).unwrap();
    let dest = d.path().join("backup.db");
    fs::write(d.path().join("backup.db-wal"), b"orphan").unwrap();
    assert!(s.backup_to(&dest).is_err());
    assert!(!dest.exists());
}

#[test]
fn simultaneous_first_open_is_serialized() {
    let (_d, w) = repo();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let w = w.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                RepositoryStore::open(&w).map(|s| s.generation().unwrap())
            })
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().unwrap().unwrap(), 0);
    }
}
#[test]
fn interrupted_empty_database_is_preserved_for_explicit_recovery() {
    let (_d, w) = repo();
    let p = w.git_common_dir.join("devmap/devmap.db");
    fs::create_dir(p.parent().unwrap()).unwrap();
    fs::write(&p, []).unwrap();
    assert!(RepositoryStore::open(&w).is_err());
    assert!(RepositoryStore::open_existing(&w).is_err());
    assert_eq!(fs::metadata(p).unwrap().len(), 0);
}
#[cfg(windows)]
#[test]
fn junction_directory_is_rejected() {
    let (d, w) = repo();
    let outside = d.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let junction = w.git_common_dir.join("devmap");
    assert!(
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(RepositoryStore::open(&w).is_err());
    assert!(RepositoryStore::open_existing(&w).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
#[cfg(unix)]
#[test]
fn symlink_directory_is_rejected() {
    let (d, w) = repo();
    let outside = d.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, w.git_common_dir.join("devmap")).unwrap();
    assert!(RepositoryStore::open(&w).is_err());
    assert!(RepositoryStore::open_existing(&w).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
