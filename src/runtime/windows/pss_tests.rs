use super::*;
use std::{process::Stdio, time::Duration};

#[test]
#[ignore = "owned suspended child only"]
fn suspended_fixture() {
    let Some(marker) = std::env::var_os("DEVMAP_PSS_OWNED_MARKER") else {
        return;
    };
    fs::write(marker, b"ran").unwrap();
}

fn scenario(fault: ResumeTestFault) -> (io::Result<ResumePath>, bool) {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("ran");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "runtime::windows::pss_tests::suspended_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("DEVMAP_PSS_OWNED_MARKER", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        prepare_identity(&mut command);
        let mut child = command.spawn().unwrap();
        assert!(
            !marker.exists(),
            "suspended child executed before owned assignment"
        );
        let result = match IdentityTree::attach_test(&child, fault) {
            Ok((mut tree, path)) => {
                // Keep the real Job alive through completion, then verify empty.
                let exited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                let cleanup = tree.terminate_and_wait().await;
                cleanup.expect("owned Job cleanup must be confirmed");
                exited.expect("owned child deadline").unwrap();
                Ok(path)
            }
            Err(error) => {
                // Failed attach must leave the original child owned and reapable.
                let _ = child.start_kill();
                tokio::time::timeout(Duration::from_secs(5), child.wait())
                    .await
                    .expect("failed attach child cleanup deadline")
                    .unwrap();
                Err(error)
            }
        };
        (result, marker.exists())
    });
    drop(runtime);
    result
}

#[test]
fn suspended_owned_child_uses_target_pss() {
    let (path, ran) = scenario(ResumeTestFault::None);
    assert_eq!(path.unwrap(), ResumePath::Pss);
    assert!(ran);
}
#[test]
fn capture_unavailable_uses_owned_toolhelp_fallback() {
    let (path, ran) = scenario(ResumeTestFault::CaptureUnavailable);
    assert_eq!(path.unwrap(), ResumePath::Toolhelp);
    assert!(ran);
}
#[test]
fn mismatched_thread_identity_refuses_resume_and_reaps() {
    let (path, ran) = scenario(ResumeTestFault::ForeignThread);
    assert!(
        path.is_err(),
        "thread identity mismatch cannot fall back to resume"
    );
    assert!(!ran, "rejected identity must never run");
    assert_eq!(
        path.unwrap_err().to_string(),
        "identity PSS thread owner mismatch"
    );
}
#[test]
fn resume_failure_refuses_fallback_and_reaps() {
    let (path, ran) = scenario(ResumeTestFault::ResumeFailure);
    assert!(
        path.is_err(),
        "ambiguous resume failure must not retry another thread"
    );
    assert!(!ran, "failed resume must never run");
    assert_eq!(
        path.unwrap_err().to_string(),
        "controlled identity resume failure"
    );
}

#[test]
fn thread_bound_violation_refuses_fallback_and_reaps() {
    let (path, ran) = scenario(ResumeTestFault::ResourceBound);
    assert!(
        path.is_err(),
        "bounded enumeration violation must fail closed"
    );
    assert!(!ran);
    assert_eq!(
        path.unwrap_err().to_string(),
        "identity PSS thread bound exceeded"
    );
}

fn rejects(fault: ResumeTestFault, expected: &str) {
    // scenario reaps the actual child before any of these assertions.
    let (path, ran) = scenario(fault);
    assert_eq!(path.unwrap_err().to_string(), expected);
    assert!(
        !ran,
        "refused thread must not execute the owned marker fixture"
    );
}
#[test]
fn opened_thread_owner_mismatch_is_terminal() {
    rejects(
        ResumeTestFault::OpenedOwnerMismatch,
        "identity opened thread owner mismatch",
    );
}
#[test]
fn opened_thread_creation_mismatch_is_terminal() {
    rejects(
        ResumeTestFault::OpenedCreationMismatch,
        "identity opened thread creation changed",
    );
}
#[test]
fn controlled_marker_release_failure_leaves_drop_cleanup() {
    rejects(
        ResumeTestFault::MarkerReleaseFailure,
        "controlled PSS marker release failure",
    );
}
#[test]
fn controlled_snapshot_release_failure_leaves_drop_cleanup() {
    rejects(
        ResumeTestFault::SnapshotReleaseFailure,
        "controlled PSS snapshot release failure",
    );
}
