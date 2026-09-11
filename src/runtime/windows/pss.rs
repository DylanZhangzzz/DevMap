//! Target-only thread discovery after suspended child assignment to its Job.
use super::*;
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_ITEMS, FILETIME, HANDLE},
    System::{
        Diagnostics::ProcessSnapshotting::*,
        Threading::{
            GetProcessIdOfThread, GetThreadTimes, OpenThread, ResumeThread,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
        },
    },
};

struct Snapshot(HPSS);
impl Drop for Snapshot {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                PssFreeSnapshot(GetCurrentProcess(), self.0);
            }
        }
    }
}
struct Marker(HPSSWALK);
impl Drop for Marker {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                PssWalkMarkerFree(self.0);
            }
        }
    }
}
struct Thread(HANDLE);
impl Drop for Thread {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn status(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// false means capture unavailable before selecting/resuming any thread. Later
/// identity, bounds, enumeration or resume failures never permit a second path.
pub(super) fn resume(
    child: &tokio::process::Child,
    #[cfg(test)] fault: ResumeTestFault,
) -> io::Result<bool> {
    #[cfg(test)]
    if matches!(fault, ResumeTestFault::CaptureUnavailable) {
        return Ok(false);
    }
    let target = child
        .id()
        .ok_or_else(|| invalid("identity child PID missing"))?;
    let process = child
        .raw_handle()
        .ok_or_else(|| invalid("identity child handle missing"))?;
    let mut snapshot = Snapshot(ptr::null_mut());
    // Only thread IDs/state; no VA clone, process handles or context capture.
    let code = unsafe { PssCaptureSnapshot(process, PSS_CAPTURE_THREADS, 0, &mut snapshot.0) };
    if code != 0 {
        return Ok(false);
    }
    let mut marker = Marker(ptr::null_mut());
    status(unsafe { PssWalkMarkerCreate(ptr::null(), &mut marker.0) })?;
    let mut selected = None;
    let mut count = 0;
    loop {
        let mut entry: PSS_THREAD_ENTRY = unsafe { mem::zeroed() };
        let code = unsafe {
            PssWalkSnapshot(
                snapshot.0,
                PSS_WALK_THREADS,
                marker.0,
                (&mut entry as *mut PSS_THREAD_ENTRY).cast(),
                mem::size_of_val(&entry) as u32,
            )
        };
        if code == ERROR_NO_MORE_ITEMS {
            break;
        }
        status(code)?;
        count += 1;
        #[cfg(test)]
        if matches!(fault, ResumeTestFault::ResourceBound) {
            count = 9;
        }
        if count > 8 {
            return Err(invalid("identity PSS thread bound exceeded"));
        }
        #[cfg(test)]
        if matches!(fault, ResumeTestFault::ForeignThread) {
            entry.ProcessId = target.wrapping_add(1);
        }
        if entry.ProcessId != target {
            return Err(invalid("identity PSS thread owner mismatch"));
        }
        if entry.Flags & PSS_THREAD_FLAGS_TERMINATED != 0 {
            continue;
        }
        if selected
            .replace((entry.ThreadId, entry.CreateTime))
            .is_some()
        {
            return Err(invalid("identity PSS initial thread ambiguous"));
        }
    }
    let (id, created) = selected.ok_or_else(|| invalid("identity PSS initial thread missing"))?;
    let thread = Thread(unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            0,
            id,
        )
    });
    if thread.0.is_null() {
        return Err(io::Error::last_os_error());
    }
    let owner = unsafe { GetProcessIdOfThread(thread.0) };
    #[cfg(test)]
    let owner = if matches!(fault, ResumeTestFault::OpenedOwnerMismatch) {
        target.wrapping_add(1)
    } else {
        owner
    };
    if owner != target {
        return Err(invalid("identity opened thread owner mismatch"));
    }
    let mut actual: FILETIME = unsafe { mem::zeroed() };
    let mut exit: FILETIME = unsafe { mem::zeroed() };
    let mut kernel: FILETIME = unsafe { mem::zeroed() };
    let mut user: FILETIME = unsafe { mem::zeroed() };
    if unsafe { GetThreadTimes(thread.0, &mut actual, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(test)]
    if matches!(fault, ResumeTestFault::OpenedCreationMismatch) {
        actual.dwLowDateTime = created.dwLowDateTime.wrapping_add(1);
    }
    if actual.dwHighDateTime != created.dwHighDateTime
        || actual.dwLowDateTime != created.dwLowDateTime
    {
        return Err(invalid("identity opened thread creation changed"));
    }
    // Release capture resources before execution. Failure leaves the child
    // suspended in the owned kill-on-close Job and is never downgraded.
    #[cfg(test)]
    if matches!(fault, ResumeTestFault::MarkerReleaseFailure) {
        return Err(invalid("controlled PSS marker release failure"));
    }
    status(unsafe { PssWalkMarkerFree(marker.0) })?;
    marker.0 = ptr::null_mut();
    #[cfg(test)]
    if matches!(fault, ResumeTestFault::SnapshotReleaseFailure) {
        return Err(invalid("controlled PSS snapshot release failure"));
    }
    status(unsafe { PssFreeSnapshot(GetCurrentProcess(), snapshot.0) })?;
    snapshot.0 = ptr::null_mut();
    #[cfg(test)]
    if matches!(fault, ResumeTestFault::ResumeFailure) {
        return Err(invalid("controlled identity resume failure"));
    }
    let prior = unsafe { ResumeThread(thread.0) };
    if prior == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    if prior != 1 {
        return Err(invalid("identity primary thread suspension changed"));
    }
    Ok(true)
}
