//! Windows security boundary: current TokenUser SID (not logon SID), protected
//! owner-only DACL, local-only pipe, and server ownership validation by clients.
use super::transport::invalid;
use std::{
    ffi::c_void,
    fs, io, mem,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
    ptr,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
pub type Client = NamedPipeClient;
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn current_sid() -> io::Result<String> {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
        // usize allocation gives TOKEN_USER sufficient alignment.
        let mut buffer = vec![0usize; (size as usize).div_ceil(mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buffer.as_ptr().cast::<TOKEN_USER>());
        sid_string(user.User.Sid)
    }
}
unsafe fn sid_string(sid: PSID) -> io::Result<String> {
    unsafe {
        let mut string = ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut string) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _allocation = LocalAllocation(string.cast());
        let mut len = 0;
        while *string.add(len) != 0 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            string, len,
        )))
    }
}
pub fn user_id() -> io::Result<String> {
    current_sid()
}
struct Security(LocalAllocation);
impl Security {
    fn new() -> io::Result<Self> {
        let sid = current_sid()?;
        let sddl = wide(format!("O:{sid}D:P(A;OICI;GA;;;{sid})"));
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(LocalAllocation(descriptor)))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
    }
}
unsafe fn validate_descriptor(owner: PSID, acl: *mut ACL) -> io::Result<()> {
    unsafe {
        let sid = current_sid()?;
        if owner.is_null() || sid_string(owner)? != sid || acl.is_null() || (*acl).AceCount != 1 {
            return Err(invalid("runtime security descriptor is not owner-only"));
        }
        let mut ace = ptr::null_mut();
        if GetAce(acl, 0, &mut ace) == 0 {
            return Err(io::Error::last_os_error());
        }
        let allow = &*(ace.cast::<ACCESS_ALLOWED_ACE>());
        if allow.Header.AceType != 0
            || sid_string((&allow.SidStart as *const u32).cast_mut().cast())? != sid
        {
            return Err(invalid("runtime ACL grants another identity access"));
        }
        Ok(())
    }
}
pub fn private_dir(path: &Path) -> io::Result<()> {
    let security = Security::new()?;
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), &security.attributes()) } == 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(e);
        }
    }
    validate(path, true)
}
pub fn validate(path: &Path, directory: bool) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || m.is_dir() != directory {
        return Err(invalid(
            "runtime path is a reparse point or wrong file type",
        ));
    }
    unsafe {
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let result = GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let _allocation = LocalAllocation(descriptor);
        validate_descriptor(owner, acl)
    }
}
pub fn lock_file(path: &Path) -> io::Result<fs::File> {
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate(path, false)?;
    Ok(f)
}
pub async fn connect(endpoint: &str) -> io::Result<Client> {
    let client = ClientOptions::new().open(endpoint)?;
    unsafe {
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let result = GetSecurityInfo(
            client.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let _allocation = LocalAllocation(descriptor);
        validate_descriptor(owner, acl)?;
    }
    Ok(client)
}
pub struct Listener {
    pending: NamedPipeServer,
    endpoint: String,
}
impl Listener {
    fn create(endpoint: &str, first: bool) -> io::Result<NamedPipeServer> {
        let security = Security::new()?;
        // Descriptor is alive throughout synchronous CreateNamedPipeW.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .max_instances(18)
                .create_with_security_attributes_raw(
                    endpoint,
                    (&security.attributes() as *const SECURITY_ATTRIBUTES)
                        .cast_mut()
                        .cast(),
                )
        }
    }
    pub fn bind(endpoint: &str) -> io::Result<Self> {
        Ok(Self {
            pending: Self::create(endpoint, true)?,
            endpoint: endpoint.to_owned(),
        })
    }
    pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.pending.connect().await?;
        // Publish the next instance before returning the connected instance.
        let next = Self::create(&self.endpoint, false)?;
        Ok(mem::replace(&mut self.pending, next))
    }
}

/// No inherited handles: a detached owner must not keep MCP/client stdio pipes
/// alive. Rust's general Command spawn inherits other inheritable handles too.
pub fn spawn_owner(
    executable: &Path,
    source: &Path,
    instance: &str,
    idle_seconds: u64,
) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    fn quote(value: &std::ffi::OsStr) -> Vec<u16> {
        let mut result = vec![34];
        let mut slashes = 0;
        for c in value.encode_wide() {
            if c == 92 {
                slashes += 1;
                continue;
            }
            result.extend(std::iter::repeat_n(
                92,
                if c == 34 { slashes * 2 + 1 } else { slashes },
            ));
            slashes = 0;
            result.push(c);
        }
        result.extend(std::iter::repeat_n(92, slashes * 2));
        result.push(34);
        result
    }
    let args = [
        executable.as_os_str().to_owned(),
        "runtime".into(),
        "--source".into(),
        source.as_os_str().to_owned(),
        "--owner".into(),
        "--instance".into(),
        instance.into(),
        "--idle-seconds".into(),
        idle_seconds.to_string().into(),
    ];
    let mut command = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if i != 0 {
            command.push(32);
        }
        command.extend(quote(arg));
    }
    command.push(0);
    unsafe {
        let mut startup: STARTUPINFOW = mem::zeroed();
        startup.cb = mem::size_of::<STARTUPINFOW>() as u32;
        let mut process: PROCESS_INFORMATION = mem::zeroed();
        if CreateProcessW(
            wide(executable).as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_directory_rejects_inherited_acl_and_accepts_owner_only_acl() {
        let temp = tempfile::tempdir().unwrap();
        assert!(private_dir(temp.path()).is_err());
        let private = temp.path().join("private");
        private_dir(&private).unwrap();
        validate(&private, true).unwrap();
        let status = std::process::Command::new("icacls")
            .arg(&private)
            .args(["/grant", "*S-1-1-0:(OI)(CI)R"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(private_dir(&private).is_err());
    }
    #[test]
    fn client_rejects_pipe_with_default_security_descriptor() {
        super::super::reactor().unwrap().block_on(async {
            let endpoint = format!(
                r"\\.\pipe\devmap-insecure-test-{}",
                super::super::nonce().unwrap()
            );
            let _server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&endpoint)
                .unwrap();
            assert!(connect(&endpoint).await.is_err());
        });
    }
}
