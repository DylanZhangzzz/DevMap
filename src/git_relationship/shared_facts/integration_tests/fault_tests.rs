use super::*;
use crate::git_relationship::{SharedFactsTestFault, resolve_with_shared_facts_fault_test};

fn fixture(
    owned: &Path,
) -> (
    crate::git::SourceWorkspace,
    Vec<crate::worktrees::WorktreeDescriptor>,
) {
    let main = owned.join("main");
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    commit(&main, "fault fixture base");
    git(&main, &["branch", "dev"]);
    for name in ["a", "b"] {
        let root = owned.join(name);
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                root.to_str().unwrap(),
                "HEAD",
            ],
        );
    }
    fs::write(owned.join("a/dirty.txt"), "independent status\n").unwrap();
    let workspace = SourceGitInspector::open(&main)
        .unwrap()
        .workspace()
        .unwrap();
    let rows = WorktreeScanner::scan(&workspace).unwrap();
    assert_eq!(rows.len(), 3);
    (workspace, rows)
}

#[test]
fn ordinary_representative_error_uses_complete_original_report() {
    let Some(owned) =
        isolated_case("fault_tests::ordinary_representative_error_uses_complete_original_report")
    else {
        return;
    };
    let (workspace, rows) = fixture(&owned);
    let (expected, _) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Original).unwrap();
    let (actual, stats) = resolve_with_shared_facts_fault_test(
        &workspace,
        &rows,
        SharedFactsTestFault::OrdinaryRepresentativeFailure,
    )
    .unwrap();
    assert_eq!(encoded(&actual), encoded(&expected));
    assert_eq!(
        stats.shared_rows, 0,
        "failed representative cannot supply shared facts"
    );
    assert_eq!(stats.shared_groups, 0);
    assert_eq!(stats.original_rows, rows.len());
}

#[test]
fn tag_change_discards_computed_facts_and_observes_fresh_original() {
    let Some(owned) = isolated_case(
        "fault_tests::tag_change_discards_computed_facts_and_observes_fresh_original",
    ) else {
        return;
    };
    let (workspace, rows) = fixture(&owned);
    let (before, _) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Original).unwrap();
    let (actual, stats) = resolve_with_shared_facts_fault_test(
        &workspace,
        &rows,
        SharedFactsTestFault::TagBeforeRecheck,
    )
    .unwrap();
    let (expected, _) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Original).unwrap();
    assert!(
        encoded(&before) != encoded(&expected),
        "owned tag mutation must be visible in fresh original facts"
    );
    assert_eq!(encoded(&actual), encoded(&expected));
    assert_eq!(stats.shared_rows, 0);
    assert_eq!(stats.shared_groups, 0);
    assert_eq!(stats.original_rows, rows.len());
    assert!(
        actual
            .by_worktree_id
            .values()
            .filter_map(|r| r.fork_point.as_ref())
            .all(|f| f.tags.iter().any(|t| t == "changed-before-shared-recheck"))
    );
}

#[test]
fn representative_real_deadline_is_fatal_not_original_fallback() {
    let Some(owned) =
        isolated_case("fault_tests::representative_real_deadline_is_fatal_not_original_fallback")
    else {
        return;
    };
    let (workspace, rows) = fixture(&owned);
    let result = resolve_with_shared_facts_fault_test(
        &workspace,
        &rows,
        SharedFactsTestFault::RepresentativeDeadline,
    );
    assert!(
        matches!(
            result,
            Err(crate::error::DevMapError::GitProcess(
                crate::git_process::GitProcessError::Deadline
            ))
        ),
        "real expired representative budget must remain fatal"
    );
    // A failed operation must not contaminate the next invocation or its budget.
    let (expected, _) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Original).unwrap();
    let (fresh, stats) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Shared).unwrap();
    assert_eq!(encoded(&fresh), encoded(&expected));
    assert_shared(&stats);
}
