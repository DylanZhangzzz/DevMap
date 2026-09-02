use std::fs::{self, File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::DevMapError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub kind: &'static str,
    pub first: u64,
    pub second: u64,
}

pub(crate) fn metadata_identity(metadata: &Metadata) -> Result<FileIdentity, DevMapError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FileIdentity {
            kind: "unix",
            first: metadata.dev(),
            second: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(FileIdentity {
            kind: "windows-metadata",
            first: metadata.creation_time(),
            second: metadata.file_size() ^ metadata.last_write_time(),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        Err(DevMapError::JournalCorruption(
            "file identity is unsupported on this platform".into(),
        ))
    }
}

pub(crate) fn is_link_or_reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub(crate) fn checked_metadata(path: &Path) -> Result<Option<Metadata>, DevMapError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => {
            Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn checked_canonical_directory(path: &Path) -> Result<PathBuf, DevMapError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component.as_os_str());
        if matches!(
            component,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = checked_metadata(&current)?
            .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(current.clone()))?;
        if !metadata.is_dir() {
            return Err(DevMapError::UnsafeInstallerOverwrite(current));
        }
    }
    Ok(fs::canonicalize(absolute)?)
}

pub(crate) fn ensure_directory(path: &Path) -> Result<(), DevMapError> {
    if let Some(metadata) = checked_metadata(path)? {
        if !metadata.is_dir() {
            return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    let parent_metadata = checked_metadata(parent)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(parent.to_path_buf()))?;
    if !parent_metadata.is_dir() {
        return Err(DevMapError::UnsafeInstallerOverwrite(parent.to_path_buf()));
    }
    match fs::create_dir(path) {
        Ok(()) => sync_directory(parent)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = checked_metadata(path)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    if !metadata.is_dir() {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn ensure_directory_chain(base: &Path, names: &[&str]) -> Result<PathBuf, DevMapError> {
    let base_metadata = checked_metadata(base)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(base.to_path_buf()))?;
    if !base_metadata.is_dir() {
        return Err(DevMapError::UnsafeInstallerOverwrite(base.to_path_buf()));
    }
    let mut current = base.to_path_buf();
    for name in names {
        current.push(name);
        ensure_directory(&current)?;
    }
    Ok(current)
}

pub(crate) fn checked_file(path: &Path, write: bool, create: bool) -> Result<File, DevMapError> {
    let before = checked_metadata(path)?;
    if before.as_ref().is_some_and(|metadata| !metadata.is_file()) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(write).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let handle_metadata = file.metadata()?;
    if !handle_metadata.is_file() || is_link_or_reparse(&handle_metadata) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let _path_metadata = checked_metadata(path)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    if file_identity(&file)? != path_file_identity(path)? {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    if let Some(before) = before
        && !cfg!(windows)
        && metadata_identity(&before)? != metadata_identity(&handle_metadata)?
    {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(file)
}

pub(crate) fn checked_new_file(path: &Path) -> Result<File, DevMapError> {
    if checked_metadata(path)?.is_some() {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || is_link_or_reparse(&metadata)
        || file_identity(&file)? != path_file_identity(path)?
    {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(file)
}

pub(crate) fn checked_directory_identity(path: &Path) -> Result<FileIdentity, DevMapError> {
    let metadata = checked_metadata(path)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    if !metadata.is_dir() {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let first = open_directory_nofollow(path)?;
    let second = open_directory_nofollow(path)?;
    let first_identity = file_identity(&first)?;
    if first_identity != file_identity(&second)? {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(first_identity)
}

fn open_directory_nofollow(path: &Path) -> Result<File, DevMapError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    Ok(file)
}

impl FileIdentity {
    pub(crate) fn stable_text(&self) -> String {
        format!("{}:{}:{}", self.kind, self.first, self.second)
    }
}

pub(crate) fn file_identity(file: &File) -> Result<FileIdentity, DevMapError> {
    #[cfg(unix)]
    {
        metadata_identity(&file.metadata()?)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;

        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct ByHandleFileInformation {
            attributes: u32,
            creation_time: FileTime,
            last_access_time: FileTime,
            last_write_time: FileTime,
            volume_serial_number: u32,
            file_size_high: u32,
            file_size_low: u32,
            number_of_links: u32,
            file_index_high: u32,
            file_index_low: u32,
        }
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut core::ffi::c_void,
                information: *mut ByHandleFileInformation,
            ) -> i32;
        }
        let mut information = std::mem::MaybeUninit::<ByHandleFileInformation>::uninit();
        // SAFETY: the handle belongs to a live File and the output points to writable storage.
        let success =
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
        if success == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: a successful call initialized the complete structure.
        let information = unsafe { information.assume_init() };
        Ok(FileIdentity {
            kind: "windows",
            first: u64::from(information.volume_serial_number),
            second: (u64::from(information.file_index_high) << 32)
                | u64::from(information.file_index_low),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(DevMapError::JournalCorruption(
            "file identity is unsupported on this platform".into(),
        ))
    }
}

fn path_file_identity(path: &Path) -> Result<FileIdentity, DevMapError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let reopened = options.open(path)?;
    file_identity(&reopened)
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), DevMapError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
