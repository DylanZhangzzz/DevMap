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
) -> io::Result<SpawnedOwner> {
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
        Ok(SpawnedOwner {
            handle: process.hProcess,
            confirmed: false,
        })
    }
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

pub fn prepare_identity(command: &mut tokio::process::Command) {
    // Start suspended so no descendant can escape before job assignment.
    command.creation_flags(0x08000000 | 0x00000004);
}
pub struct IdentityTree(windows_sys::Win32::Foundation::HANDLE);
mod pss;
#[cfg(test)]
mod pss_tests;
#[cfg(test)]
#[derive(Clone, Copy)]
enum ResumeTestFault {
    None,
    CaptureUnavailable,
    ForeignThread,
    ResourceBound,
    ResumeFailure,
    OpenedOwnerMismatch,
    OpenedCreationMismatch,
    MarkerReleaseFailure,
    SnapshotReleaseFailure,
}
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum ResumePath {
    Pss,
    Toolhelp,
}
#[cfg(test)]
impl IdentityTree {
    fn attach_test(
        child: &tokio::process::Child,
        fault: ResumeTestFault,
    ) -> io::Result<(Self, ResumePath)> {
        Self::attach_inner(child, fault).map(|(tree, pss)| {
            (
                tree,
                if pss {
                    ResumePath::Pss
                } else {
                    ResumePath::Toolhelp
                },
            )
        })
    }
}
// HANDLE ownership is unique and kernel operations are thread safe.
unsafe impl Send for IdentityTree {}
impl Drop for IdentityTree {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl IdentityTree {
    pub fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        Self::attach_inner(
            child,
            #[cfg(test)]
            ResumeTestFault::None,
        )
        .map(|(tree, _)| tree)
    }
    fn attach_inner(
        child: &tokio::process::Child,
        #[cfg(test)] fault: ResumeTestFault,
    ) -> io::Result<(Self, bool)> {
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::{
                Diagnostics::ToolHelp::*,
                JobObjects::*,
                Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
            },
        };
        unsafe {
            let job = CreateJobObjectW(ptr::null(), ptr::null());
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let tree = Self(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = 8;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(
                    job,
                    child
                        .raw_handle()
                        .ok_or_else(|| invalid("identity child handle missing"))?,
                ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if pss::resume(
                child,
                #[cfg(test)]
                fault,
            )? {
                return Ok((tree, true));
            }
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = mem::zeroed();
            entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
            let mut found = false;
            let mut next = Thread32First(snapshot, &mut entry);
            while next != 0 {
                if Some(entry.th32OwnerProcessID) == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        found = ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                    break;
                }
                next = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if !found {
                return Err(invalid("identity primary thread could not resume"));
            }
            Ok((tree, false))
        }
    }
}

/// Parent rights matter: DELETE_CHILD can replace even a private child directory.
pub fn validate_parent(path: &Path) -> io::Result<()> {
    validate_directory(path, false)
}

/// Validate every existing component before trusting a new private descendant.
/// An ancestor may permit creating a sibling directory without permitting any
/// existing component to be removed, replaced, or have its security changed.
pub fn validate_chain(path: &Path) -> io::Result<()> {
    for (depth, ancestor) in path.ancestors().enumerate() {
        if depth == 0 {
            validate_parent(ancestor)?;
        } else {
            validate_directory(ancestor, true)?;
        }
    }
    Ok(())
}

fn validate_directory(path: &Path, ancestor: bool) -> io::Result<()> {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) => {}
        _ => return Err(invalid("runtime parent must be a local drive path")),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid("runtime parent is not a plain directory"));
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
        let user = current_sid()?;
        // TrustedInstaller owns normal Windows volume/system ancestors. It is
        // an OS service identity, not an arbitrary SID discovered on this path.
        let trusted = |sid: &str| {
            sid == user
                || sid == "S-1-5-18"
                || sid == "S-1-5-32-544"
                || sid == "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
        };
        if owner.is_null() || !trusted(&sid_string(owner)?) || acl.is_null() {
            return Err(invalid("runtime parent ownership is untrusted"));
        }
        const MUTATION: u32 = 0x10000000 | 0x40000000 | 0x000d0156;
        // FILE_ADD_SUBDIRECTORY alone does not authorize replacement of an
        // existing child. C:\ commonly grants it to Authenticated Users.
        let forbidden = if ancestor { MUTATION & !0x4 } else { MUTATION };
        for index in 0..(*acl).AceCount {
            let mut raw = ptr::null_mut();
            if GetAce(acl, index as u32, &mut raw) == 0 {
                return Err(io::Error::last_os_error());
            }
            let ace = &*(raw.cast::<ACCESS_ALLOWED_ACE>());
            if ace.Header.AceFlags & 8 != 0 || ace.Header.AceType == 1 {
                continue;
            }
            if ace.Header.AceType != 0 {
                return Err(invalid("runtime parent has unsupported access rule"));
            }
            let sid = sid_string((&ace.SidStart as *const u32).cast_mut().cast())?;
            if !trusted(&sid) && ace.Mask & forbidden != 0 {
                return Err(invalid("runtime parent permits untrusted mutation"));
            }
        }
        Ok(())
    }
}
pub fn runtime_base() -> io::Result<std::path::PathBuf> {
    let temporary = std::env::temp_dir();
    if validate_chain(&temporary).is_ok() {
        return Ok(temporary);
    }
    let profile =
        std::env::var_os("USERPROFILE").ok_or_else(|| invalid("no trusted runtime parent"))?;
    let profile = std::path::PathBuf::from(profile);
    validate_chain(&profile)?;
    Ok(profile)
}

impl IdentityTree {
    pub async fn terminate_and_wait(&mut self) -> io::Result<()> {
        use windows_sys::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::{
                JobObjects::*,
                Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
            },
        };
        struct ProcessHandle(windows_sys::Win32::Foundation::HANDLE);
        unsafe impl Send for ProcessHandle {}
        impl Drop for ProcessHandle {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let mut processes = Vec::new();
        unsafe {
            let mut buffer = [0usize; 16];
            if QueryInformationJobObject(
                self.0,
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                mem::size_of_val(&buffer) as u32,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let list = &*(buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>());
            if list.NumberOfProcessIdsInList > 8 {
                return Err(invalid("identity job process bound exceeded"));
            }
            let ids = std::slice::from_raw_parts(
                list.ProcessIdList.as_ptr(),
                list.NumberOfProcessIdsInList as usize,
            );
            for id in ids {
                let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, *id as u32);
                if !handle.is_null() {
                    processes.push(ProcessHandle(handle));
                }
            }
            if TerminateJobObject(self.0, 1) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let mut done = true;
            for process in &processes {
                match unsafe { WaitForSingleObject(process.0, 0) } {
                    WAIT_OBJECT_0 => {}
                    WAIT_TIMEOUT => done = false,
                    _ => return Err(io::Error::last_os_error()),
                }
            }
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { mem::zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    mem::size_of_val(&accounting) as u32,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if done && accounting.ActiveProcesses == 0 {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(invalid("identity tree termination deadline"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

pub struct SpawnedOwner {
    handle: windows_sys::Win32::Foundation::HANDLE,
    confirmed: bool,
}
impl SpawnedOwner {
    pub fn detach(mut self) {
        self.confirmed = true;
    }
}
impl Drop for SpawnedOwner {
    fn drop(&mut self) {
        unsafe {
            if !self.confirmed {
                windows_sys::Win32::System::Threading::TerminateProcess(self.handle, 1);
                windows_sys::Win32::System::Threading::WaitForSingleObject(self.handle, 1000);
            }
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod candidate_tests {
    use super::*;
    #[test]
    fn candidate_guard_cleans_only_unconfirmed_owned_process() {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
        for confirmed in [false, true] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "runtime::bounded_identity_tests::identity_descendant_fixture",
                    "--nocapture",
                ])
                .env("DEVMAP_TEST_IDENTITY_DESCENDANT", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap();
            let mut handle = ptr::null_mut();
            assert_ne!(
                unsafe {
                    DuplicateHandle(
                        GetCurrentProcess(),
                        child.as_raw_handle(),
                        GetCurrentProcess(),
                        &mut handle,
                        0,
                        0,
                        DUPLICATE_SAME_ACCESS,
                    )
                },
                0
            );
            let guard = SpawnedOwner {
                handle,
                confirmed: false,
            };
            if confirmed {
                guard.detach();
                assert!(child.try_wait().unwrap().is_none());
                child.kill().unwrap();
            } else {
                drop(guard);
                assert!(child.try_wait().unwrap().is_some());
            }
            child.wait().unwrap();
        }
    }
}
