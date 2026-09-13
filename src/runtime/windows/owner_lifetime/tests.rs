use super::*;
use std::{
    fs,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    },
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
struct SuspendedChild(OwnedHandle);
impl Drop for SuspendedChild {
    fn drop(&mut self) {
        unsafe {
            if WaitForSingleObject(self.0.as_raw_handle(), 0) == WAIT_TIMEOUT {
                TerminateProcess(self.0.as_raw_handle(), 1);
                WaitForSingleObject(self.0.as_raw_handle(), 5000);
            }
        }
    }
}

#[test]
fn owner_job_crash_fixture() {
    let Ok(mode) = std::env::var("DEVMAP_OWNER_JOB_CRASH_MODE") else {
        return;
    };
    assert!(["unprotected", "protected", "nested"].contains(&mode.as_str()));
    if mode != "unprotected" {
        install().unwrap();
        install().unwrap();
    }
    let child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .creation_flags(0x08000000 | 0x00000004)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let _nested = if mode == "nested" {
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            assert!(!raw.is_null());
            let job = OwnedHandle::from_raw_handle(raw);
            assert_ne!(AssignProcessToJobObject(raw, child.0.as_raw_handle()), 0);
            Some(job)
        }
    } else {
        None
    };
    let marker =
        std::path::PathBuf::from(std::env::var_os("DEVMAP_OWNER_JOB_CRASH_MARKER").unwrap());
    fs::write(marker.with_extension("tmp"), child.0.id().to_string()).unwrap();
    fs::rename(marker.with_extension("tmp"), &marker).unwrap();
    // The external parent terminates this process exactly before attach/resume.
    // A bounded fallback and OwnedChild prevent leakage if the parent fails.
    std::thread::sleep(Duration::from_secs(20));
}

#[test]
fn hard_owner_exit_reaps_children_before_attach_and_inside_nested_job() {
    for mode in ["unprotected", "protected", "nested"] {
        let fixture = tempfile::tempdir().unwrap();
        let marker = fixture.path().join("child-pid");
        let mut owner = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "runtime::windows::owner_lifetime::tests::owner_job_crash_fixture",
                    "--test-threads=1",
                ])
                .env("DEVMAP_OWNER_JOB_CRASH_MODE", mode)
                .env("DEVMAP_OWNER_JOB_CRASH_MARKER", &marker)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                owner.0.try_wait().unwrap().is_none(),
                "{mode}: owner exited before marker"
            );
            assert!(Instant::now() < deadline, "{mode}: child marker deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = fs::read_to_string(marker).unwrap().parse().unwrap();
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
        assert!(!raw.is_null());
        let child = SuspendedChild(unsafe { OwnedHandle::from_raw_handle(raw) });
        assert_eq!(
            unsafe { WaitForSingleObject(child.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        let status = unsafe {
            WaitForSingleObject(
                child.0.as_raw_handle(),
                if mode == "unprotected" { 100 } else { 5000 },
            )
        };
        assert_eq!(
            status,
            if mode == "unprotected" {
                WAIT_TIMEOUT
            } else {
                WAIT_OBJECT_0
            },
            "{mode}"
        );
        // Unprotected control is deliberately cleaned by its retained handle.
        drop(child);
    }
}
