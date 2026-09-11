use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn failed_body_discards_candidate(root: &Path, unwind: bool) {
    let mut harness = boundary_active_fixture(root);
    let prior = harness.warm();
    assert!(prior.store_generation.is_some());
    assert!(!harness.app.as_ref().unwrap().test_query_caches_discarded());
    let sql_before = boundary_sql_bytes(&harness.id.common);
    let frozen_before = boundary_frozen_files(&root.join("frozen"));
    let legacy = harness
        .id
        .git_dir
        .join("devmap/sessions/boundary-session/events.ndjson");
    let legacy_before = fs::read(&legacy).unwrap();
    let reached = Arc::new(AtomicUsize::new(0));
    let hook_reached = Arc::clone(&reached);
    harness.app.as_mut().unwrap().test_after_storage = Some(Box::new(move || {
        hook_reached.fetch_add(1, Ordering::SeqCst);
        if unwind {
            panic!("owned after-storage unwind");
        }
        Err(DevMapError::Store("owned after-storage failure".into()))
    }));

    // This seam runs only after the real pinned SQL/origin/full inventory read.
    // Catch unwind at the request caller, so the same application is inspected.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        boundary_call_result(&mut harness)
    }));
    if unwind {
        let payload = outcome.expect_err("injected unwind must escape the query");
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"owned after-storage unwind")
        );
    } else {
        let result = outcome.expect("ordinary errors must not become panics");
        assert!(
            matches!(result, Err(DevMapError::Store(ref reason)) if reason == "owned after-storage failure")
        );
    }
    assert_eq!(reached.load(Ordering::SeqCst), 1);
    harness.origin.validate().unwrap();
    assert_eq!(boundary_sql_bytes(&harness.id.common), sql_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(boundary_frozen_files(&root.join("frozen")), frozen_before);
    let discarded = harness.app.as_ref().unwrap().test_query_caches_discarded();

    // Exercise recovery even on the pre-fix implementation, then assert the
    // failure cleanup observed above. No failed candidate is reused as output.
    let (mut recovered, _) = harness.call();
    let expected = harness.direct();
    assert_eq!(
        recovered.model.development_target,
        expected.model.development_target
    );
    assert_eq!(recovered.model.lanes, expected.model.lanes);
    assert_eq!(recovered.store_generation, prior.store_generation);
    assert!(recovered.git_cycle >= prior.git_cycle);
    assert_eq!(
        recovered.model.workspace_facts.len(),
        prior.model.workspace_facts.len()
    );
    // Re-acquisition may advance these observation clocks and collection cycle.
    // Compare every remaining serialized field, including origin qualification.
    recovered.model.generated_at = prior.model.generated_at.clone();
    recovered.git_observed_at = prior.git_observed_at.clone();
    recovered.store_inputs_observed_at = prior.store_inputs_observed_at.clone();
    recovered.git_cycle = prior.git_cycle;
    for (actual, before) in recovered
        .model
        .workspace_facts
        .iter_mut()
        .zip(&prior.model.workspace_facts)
    {
        actual.git_observed_at = before.git_observed_at.clone();
    }
    assert_eq!(
        serde_json::to_value(&recovered).unwrap(),
        serde_json::to_value(&prior).unwrap()
    );
    assert_eq!(boundary_sql_bytes(&harness.id.common), sql_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(boundary_frozen_files(&root.join("frozen")), frozen_before);
    assert!(
        discarded,
        "failed query must discard all tentative storage and projection caches before the next request"
    );
}

#[test]
fn after_storage_error_discards_all_query_caches() {
    let Some(root) =
        isolated("boundary_failure_tests::after_storage_error_discards_all_query_caches")
    else {
        return;
    };
    failed_body_discards_candidate(&root, false);
}

#[test]
fn after_storage_unwind_discards_all_query_caches() {
    let Some(root) =
        isolated("boundary_failure_tests::after_storage_unwind_discards_all_query_caches")
    else {
        return;
    };
    failed_body_discards_candidate(&root, true);
}
