use super::*;
use fs2::FileExt;
use std::{
    fs::OpenOptions,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "owned quarantine fixture deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn unconfirmed_git_cleanup_retains_owner_and_rejects_queued_work() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::executor::quarantine_tests::quarantine_fixture",
            "--ignored",
            "--nocapture",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "owned quarantine helper timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "isolated process health-poison fixture"]
fn quarantine_fixture() {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let temporary = tempfile::tempdir().unwrap();
    let marker = temporary.path().join("child.pid");
    let marker_for_job = marker.clone();
    let path = temporary.path().join("owner.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let job_calls = calls.clone();
    crate::git_process::test_force_cleanup_failure(true);
    let executor = Executor::start_with(move |_, _| {
        job_calls.fetch_add(1, Ordering::SeqCst);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "git_process::tests::process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("DEVMAP_GIT_PROCESS_FIXTURE", "descendant")
            .env("DEVMAP_GIT_PROCESS_MARKER", &marker_for_job);
        let budget = crate::git_process::GitBudget::new(Duration::from_millis(700));
        crate::git_process::with_budget(&budget, || crate::git_process::output(&mut command))?;
        unreachable!("quarantined Git must not return success")
    })
    .unwrap();
    let admission = executor.admission();
    let mut replies = Vec::new();
    for _ in 0..protocol::MAX_EXCHANGES {
        let reservation = admission
            .memory
            .clone()
            .try_acquire_many_owned(protocol::EXCHANGE_RESERVATION as u32)
            .unwrap();
        let (reply, receive) = oneshot::channel();
        let job = Job {
            query_origin: None,
            identity: Identity {
                source: ".".into(),
                git_dir: ".".into(),
                common: ".".into(),
                repository: "fixture".into(),
            },
            bytes: vec![1],
            reservation,
            reply,
        };
        // The second job must enter only after the first is executing.
        admission.sender.try_send(job).ok().unwrap();
        replies.push(receive);
        wait_until(|| calls.load(Ordering::SeqCst) == 1);
    }
    let mut pid = None;
    wait_until(|| {
        pid = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|s| s.parse::<u32>().ok());
        pid.is_some()
    });
    let pid = pid.unwrap();
    // Retain a handle to the PID reported by our own live child before cleanup.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null());
    wait_until(|| !crate::git_process::healthy());
    assert_eq!(unsafe { WaitForSingleObject(process, 0) }, WAIT_TIMEOUT);
    let memory = admission.memory.clone();
    drop(admission);
    let (done, finished) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        super::super::owner::finish_owner(super::super::reactor().unwrap(), lock, Ok(()), executor)
            .unwrap();
        done.send(()).unwrap();
    });
    assert!(finished.recv_timeout(Duration::from_millis(150)).is_err());
    assert!(probe.try_lock_exclusive().is_err());
    assert_eq!(memory.available_permits(), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    crate::git_process::test_force_cleanup_failure(false);
    finished.recv_timeout(Duration::from_secs(8)).unwrap();
    shutdown.join().unwrap();
    assert_eq!(unsafe { WaitForSingleObject(process, 0) }, WAIT_OBJECT_0);
    unsafe {
        CloseHandle(process);
    }
    assert!(!crate::git_process::healthy());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "queued work executed after health poison"
    );
    for reply in replies {
        let completed = reply.blocking_recv().unwrap();
        let result: ApplicationResult = serde_json::from_slice(&completed.bytes).unwrap();
        assert!(matches!(result, ApplicationResult::Error { .. }));
    }
    assert_eq!(
        memory.available_permits(),
        protocol::EXCHANGE_RESERVATION * protocol::MAX_EXCHANGES
    );
    probe.try_lock_exclusive().unwrap();
}
