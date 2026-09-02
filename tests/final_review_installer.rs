mod support;

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use devmap::adapter::{
    install_adapter, plan_adapter, plan_uninstall_adapter, uninstall_adapter, verify_adapter,
};
use devmap::cli::AdapterHost;
use devmap::error::DevMapError;
use devmap::events::CaptureGrade;
use serde_json::json;
use support::committed_repo;

#[test]
fn plan_digest_covers_exact_prior_bytes_identity_host_and_desired_result() {
    let repository = committed_repo();
    let path = repository.path().join(".codex/hooks.json");
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{}\n").unwrap();
    let first = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();

    fs::write(&path, b"{ }\n").unwrap();
    let exact_bytes_changed = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let other_host = plan_adapter(repository.path(), AdapterHost::Claude).unwrap();
    let removal = plan_uninstall_adapter(repository.path(), AdapterHost::Codex).unwrap();

    assert_ne!(first.plan_digest, exact_bytes_changed.plan_digest);
    assert_ne!(first.plan_digest, other_host.plan_digest);
    assert_ne!(exact_bytes_changed.plan_digest, removal.plan_digest);
    assert_eq!(first.plan_digest.len(), "sha256-".len() + 64);
}

#[test]
fn install_requires_the_exact_reviewed_plan_token() {
    let repository = committed_repo();
    let plan = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let path = plan.config_path.clone();

    let error = install_adapter(plan, "sha256-deadbeef")
        .expect_err("an unreviewed plan must not write configuration");

    assert!(matches!(error, DevMapError::AdapterApprovalMismatch));
    assert!(!path.exists());
}

#[test]
fn a_user_edit_between_plan_and_install_wins_the_compare_and_swap() {
    let repository = committed_repo();
    let path = repository.path().join(".codex/hooks.json");
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{\"hooks\":{}}\n").unwrap();
    let plan = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let token = plan.plan_digest.clone();
    let user_bytes = b"{ \"hooks\": {} }\n";
    fs::write(&path, user_bytes).unwrap();

    let error = install_adapter(plan, &token).expect_err("stale plan must fail closed");

    assert!(matches!(error, DevMapError::AdapterPlanStale(_)));
    assert_eq!(fs::read(path).unwrap(), user_bytes);
}

#[test]
fn two_installers_from_one_plan_cannot_both_commit() {
    let repository = committed_repo();
    let plan = plan_adapter(repository.path(), AdapterHost::Claude).unwrap();
    let token = plan.plan_digest.clone();
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let plan = plan.clone();
        let token = token.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            install_adapter(plan, &token)
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(DevMapError::AdapterPlanStale(_))))
            .count(),
        1
    );
    let verification = verify_adapter(repository.path(), AdapterHost::Claude).unwrap();
    assert!(verification.configured);
    assert_eq!(verification.capture_grade, CaptureGrade::D);
}

#[test]
fn a_user_edit_after_uninstall_review_is_never_removed() {
    let repository = committed_repo();
    let install = plan_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();
    let token = install.plan_digest.clone();
    install_adapter(install, &token).unwrap();
    let removal = plan_uninstall_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();
    let removal_token = removal.plan_digest.clone();
    let path = removal.config_path.clone();
    let user_bytes = b"{\"command\":[\"another\"],\"transport\":\"stdio\"}\n";
    fs::write(&path, user_bytes).unwrap();

    let error = uninstall_adapter(removal, &removal_token)
        .expect_err("uninstall must compare exact reviewed bytes");

    assert!(matches!(error, DevMapError::AdapterPlanStale(_)));
    assert_eq!(fs::read(path).unwrap(), user_bytes);
}

#[test]
fn replacing_a_target_with_identical_bytes_still_invalidates_the_plan_identity() {
    let repository = committed_repo();
    let install = plan_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();
    let token = install.plan_digest.clone();
    install_adapter(install, &token).unwrap();
    let plan = plan_uninstall_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();
    let token = plan.plan_digest.clone();
    let path = plan.config_path.clone();
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, &bytes).unwrap();

    let error = uninstall_adapter(plan, &token).expect_err("target identity changed");

    assert!(matches!(error, DevMapError::AdapterPlanStale(_)));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn replacing_the_adapter_parent_directory_invalidates_the_reviewed_plan() {
    let repository = committed_repo();
    let parent = repository.path().join(".codex");
    let displaced = repository.path().join(".codex-user-original");
    fs::create_dir(&parent).unwrap();
    let plan = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let token = plan.plan_digest.clone();
    fs::rename(&parent, &displaced).unwrap();
    fs::create_dir(&parent).unwrap();

    let error = install_adapter(plan, &token).expect_err("parent identity changed");

    assert!(matches!(error, DevMapError::AdapterPlanStale(_)));
    assert!(fs::read_dir(&parent).unwrap().next().is_none());
    assert!(fs::read_dir(&displaced).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn replacing_an_existing_config_preserves_its_unix_mode() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let repository = committed_repo();
    let path = repository.path().join(".codex/hooks.json");
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{}\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let plan = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let token = plan.plan_digest.clone();

    install_adapter(plan, &token).unwrap();

    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o640);
}

#[test]
fn generic_recognized_drift_is_a_modified_grade_d_report_but_bad_shape_is_malformed() {
    let repository = committed_repo();
    let path = repository.path().join(".devmap/mcp.json");
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "command": ["another", "mcp"],
            "transport": "stdio"
        }))
        .unwrap(),
    )
    .unwrap();

    let drift = verify_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();

    assert!(!drift.configured);
    assert_eq!(drift.modified, vec!["descriptor"]);
    assert_eq!(drift.capture_grade, CaptureGrade::D);
    fs::write(&path, b"{\"command\":false,\"transport\":\"stdio\"}").unwrap();
    assert!(matches!(
        verify_adapter(repository.path(), AdapterHost::GenericMcp),
        Err(DevMapError::MalformedAdapterConfig(_))
    ));
}

#[test]
fn generic_descriptor_does_not_claim_unobservable_host_registration() {
    let repository = committed_repo();
    let install = plan_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();
    let token = install.plan_digest.clone();
    install_adapter(install, &token).unwrap();

    let report = verify_adapter(repository.path(), AdapterHost::GenericMcp).unwrap();

    assert!(report.configured);
    assert!(!report.activation_verified);
    assert!(
        report
            .activation_reasons
            .iter()
            .any(|reason| reason.contains("registration"))
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_adapter_parent_is_refused_without_touching_its_destination() {
    use std::os::unix::fs::symlink;

    let repository = committed_repo();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), repository.path().join(".codex")).unwrap();

    let error = plan_adapter(repository.path(), AdapterHost::Codex)
        .expect_err("a symlinked component must fail closed");

    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}
