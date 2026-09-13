//! Read-only kernel path validation for ordinary Windows disk paths.
//! No path/byte cache; unsupported syntax is handled by the original caller.
use super::*;
use std::ffi::{OsStr, c_void};
use std::os::windows::{
    ffi::OsStrExt,
    io::{AsRawHandle, FromRawHandle},
};
use windows_sys::Win32::Foundation::UNICODE_STRING;

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: *mut c_void,
    name: *mut UNICODE_STRING,
    attributes: u32,
    security: *mut c_void,
    quality: *mut c_void,
}
#[repr(C)]
struct IoStatusBlock {
    // The ABI's first member is a union of NTSTATUS and a pointer.
    status_or_pointer: usize,
    information: usize,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtOpenFile(
        handle: *mut *mut c_void,
        access: u32,
        attributes: *const ObjectAttributes,
        io: *mut IoStatusBlock,
        sharing: u32,
        options: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

fn invalid() -> DevMapError {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "no-reparse read requires a normal absolute disk path",
    )
    .into()
}

fn split(path: &Path) -> Result<(PathBuf, Vec<u16>), DevMapError> {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(invalid());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err(invalid());
    }
    let mut root = PathBuf::from(prefix.as_os_str());
    root.push(OsStr::new("\\"));
    let mut tail = Vec::new();
    for component in components {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        // Native paths do not apply all Win32 filename normalization. Decline
        // ambiguous names instead of changing which object the caller opens.
        let Some(text) = name.to_str() else {
            return Err(invalid());
        };
        if text.ends_with(['.', ' ']) || text.chars().any(|ch| ch < ' ' || ":*?\"<>|/".contains(ch))
        {
            return Err(invalid());
        }
        let stem = text.split('.').next().unwrap_or("");
        if ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"]
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.get(..3)
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
                    && stem.get(3..).is_some_and(|tail| {
                        matches!(
                            tail,
                            "0" | "1"
                                | "2"
                                | "3"
                                | "4"
                                | "5"
                                | "6"
                                | "7"
                                | "8"
                                | "9"
                                | "¹"
                                | "²"
                                | "³"
                        )
                    })
            })
        {
            return Err(invalid());
        }
        if !tail.is_empty() {
            tail.push(u16::from(b'\\'));
        }
        for unit in name.encode_wide() {
            if tail.len() >= usize::from(u16::MAX) / 2 {
                return Err(invalid());
            }
            tail.push(unit);
        }
    }
    if tail.is_empty()
        || tail.contains(&0)
        || tail.contains(&u16::from(b':'))
        || tail.len() > usize::from(u16::MAX) / 2
    {
        return Err(invalid());
    }
    Ok((root, tail))
}

fn relative_file(root: &File, name: &mut [u16]) -> Result<File, DevMapError> {
    let bytes = u16::try_from(name.len() * 2).map_err(|_| invalid())?;
    let mut name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = ObjectAttributes {
        length: u32::try_from(std::mem::size_of::<ObjectAttributes>()).unwrap(),
        root: root.as_raw_handle(),
        name: &mut name,
        attributes: 0x40 | 0x1000, // OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE
        security: std::ptr::null_mut(),
        quality: std::ptr::null_mut(),
    };
    let mut io = IoStatusBlock {
        status_or_pointer: 0,
        information: 0,
    };
    let mut handle = std::ptr::null_mut();
    // FILE_GENERIC_READ; sharing read/write/delete; synchronous, non-directory,
    // open-reparse-point. OBJ_DONT_REPARSE also covers intermediate components.
    // SAFETY: all input/output buffers live through this synchronous call;
    // RootDirectory is a live owned directory handle. No kernel handle flag.
    let status = unsafe {
        NtOpenFile(
            &mut handle,
            0x0012_0089,
            &attributes,
            &mut io,
            7,
            0x20 | 0x40 | 0x0020_0000,
        )
    };
    if status < 0 {
        // SAFETY: status conversion has no pointer arguments or ownership effect.
        return Err(std::io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32
        )
        .into());
    }
    if handle.is_null() {
        return Err(std::io::Error::other("no-reparse open returned a null handle").into());
    }
    // SAFETY: a successful synchronous open transferred this unique handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// None means unsupported syntax; a native I/O failure is not a syntax decline.
pub(crate) fn try_checked_file(path: &Path) -> Option<Result<File, DevMapError>> {
    let (root_path, tail) = split(path).ok()?;
    Some(checked_parts(path, root_path, tail, || {}))
}

#[cfg(test)]
pub(crate) fn checked_file(path: &Path) -> Result<File, DevMapError> {
    checked_with_hook(path, || {})
}

#[cfg(test)]
fn checked_with_hook(path: &Path, after_first: impl FnOnce()) -> Result<File, DevMapError> {
    let (root_path, tail) = split(path)?;
    checked_parts(path, root_path, tail, after_first)
}

fn checked_parts(
    path: &Path,
    root_path: PathBuf,
    mut tail: Vec<u16>,
    after_first: impl FnOnce(),
) -> Result<File, DevMapError> {
    let root = open_directory_nofollow(&root_path)?;
    let root_identity = file_identity(&root)?;
    let file = relative_file(&root, &mut tail)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_owned()));
    }
    after_first();
    let reopened = relative_file(&root, &mut tail)?;
    let closing_metadata = reopened.metadata()?;
    if !closing_metadata.is_file()
        || is_link_or_reparse(&closing_metadata)
        || file_identity(&file)? != file_identity(&reopened)?
        || root_identity != file_identity(&open_directory_nofollow(&root_path)?)?
    {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_owned()));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn ordinary_unicode_and_long_files_match_original_identity_and_bytes() {
        let owned = tempfile::tempdir().unwrap();
        let mut parent = fs::canonicalize(owned.path()).unwrap();
        for _ in 0..12 {
            parent.push("long-directory-component");
        }
        fs::create_dir_all(&parent).unwrap();
        for path in [owned.path().join("中文-file"), parent.join("long-file")] {
            fs::write(&path, b"exact bytes\0\r\n").unwrap();
            let mut file = checked_file(&path).unwrap();
            assert_eq!(
                file_identity(&file).unwrap(),
                file_identity(&super::super::checked_file(&path, false, false).unwrap()).unwrap()
            );
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, fs::read(&path).unwrap());
        }
    }

    #[test]
    fn ancestor_junction_is_refused_without_reading_target() {
        let owned = tempfile::tempdir().unwrap();
        let target = owned.path().join("target");
        let alias = owned.path().join("alias");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::write(target.join("nested/payload"), b"must not follow alias").unwrap();
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(checked_file(&alias.join("nested/payload")).is_err());
        assert!(checked_file(&target.join("nested/payload")).is_ok());
        fs::remove_dir(&alias).unwrap();
    }

    #[test]
    fn missing_directory_and_unsupported_namespace_are_refused() {
        let owned = tempfile::tempdir().unwrap();
        assert!(checked_file(&owned.path().join("missing")).is_err());
        assert!(checked_file(owned.path()).is_err());
        for path in [
            r"relative\file",
            r"C:relative",
            r"\\server\share\file",
            r"C:\dir\..\file",
            r"C:\dir\file:stream",
        ] {
            assert!(split(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn replacement_and_hardlink_observations_stay_live() {
        let owned = tempfile::tempdir().unwrap();
        let path = owned.path().join("payload");
        fs::write(&path, b"old").unwrap();
        let first = checked_file(&path).unwrap();
        let original_identity = file_identity(&first).unwrap();
        fs::rename(&path, owned.path().join("previous")).unwrap();
        fs::write(&path, b"new").unwrap();
        let mut current = checked_file(&path).unwrap();
        assert_ne!(file_identity(&current).unwrap(), original_identity);
        let mut bytes = Vec::new();
        current.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"new");
        let alias = owned.path().join("alias");
        fs::hard_link(&path, &alias).unwrap();
        let alias_file = checked_file(&alias).unwrap();
        assert_eq!(
            file_identity(&alias_file).unwrap(),
            file_identity(&current).unwrap()
        );
    }

    #[test]
    fn ambiguous_win32_names_decline_without_opening() {
        for path in [
            r"C:\dir\tail.",
            r"C:\dir\tail ",
            r"C:\dir\CON",
            r"C:\dir\con.txt",
            r"C:\dir\LPT¹.txt",
            r"C:\dir\COM9",
            r"C:\dir\CONOUT$",
            r"C:\dir\wild*",
            r"C:\dir\stream:name",
        ] {
            assert!(try_checked_file(Path::new(path)).is_none(), "{path}");
        }
    }

    #[test]
    fn replacement_between_native_opens_is_rejected() {
        let owned = tempfile::tempdir().unwrap();
        let path = owned.path().join("payload");
        fs::write(&path, b"first").unwrap();
        let result = checked_with_hook(&path, || {
            fs::rename(&path, owned.path().join("previous")).unwrap();
            fs::write(&path, b"second").unwrap();
        });
        assert!(matches!(
            result,
            Err(DevMapError::UnsafeInstallerOverwrite(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(fs::read(owned.path().join("previous")).unwrap(), b"first");
    }

    #[test]
    fn ancestor_junction_inserted_between_native_opens_is_rejected() {
        let owned = tempfile::tempdir().unwrap();
        let parent = owned.path().join("parent");
        let target = owned.path().join("target");
        for path in [&parent, &target] {
            assert!(path.is_absolute() && path.starts_with(owned.path()));
        }
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(parent.join("payload"), b"first").unwrap();
        let result = checked_with_hook(&parent.join("payload"), || {
            // Windows refuses renaming the containing directory while its file
            // is open. Move the share-delete file first, then replace the empty
            // parent with a junction to the SAME file identity. Identity checks
            // alone would accept it if the kernel followed the new junction.
            fs::rename(parent.join("payload"), target.join("payload")).unwrap();
            fs::remove_dir(&parent).unwrap();
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&parent)
                .arg(&target)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        });
        assert!(result.is_err());
        fs::remove_dir(&parent).unwrap();
        assert_eq!(fs::read(target.join("payload")).unwrap(), b"first");
    }

    #[test]
    fn handles_close_on_success_and_second_open_failure() {
        const CHILD: &str = "DEVMAP_NOREPARSE_HANDLE_CHILD";
        if std::env::var(CHILD).as_deref() != Ok("1") {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "fs_security::read_no_reparse::tests::handles_close_on_success_and_second_open_failure", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        fn count() -> u32 {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
            let mut count = 0;
            // SAFETY: the pseudo-handle refers to this process; count is writable.
            assert_ne!(
                unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
                0
            );
            count
        }
        let owned = tempfile::tempdir().unwrap();
        let path = owned.path().join("payload");
        fs::write(&path, b"warm").unwrap();
        drop(checked_file(&path).unwrap());
        let before = count();
        for _ in 0..128 {
            fs::write(&path, b"current").unwrap();
            drop(checked_file(&path).unwrap());
            assert!(checked_with_hook(&path, || fs::remove_file(&path).unwrap()).is_err());
        }
        assert_eq!(count(), before, "owned native handles leaked");
    }
}
