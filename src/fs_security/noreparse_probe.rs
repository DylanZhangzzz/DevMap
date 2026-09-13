//! Windows-only experiment; not compiled into or used by production queries.
//! Ask the kernel to reject reparse traversal on the complete relative path.
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
        "noreparse probe requires a normal absolute disk path",
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
        if !tail.is_empty() {
            tail.push(u16::from(b'\\'));
        }
        tail.extend(name.encode_wide());
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
        return Err(std::io::Error::other("noreparse probe returned a null handle").into());
    }
    // SAFETY: a successful synchronous open transferred this unique handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

pub(crate) fn checked_file(path: &Path) -> Result<File, DevMapError> {
    let (root_path, mut tail) = split(path)?;
    let root = open_directory_nofollow(&root_path)?;
    let root_identity = file_identity(&root)?;
    let file = relative_file(&root, &mut tail)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_owned()));
    }
    let reopened = relative_file(&root, &mut tail)?;
    if file_identity(&file)? != file_identity(&reopened)?
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
}
