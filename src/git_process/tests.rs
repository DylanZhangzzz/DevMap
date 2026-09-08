//! Private process fault tests, included by git_process.rs under cfg(test).
use super::*;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn limits() -> ProcessLimits {
    ProcessLimits {
        command_timeout: Duration::from_millis(700),
        cleanup_timeout: Duration::from_secs(1),
        stdout_bytes: 128 * 1024,
        stderr_bytes: 128 * 1024,
    }
}
fn helper(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "git_process::tests::process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("DEVMAP_GIT_PROCESS_FIXTURE", mode);
    command
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "owned fixture failed to start: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "owned subprocess fixture only"]
fn process_fixture() {
    let Ok(mode) = std::env::var("DEVMAP_GIT_PROCESS_FIXTURE") else {
        return;
    };
    match mode.as_str() {
        "normal" => {
            std::io::stdout().write_all(b"normal\0bytes").unwrap();
            std::io::stderr().write_all(b"stderr retained").unwrap();
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            std::process::exit(128);
        }
        "short" => std::thread::sleep(Duration::from_millis(220)),
        "stdout" => {
            std::io::stdout()
                .write_all(&vec![b'x'; 1024 * 1024])
                .unwrap();
        }
        "stderr" => {
            std::io::stderr()
                .write_all(&vec![b'x'; 1024 * 1024])
                .unwrap();
        }
        "both" => {
            std::thread::scope(|scope| {
                scope.spawn(|| std::io::stdout().write_all(&vec![b'a'; 96 * 1024]).unwrap());
                scope.spawn(|| std::io::stderr().write_all(&vec![b'b'; 96 * 1024]).unwrap());
            });
        }
        "descendant" | "marked-hang" => {
            let marker = std::env::var_os("DEVMAP_GIT_PROCESS_MARKER").unwrap();
            fs::write(marker, std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "tree-pipes" | "tree-null" => {
            let mut child_command = helper("descendant");
            if mode == "tree-null" {
                child_command.stdout(Stdio::null()).stderr(Stdio::null());
            }
            let child = child_command.spawn().unwrap();
            let marker =
                std::path::PathBuf::from(std::env::var_os("DEVMAP_GIT_PROCESS_MARKER").unwrap());
            wait_file(&marker);
            wait_file(&marker.with_extension("release"));
            // Deliberately leave this owned descendant to the tested supervisor.
            drop(child);
        }
        "hang" => std::thread::sleep(Duration::from_secs(30)),
        other => panic!("unknown owned helper mode {other}"),
    }
    std::io::stdout().flush().unwrap();
    std::io::stderr().flush().unwrap();
    std::process::exit(0);
}

#[test]
fn ordinary_exit_and_binary_streams_are_preserved() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let output = output_with_limits(&mut helper("normal"), limits()).unwrap();
    assert_eq!(output.status.code(), Some(128));
    assert!(output.stdout.ends_with(b"normal\0bytes"));
    assert_eq!(output.stderr, b"stderr retained");
    let git = super::output(Command::new("git").arg("--version")).unwrap();
    assert!(git.status.success());
    assert!(git.stdout.starts_with(b"git version "));
}

#[test]
fn hang_has_finite_cleanup_and_next_command_works() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let started = Instant::now();
    assert!(matches!(
        output_with_limits(&mut helper("hang"), limits()),
        Err(GitProcessError::Deadline)
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(output_with_limits(&mut helper("normal"), limits()).is_ok());
}

#[test]
fn stdout_and_stderr_overflow_are_explicit() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(matches!(
        output_with_limits(&mut helper("stdout"), limits()),
        Err(GitProcessError::StdoutLimit)
    ));
    assert!(matches!(
        output_with_limits(&mut helper("stderr"), limits()),
        Err(GitProcessError::StderrLimit)
    ));
}

#[test]
fn simultaneous_streams_are_drained_and_exact_cap_is_accepted() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut policy = limits();
    policy.command_timeout = Duration::from_secs(3);
    let initial = output_with_limits(&mut helper("both"), policy.clone()).unwrap();
    assert!(initial.status.success());
    assert_eq!(initial.stderr.len(), 96 * 1024);
    policy.stdout_bytes = initial.stdout.len();
    policy.stderr_bytes = initial.stderr.len();
    let exact = output_with_limits(&mut helper("both"), policy.clone()).unwrap();
    assert_eq!(exact.stdout, initial.stdout);
    assert_eq!(exact.stderr, initial.stderr);
    policy.stderr_bytes -= 1;
    assert!(matches!(
        output_with_limits(&mut helper("both"), policy),
        Err(GitProcessError::StderrLimit)
    ));
}

#[test]
fn commands_share_one_absolute_operation_deadline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let budget = GitBudget::new(Duration::from_millis(500));
    let started = Instant::now();
    with_budget(&budget, || {
        let mut policy = limits();
        policy.command_timeout = Duration::from_secs(3);
        output_with_limits(&mut helper("short"), policy.clone()).unwrap();
        assert!(matches!(
            output_with_limits(&mut helper("hang"), policy),
            Err(GitProcessError::Deadline)
        ));
    });
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn explicitly_propagated_worker_budget_cannot_renew_expired_request() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let budget = GitBudget::new(Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(10));
    std::thread::scope(|scope| {
        let budget = budget.clone();
        scope
            .spawn(move || {
                with_budget(&budget, || {
                    assert!(matches!(
                        output_with_limits(&mut helper("normal"), limits()),
                        Err(GitProcessError::Deadline)
                    ));
                })
            })
            .join()
            .unwrap();
    });
}

#[cfg(windows)]
#[test]
fn owned_descendant_handles_signal_with_and_without_inherited_pipes() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    for mode in ["tree-pipes", "tree-null"] {
        let fixture = tempfile::tempdir().unwrap();
        let marker = fixture.path().join("owned-child");
        std::thread::scope(|scope| {
            let runner_marker = marker.clone();
            let runner = scope.spawn(move || {
                let mut command = helper(mode);
                command.env("DEVMAP_GIT_PROCESS_MARKER", &runner_marker);
                let mut policy = limits();
                policy.command_timeout = Duration::from_secs(5);
                output_with_limits(&mut command, policy)
            });
            wait_file(&marker);
            let pid = fs::read_to_string(&marker).unwrap().parse::<u32>().unwrap();
            let owned_handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(
                !owned_handle.is_null(),
                "fixture must be alive before supervisor cleanup"
            );
            fs::write(marker.with_extension("release"), b"release").unwrap();
            let result = runner.join().unwrap();
            let signalled = unsafe { WaitForSingleObject(owned_handle, 0) };
            unsafe {
                CloseHandle(owned_handle);
            }
            assert_eq!(
                signalled, WAIT_OBJECT_0,
                "supervisor returned while owned descendant still executed"
            );
            assert!(
                result.is_ok(),
                "normal leader completion should clean descendants: {result:?}"
            );
        });
    }
}

#[test]
fn admission_wait_uses_deadline_without_starting_a_fifth_child() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for index in 0..4 {
            let marker = fixture.path().join(index.to_string());
            workers.push(scope.spawn(move || {
                let mut command = helper("marked-hang");
                command.env("DEVMAP_GIT_PROCESS_MARKER", marker);
                let mut policy = limits();
                policy.command_timeout = Duration::from_secs(3);
                output_with_limits(&mut command, policy)
            }));
        }
        for index in 0..4 {
            wait_file(&fixture.path().join(index.to_string()));
        }
        let never_started = fixture.path().join("fifth");
        let mut command = helper("marked-hang");
        command.env("DEVMAP_GIT_PROCESS_MARKER", &never_started);
        let budget = GitBudget::new(Duration::from_millis(100));
        assert!(matches!(
            with_budget(&budget, || output_with_limits(&mut command, limits())),
            Err(GitProcessError::Deadline)
        ));
        assert!(!never_started.exists());
        for worker in workers {
            assert!(matches!(
                worker.join().unwrap(),
                Err(GitProcessError::Deadline)
            ));
        }
    });
}
