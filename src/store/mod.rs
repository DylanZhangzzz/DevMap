//! Repository-local transactional storage. Creating a store does not activate it.
pub mod migration;
pub(crate) mod origin_links;
pub(crate) mod snapshot;
pub(crate) mod transition;
use crate::{error::DevMapError, fs_security, git::SourceWorkspace, worktrees};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};
const VERSION: i64 = 2;
const TIMEOUT: Duration = Duration::from_secs(2);
pub struct RepositoryStore {
    connection: Connection,
    path: PathBuf,
}
impl RepositoryStore {
    /// Opens or creates a shadow store. Interrupted initialization is never repaired implicitly.
    pub fn open(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        let guard = transition::Guard::acquire(workspace)?;
        Self::open_guarded(workspace, &guard)
    }
    fn open_guarded(
        workspace: &SourceWorkspace,
        _guard: &transition::Guard,
    ) -> Result<Self, DevMapError> {
        let (common, path) = location(workspace)?;
        fs_security::ensure_directory(path.parent().unwrap())?;
        let existing = fs_security::checked_metadata(&path)?.is_some();
        // The transition guard excludes another candidate's initialization.
        // Reject an incompatible existing store before creating bookkeeping,
        // including when its transition gate predates this invocation.
        if existing {
            let probe =
                Self::open_existing(workspace)?.ok_or_else(|| err("database disappeared"))?;
            probe.integrity_check()?;
        }
        let _initialization = initialization_lock(path.parent().unwrap())?;
        if !existing {
            migration::refuse_missing_database(workspace)?;
            reject_sidecars(&path)?;
            fs_security::ensure_directory(path.parent().unwrap())?;
            check_files(&path)?;
            let f = fs_security::checked_new_file(&path)?;
            f.sync_all()?;
        }
        check_files(&path)?;
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(TIMEOUT)?;
        if existing {
            validate(&connection, workspace, &common)?;
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        if !existing {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute("INSERT INTO store_meta(singleton,schema_version,repository_id,common_dir) VALUES(1,?1,?2,?3)",rusqlite::params![VERSION,worktrees::repository_id(workspace),common.to_string_lossy()])?;
            tx.commit()?;
        }
        Ok(Self { connection, path })
    }
    /// Reads an existing store; SQLite locking sidecars are permitted, domain writes are not.
    pub fn open_existing(workspace: &SourceWorkspace) -> Result<Option<Self>, DevMapError> {
        let (common, path) = location(workspace)?;
        if fs_security::checked_metadata(path.parent().unwrap())?.is_none() {
            return Ok(None);
        }
        fs_security::checked_canonical_directory(path.parent().unwrap())?;
        check_files(&path)?;
        if fs_security::checked_metadata(&path)?.is_none() {
            migration::refuse_missing_database(workspace)?;
            return Ok(None);
        }
        // SQLite may maintain locking/WAL-index sidecars, but this handle cannot
        // create a main database, change its schema, or repair domain records.
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(TIMEOUT)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        validate(&connection, workspace, &common)?;
        Ok(Some(Self { connection, path }))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn generation(&self) -> Result<u64, DevMapError> {
        {
            let n: i64 = self.connection.query_row(
                "SELECT generation FROM store_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )?;
            u64::try_from(n).map_err(|_| err("negative generation"))
        }
    }
    pub fn integrity_check(&self) -> Result<(), DevMapError> {
        let mut stmt = self.connection.prepare("PRAGMA integrity_check")?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if row? != "ok" {
                return Err(err("integrity check failed"));
            }
        }
        let mut stmt = self.connection.prepare("PRAGMA foreign_key_check")?;
        if stmt.query([])?.next()?.is_some() {
            return Err(err("foreign key check failed"));
        }
        Ok(())
    }
    /// Produces a consistent new SQLite snapshot; failures retain the incomplete destination.
    pub fn backup_to(&self, destination: &Path) -> Result<(), DevMapError> {
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| err("backup requires an existing parent"))?;
        fs_security::checked_canonical_directory(parent)?;
        check_files(destination)?;
        reject_sidecars(destination)?;
        let file = fs_security::checked_new_file(destination)?;
        let mut target =
            Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        target.busy_timeout(TIMEOUT)?;
        target.pragma_update(None, "synchronous", "FULL")?;
        {
            let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)?;
            // One finite step: do not retry forever while another writer is busy.
            if !matches!(backup.step(-1)?, rusqlite::backup::StepResult::Done) {
                return Err(err("backup busy; incomplete destination retained"));
            }
        }
        let result: String = target.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if result != "ok" {
            return Err(err("backup integrity check failed"));
        }
        if target
            .prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_some()
        {
            return Err(err("backup foreign key check failed"));
        }
        drop(target);
        file.sync_all()?;
        fs_security::sync_directory(parent)?;
        Ok(())
    }
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
    /// Commits accepted writes atomically. Callers explicitly increment generation only on change.
    pub(crate) fn transaction<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, DevMapError>,
    ) -> Result<T, DevMapError> {
        fs_security::checked_canonical_directory(self.path.parent().unwrap())?;
        check_files(&self.path)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}
fn err(s: &str) -> DevMapError {
    DevMapError::Store(s.into())
}
fn location(w: &SourceWorkspace) -> Result<(PathBuf, PathBuf), DevMapError> {
    let c = fs_security::checked_canonical_directory(&w.git_common_dir)?;
    let p = c.join("devmap/devmap.db");
    if fs_security::checked_metadata(p.parent().unwrap())?.is_some() {
        fs_security::checked_canonical_directory(p.parent().unwrap())?;
    }
    Ok((c, p))
}
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(suffix);
    p.into()
}
fn check_files(path: &Path) -> Result<(), DevMapError> {
    for p in [
        path.to_owned(),
        sidecar(path, "-wal"),
        sidecar(path, "-shm"),
        sidecar(path, "-journal"),
    ] {
        if fs_security::checked_metadata(&p)?.is_some() {
            let f = match fs_security::checked_file(&p, false, false) {
                Ok(file) => file,
                Err(_) if p != path && fs_security::checked_metadata(&p)?.is_none() => continue,
                Err(error) => return Err(error),
            };
            if link_count(&f)? != 1 {
                return Err(err("hard-linked database or sidecar refused"));
            }
        }
    }
    Ok(())
}
fn validate(c: &Connection, w: &SourceWorkspace, common: &Path) -> Result<(), DevMapError> {
    let (version, id, dir): (i64, String, String) = c.query_row(
        "SELECT schema_version,repository_id,common_dir FROM store_meta WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if version != VERSION {
        return Err(err("unsupported schema version"));
    }
    if id != worktrees::repository_id(w) || dir != common.to_string_lossy() {
        return Err(err("repository identity mismatch"));
    }
    origin_links::validate_schema(c)
}
#[cfg(unix)]
fn link_count(f: &File) -> Result<u64, DevMapError> {
    use std::os::unix::fs::MetadataExt;
    Ok(f.metadata()?.nlink())
}
#[cfg(windows)]
fn link_count(f: &File) -> Result<u64, DevMapError> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info {
        allocation: i64,
        size: i64,
        links: u32,
        delete_pending: u8,
        directory: u8,
    }
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut core::ffi::c_void,
            class: i32,
            info: *mut core::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let mut info = std::mem::MaybeUninit::<Info>::uninit();
    // SAFETY: File supplies a live handle; Info matches FILE_STANDARD_INFO.
    if unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle(),
            1,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<Info>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful Windows call initialized the structure.
    Ok(u64::from(unsafe { info.assume_init() }.links))
}
#[cfg(not(any(windows, unix)))]
fn link_count(_: &File) -> Result<u64, DevMapError> {
    Err(err("unsupported filesystem identity"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, RepositoryStore) {
        let d = tempfile::tempdir().unwrap();
        let common = d.path().join("git");
        std::fs::create_dir(&common).unwrap();
        let w = SourceWorkspace {
            root: d.path().into(),
            git_dir: common.clone(),
            git_common_dir: common,
            branch: None,
            head: String::new(),
        };
        let s = RepositoryStore::open(&w).unwrap();
        (d, s)
    }
    #[test]
    fn rollback_visibility_constraints_and_durability() {
        let (_d, mut s) = store();
        assert_eq!(
            s.connection()
                .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            s.connection()
                .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let reader = Connection::open(s.path()).unwrap();
        let failed: Result<(), DevMapError> = s.transaction(|tx| {
            tx.execute("UPDATE store_meta SET generation=5", [])?;
            assert_eq!(
                reader.query_row("SELECT generation FROM store_meta", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Err(err("abort"))
        });
        assert!(failed.is_err());
        assert_eq!(s.generation().unwrap(), 0);
        s.transaction(|tx| {
            tx.execute("UPDATE store_meta SET generation=1", [])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            reader
                .query_row("SELECT generation FROM store_meta", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            s.transaction(|tx| {
                tx.execute(
                    "INSERT INTO journal_records VALUES('missing',1,'e','{}',2)",
                    [],
                )?;
                Ok(())
            })
            .is_err()
        );
        s.transaction(|_| Ok(())).unwrap();
        assert_eq!(s.generation().unwrap(), 1);
        s.integrity_check().unwrap();
    }
    #[test]
    fn contention_has_a_finite_timeout() {
        let (_d, mut s) = store();
        let blocker = Connection::open(s.path()).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let start = std::time::Instant::now();
        assert!(s.transaction(|_| Ok(())).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        blocker.execute_batch("ROLLBACK").unwrap();
        s.transaction(|_| Ok(())).unwrap();
    }
}

fn reject_sidecars(path: &Path) -> Result<(), DevMapError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        if fs_security::checked_metadata(&sidecar(path, suffix))?.is_some() {
            return Err(err("new database destination has existing sidecars"));
        }
    }
    Ok(())
}

// Serializes first creation and open validation across processes. The lock file
// is bookkeeping only; a crash leaves any partial database intact for recovery.
fn initialization_lock(directory: &Path) -> Result<File, DevMapError> {
    let path = directory.join("store-init.lock");
    let file = fs_security::checked_file(&path, true, true)?;
    if link_count(&file)? != 1 {
        return Err(err("hard-linked initialization lock refused"));
    }
    let started = std::time::Instant::now();
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if started.elapsed() >= TIMEOUT {
                    return Err(err("initialization lock timeout"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}
#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn long_reader_snapshot_is_pinned_until_commit() {
        let d = tempfile::tempdir().unwrap();
        let common = d.path().join("git");
        std::fs::create_dir(&common).unwrap();
        let w = SourceWorkspace {
            root: d.path().into(),
            git_dir: common.clone(),
            git_common_dir: common,
            branch: None,
            head: String::new(),
        };
        let mut writer = RepositoryStore::open(&w).unwrap();
        let reader = RepositoryStore::open_existing(&w).unwrap().unwrap();
        reader.connection().execute_batch("BEGIN").unwrap();
        assert_eq!(reader.generation().unwrap(), 0);
        writer
            .transaction(|tx| {
                tx.execute("UPDATE store_meta SET generation=1", [])?;
                Ok(())
            })
            .unwrap();
        assert_eq!(reader.generation().unwrap(), 0);
        reader.connection().execute_batch("COMMIT").unwrap();
        assert_eq!(reader.generation().unwrap(), 1);
    }
}

/// Validate the persisted selector even when legacy remains authoritative.
pub(crate) fn is_active(connection: &Connection) -> Result<bool, DevMapError> {
    let state: String = connection.query_row(
        "SELECT backend_state FROM store_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    match state.as_str() {
        "active" => Ok(true),
        "shadow" => {
            let evidence:i64=connection.query_row("SELECT generation+(SELECT count(*) FROM migration_sources WHERE source_path='@activation') FROM store_meta WHERE singleton=1",[],|r|r.get(0))?;
            if evidence != 0 {
                return Err(err(
                    "shadow selector conflicts with accepted SQL history; downgrade refused",
                ));
            }
            Ok(false)
        }
        _ => Err(err("unknown backend state")),
    }
}
pub(crate) fn active_existing(
    workspace: &SourceWorkspace,
) -> Result<Option<RepositoryStore>, DevMapError> {
    let Some(store) = RepositoryStore::open_existing(workspace)? else {
        return Ok(None);
    };
    if is_active(store.connection())? {
        migration::check_legacy_drift(workspace, store.connection())?;
        Ok(Some(store))
    } else {
        Ok(None)
    }
}

/// A journal-only capability bound to one current physical incarnation. It does
/// not authorize route, binding, standalone presence, or migration operations.
pub(crate) struct JournalAdmission {
    origin: migration::VerifiedCurrentOrigin,
    session_id: String,
    fingerprint: String,
}
impl JournalAdmission {
    pub(crate) fn sql_origin(&self) -> (String, String, String) {
        (
            self.origin.worktree_id.clone(),
            self.origin.incarnation.clone(),
            self.origin.git_dir.to_string_lossy().into_owned(),
        )
    }
    fn check(
        workspace: &SourceWorkspace,
        c: &Connection,
        session_id: &str,
        opened_incarnation: &str,
    ) -> Result<Self, DevMapError> {
        use rusqlite::OptionalExtension;
        let report = migration::observe_active_journal_origins(workspace, c)?;
        let root = fs_security::checked_canonical_directory(&workspace.root)?;
        let git = fs_security::checked_canonical_directory(&workspace.git_dir)?;
        let observed = report
            .current()
            .values()
            .find(|origin| origin.git_dir == git)
            .ok_or_else(|| err("journal target is not a verified current origin"))?;
        let origin = report
            .current_origin(&observed.worktree_id)
            .ok_or_else(|| err("verified journal origin missing"))?;
        if fs_security::checked_canonical_directory(&origin.workspace_path)? != root {
            return Err(err("journal target workspace identity mismatch"));
        }
        if origin.incarnation != opened_incarnation {
            return Err(err("opened journal worktree incarnation changed"));
        }
        let saved: Option<(String, String, String)> = c.query_row(
            "SELECT worktree_id,incarnation,origin_path FROM journal_sessions WHERE session_id=?1",
            [session_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        let registered = saved.is_some();
        if let Some((worktree, incarnation, path)) = saved {
            if worktree != origin.worktree_id
                || incarnation != origin.incarnation
                || path != origin.git_dir.to_string_lossy()
            {
                return Err(err("session origin or worktree incarnation mismatch"));
            }
        } else if !report.unavailable.is_empty() {
            let exists: i64 = c.query_row(
                "SELECT count(*) FROM presence_records WHERE session_id=?1",
                [session_id],
                |r| r.get(0),
            )?;
            if exists != 0 {
                return Err(err(
                    "unregistered historical presence cannot acquire a new journal incarnation",
                ));
            }
        }
        let registry: Option<i64> = c.query_row(
            "SELECT 1 FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",
            rusqlite::params![origin.worktree_id, origin.incarnation],
            |r| r.get(0),
        ).optional()?;
        if registry.is_some() {
            let (saved, retired) =
                origin_links::registered_origin(c, &origin.worktree_id, &origin.incarnation)?;
            if !origin.matches(&saved) || retired.is_some() {
                return Err(err(
                    "journal registry identity mismatch or retired incarnation",
                ));
            }
        } else if registered {
            return Err(err("journal session has no matching registry"));
        }
        Ok(Self {
            origin,
            session_id: session_id.to_owned(),
            fingerprint: report.fingerprint,
        })
    }

    pub(crate) fn validate_registry(&self, c: &Connection) -> Result<(), DevMapError> {
        let exists: i64 = c.query_row(
            "SELECT count(*) FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",
            rusqlite::params![self.origin.worktree_id, self.origin.incarnation],
            |r| r.get(0),
        )?;
        if exists != 0 {
            let (saved, retired) = origin_links::registered_origin(
                c,
                &self.origin.worktree_id,
                &self.origin.incarnation,
            )?;
            if !self.origin.matches(&saved) || retired.is_some() {
                return Err(err(
                    "journal registry identity mismatch or retired incarnation",
                ));
            }
        }
        Ok(())
    }

    fn recheck(&self, workspace: &SourceWorkspace, c: &Connection) -> Result<(), DevMapError> {
        let after = Self::check(workspace, c, &self.session_id, &self.origin.incarnation)?;
        if after.origin != self.origin || after.fingerprint != self.fingerprint {
            return Err(err("journal origins changed during acceptance"));
        }
        Ok(())
    }
}

/// SQL journal opening/replay is read-only and never repairs/registers a session.
pub(crate) fn journal_read<T>(
    workspace: &SourceWorkspace,
    session_id: &str,
    opened_incarnation: &str,
    f: impl FnOnce(&Connection, &JournalAdmission) -> Result<T, DevMapError>,
) -> Result<Option<T>, DevMapError> {
    let Some(store) = RepositoryStore::open_existing(workspace)? else {
        return Ok(None);
    };
    let tx = store.connection().unchecked_transaction()?;
    if !is_active(&tx)? {
        return Ok(None);
    }
    let admission = JournalAdmission::check(workspace, &tx, session_id, opened_incarnation)?;
    let result = f(&tx, &admission)?;
    admission.recheck(workspace, &tx)?;
    tx.commit()?;
    Ok(Some(result))
}

pub(crate) fn journal_write_guarded<T>(
    workspace: &SourceWorkspace,
    session_id: &str,
    opened_incarnation: &str,
    f: impl FnOnce(
        Option<(&Transaction<'_>, &JournalAdmission)>,
        &transition::Guard,
    ) -> Result<T, DevMapError>,
) -> Result<T, DevMapError> {
    let guard = transition::Guard::acquire(workspace)?;
    if RepositoryStore::open_existing(workspace)?.is_none() {
        return f(None, &guard);
    }
    let mut store = RepositoryStore::open_guarded(workspace, &guard)?;
    store.transaction(|tx| {
        if !is_active(tx)? {
            return f(None, &guard);
        }
        let admission = JournalAdmission::check(workspace, tx, session_id, opened_incarnation)?;
        let result = f(Some((tx, &admission)), &guard)?;
        admission.recheck(workspace, tx)?;
        Ok(result)
    })
}
/// Closed route/binding admission. Raw accepted history is validated by the
/// domain writer; current targets are resolved only when that operation needs
/// a new association. This does not authorize journal or standalone presence.
pub(crate) struct OriginAdmission {
    report: migration::ActiveOriginReport,
    actor: migration::FrozenOrigin,
}
impl OriginAdmission {
    fn check(workspace: &SourceWorkspace, c: &Connection) -> Result<Self, DevMapError> {
        let report = migration::observe_active_read_origins(workspace, c)?;
        let git = fs_security::checked_canonical_directory(&workspace.git_dir)?;
        let root = fs_security::checked_canonical_directory(&workspace.root)?;
        let actor = report
            .current()
            .values()
            .find(|origin| origin.git_dir == git)
            .ok_or_else(|| err("origin writer is not a current repository workspace"))?
            .clone();
        if fs_security::checked_canonical_directory(&actor.workspace_path)? != root
            || crate::journal::worktree_incarnation(workspace)? != actor.incarnation
        {
            return Err(err("origin writer workspace identity mismatch"));
        }
        Ok(Self { report, actor })
    }
    pub(crate) fn current_origin(
        &self,
        worktree_id: &str,
    ) -> Result<migration::VerifiedCurrentOrigin, DevMapError> {
        self.report
            .current_origin(worktree_id)
            .ok_or_else(|| err("target worktree is not currently verified"))
    }
    fn recheck(&self, workspace: &SourceWorkspace, c: &Connection) -> Result<(), DevMapError> {
        let after = Self::check(workspace, c)?;
        if after.actor != self.actor || after.report.fingerprint != self.report.fingerprint {
            return Err(err("repository origins changed during acceptance"));
        }
        Ok(())
    }
}

pub(crate) fn origin_write<T>(
    workspace: &SourceWorkspace,
    f: impl FnOnce(Option<(&Transaction<'_>, &OriginAdmission)>) -> Result<T, DevMapError>,
) -> Result<T, DevMapError> {
    let guard = transition::Guard::acquire(workspace)?;
    if RepositoryStore::open_existing(workspace)?.is_none() {
        return f(None);
    }
    let mut store = RepositoryStore::open_guarded(workspace, &guard)?;
    store.transaction(|tx| {
        if !is_active(tx)? {
            return f(None);
        }
        let admission = OriginAdmission::check(workspace, tx)?;
        let result = f(Some((tx, &admission)))?;
        admission.recheck(workspace, tx)?;
        Ok(result)
    })
}

/// Serialize selector inspection and writes with activation of an existing shadow.
pub(crate) fn domain_write<T>(
    workspace: &SourceWorkspace,
    f: impl FnOnce(Option<&Transaction<'_>>) -> Result<T, DevMapError>,
) -> Result<T, DevMapError> {
    domain_write_guarded(workspace, |tx, _guard| f(tx))
}

pub(crate) fn domain_write_guarded<T>(
    workspace: &SourceWorkspace,
    f: impl FnOnce(Option<&Transaction<'_>>, &transition::Guard) -> Result<T, DevMapError>,
) -> Result<T, DevMapError> {
    let transition = transition::Guard::acquire(workspace)?;
    if RepositoryStore::open_existing(workspace)?.is_none() {
        return f(None, &transition);
    }
    let mut store = RepositoryStore::open_guarded(workspace, &transition)?;
    store.transaction(|tx| {
        if is_active(tx)? {
            migration::check_legacy_drift(workspace, tx)?;
            f(Some(tx), &transition)
        } else {
            f(None, &transition)
        }
    })
}

/// Lock order for legacy writes and activation: SQLite IMMEDIATE, then domain files.
pub(crate) fn lock_domain_file(file: &File) -> Result<(), DevMapError> {
    let start = std::time::Instant::now();
    loop {
        match fs2::FileExt::try_lock_exclusive(file) {
            Ok(()) => return Ok(()),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if start.elapsed() >= TIMEOUT {
                    return Err(err("domain lock timeout"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e.into()),
        }
    }
}
