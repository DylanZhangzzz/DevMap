use super::*;

#[test]
fn cold_main_linked_unborn_use_one_authoritative_inspection() {
    let Some(owned) =
        isolated("cold_tests::cold_main_linked_unborn_use_one_authoritative_inspection")
    else {
        return;
    };
    for layout in ["main", "linked", "unborn"] {
        let root = owned.join(layout);
        fs::create_dir(&root).unwrap();
        let (id, query) = if layout == "unborn" {
            git(&root, &["init", "-q", "-b", "main"]);
            let w = SourceGitInspector::open(&root)
                .unwrap()
                .workspace_allow_unborn()
                .unwrap();
            (
                crate::runtime::identity(&root).unwrap(),
                crate::application::ClientView::new(w)
                    .query_input()
                    .unwrap(),
            )
        } else {
            let (id, query) = fixture(&root);
            if layout == "linked" {
                let linked = owned.join("linked-checkout");
                git(
                    &root,
                    &["worktree", "add", "-qb", "side", linked.to_str().unwrap()],
                );
                let w = SourceGitInspector::open(&linked)
                    .unwrap()
                    .workspace_allow_unborn()
                    .unwrap();
                (
                    crate::runtime::identity(&linked).unwrap(),
                    crate::application::ClientView::new(w)
                        .query_input()
                        .unwrap(),
                )
            } else {
                (id, query)
            }
        };
        let before = crate::git_process::test_spawn_count();
        SourceGitInspector::open(&id.source)
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let once = crate::git_process::test_spawn_count() - before;
        assert!(once > 0);
        let mut harness = QueryHarness::new(id, query);
        let (snapshot, starts) = harness.call();
        println!(
            "layout={layout} cold_git_starts={starts} inspection_counts={:?} one_inspector={once}",
            harness.state.test_inspections
        );
        assert_eq!(
            harness.state.test_inspections,
            vec![once],
            "cold path repeated authoritative inspection"
        );
        let direct = harness.direct();
        assert_eq!(snapshot.model.lanes, direct.model.lanes);
        assert_eq!(snapshot.model.counts, direct.model.counts);
        assert_eq!(
            snapshot.model.development_target,
            direct.model.development_target
        );
        harness.max_age(Duration::from_secs(60));
        let (hot, count) = harness.call();
        assert_eq!(count, 0);
        assert_eq!(hot.git_cycle, snapshot.git_cycle);
        assert_eq!(hot.git_observed_at, snapshot.git_observed_at);
    }
}

#[test]
fn candidate_objects_loss_is_seen_by_authoritative_inspector() {
    let Some(owned) =
        isolated("cold_tests::candidate_objects_loss_is_seen_by_authoritative_inspector")
    else {
        return;
    };
    let root = owned.join("repo");
    fs::create_dir(&root).unwrap();
    let (id, query) = fixture(&root);
    let mut harness = QueryHarness::new(id, query);
    let objects = harness.id.common.join("objects");
    let retained = owned.join("objects-retained");
    harness.state.test_cold_hook = Some(Box::new(move |stage| {
        if stage == ColdStage::CandidateCaptured {
            fs::rename(&objects, &retained).unwrap();
        }
    }));
    let bytes = serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query {
        query: harness.query.clone(),
    })
    .unwrap();
    let result = with_query_origin(&bytes, Some(&harness.origin), || {
        crate::runtime::executor::execute_with_queries(
            &mut harness.app,
            &mut harness.state,
            &harness.id,
            &bytes,
        )
    });
    assert!(
        result.is_err(),
        "candidate capture cannot replace authoritative discovery"
    );
    assert!(
        harness.app.is_none(),
        "failed authoritative inspection must not construct application"
    );
    assert!(harness.state.sources.is_empty());
}

#[test]
fn post_inspector_config_drift_discards_candidate_proof() {
    let Some(owned) = isolated("cold_tests::post_inspector_config_drift_discards_candidate_proof")
    else {
        return;
    };
    let root = owned.join("repo");
    fs::create_dir(&root).unwrap();
    let (id, query) = fixture(&root);
    let mut harness = QueryHarness::new(id, query);
    let config = harness.id.common.join("config");
    harness.state.test_cold_hook = Some(Box::new(move |stage| {
        if stage == ColdStage::AuthoritativeInspected {
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(&config)
                .unwrap()
                .write_all(b"\n[devmap]\n developmentTarget = alternate\n")
                .unwrap();
        }
    }));
    let (snapshot, _) = harness.call();
    assert_eq!(
        snapshot.model.development_target.as_ref().unwrap().name,
        "alternate"
    );
    assert!(
        harness.state.sources.is_empty(),
        "drifted candidate proof must not be cached"
    );
    assert_eq!(snapshot.model.lanes, harness.direct().model.lanes);
}
