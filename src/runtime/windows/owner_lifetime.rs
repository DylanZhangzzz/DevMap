use std::{
    io,
    os::windows::io::{FromRawHandle, IntoRawHandle, OwnedHandle},
    sync::Mutex,
};
use windows_sys::Win32::System::{JobObjects::*, Threading::GetCurrentProcess};

// This handle belongs to the dedicated owner process, not a client connection.
// Keep it until OS teardown: closing it while the owner lives would kill self.
// Descendants inherit membership at creation, before their per-command Job is
// attached. The handle itself is non-inheritable and breakaway is not enabled.
static OWNER_JOB: Mutex<Option<usize>> = Mutex::new(None);

pub(crate) fn install() -> io::Result<()> {
    let mut retained = OWNER_JOB
        .lock()
        .map_err(|_| io::Error::other("owner Job lock poisoned"))?;
    if retained.is_some() {
        return Ok(());
    }
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if AssignProcessToJobObject(raw, GetCurrentProcess()) == 0 {
            return Err(io::Error::last_os_error());
        }
        *retained = Some(job.into_raw_handle() as usize);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
