//! Cooperative candidate-writer gate. Old binaries do not participate.
use crate::{error::DevMapError, fs_security as safe, git::SourceWorkspace, runtime::platform};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) const DIRECTORY: &str = "backend-transition";
pub(crate) struct Guard {
    _file: File,
    pub(super) directory: PathBuf,
}
impl Guard {
    pub(crate) fn acquire(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        let common = safe::checked_canonical_directory(&workspace.git_common_dir)?;
        platform::validate_chain(&common)?;
        let parent = common.join("devmap");
        let directory = parent.join(DIRECTORY);
        // With no existing gate, reject unsupported stores before creating
        // bookkeeping. When a gate exists, another first writer may still be
        // initializing SQLite: wait for its lock before inspecting that schema.
        // Every caller revalidates the selector/store after acquiring the gate.
        if safe::checked_metadata(&directory.join("lock"))?.is_none() {
            drop(super::RepositoryStore::open_existing(workspace)?);
        }
        safe::ensure_directory(&parent)?;
        platform::validate_chain(&parent)?;
        platform::private_dir(&directory)?;
        let path = directory.join("lock");
        let file = platform::lock_file(&path)?;
        let checked = safe::checked_file(&path, false, false)?;
        if super::link_count(&file)? != 1
            || safe::file_identity(&file)? != safe::file_identity(&checked)?
        {
            return Err(super::err("unsafe backend transition lock"));
        }
        let started = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => break,
                Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                    if started.elapsed() >= Duration::from_secs(2) {
                        return Err(super::err("backend transition lock timeout"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e.into()),
            }
        }
        validate_directory(&directory)?;
        Ok(Self {
            _file: file,
            directory,
        })
    }
}

pub(super) fn validate_directory(path: &Path) -> Result<(), DevMapError> {
    platform::validate(path, true)?;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if !matches!(entry.file_name().to_str(), Some("lock" | "attempt.json")) {
            return Err(super::err("unknown backend transition artifact"));
        }
        platform::validate(&entry.path(), false)?;
        let file = safe::checked_file(&entry.path(), false, false)?;
        if super::link_count(&file)? != 1 {
            return Err(super::err("hardlinked backend transition artifact"));
        }
    }
    Ok(())
}
