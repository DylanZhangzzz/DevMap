//! Explicitly selected, read-only diagnostic; never a performance acceptance gate.
use super::*;
use sha2::{Digest, Sha256};

const RECEIPT: &str = "C:/Users/user/.devmap-test-fixtures/devmap-sqlite-compatible-20260909/schema2-e063c6bfb9bc0a9400251e092cd78bb1/benchmark-receipt.json";
const SHA: &str = "ae37686231757040dcbe1e274f40e79e30068cb489b87ed839e0c3742e6ebcaa";
const SCALE_RECEIPT: &str = "C:/Users/user/.devmap-test-fixtures/devmap-sqlite-compatible-20260909/schema2-4e10e571ab7fa425789f127100258231/benchmark-receipt.json";
const SCALE_SHA: &str = "d968690717e51b531deaeb1f095b9c615c1474543ba32c6c9bbe789756cd4620";

fn measured<T>(stage: &str, iteration: usize, f: impl FnOnce() -> T) -> T {
    let before = crate::git_process::test_spawn_count();
    // Wall-clock boundaries let the optional Git Trace2 diagnostic select this
    // observation. Monotonic elapsed time remains the duration measurement.
    let started_unix_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let start = Instant::now();
    let result = f();
    let wall_us = start.elapsed().as_micros();
    let finished_unix_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    println!(
        "{}",
        serde_json::json!({"diagnostic":"query-stage/1", "stage":stage,
        "iteration":iteration,"wall_us":wall_us,
        "started_unix_us":started_unix_us,"finished_unix_us":finished_unix_us,
        "git_starts":crate::git_process::test_spawn_count()-before})
    );
    result
}

fn checked_owned(path: &str, allocation: &Path, expected: &serde_json::Value) -> PathBuf {
    let path = crate::fs_security::checked_canonical_directory(Path::new(path)).unwrap();
    assert!(path.starts_with(allocation));
    let actual = crate::fs_security::checked_directory_identity(&path).unwrap();
    assert_eq!(actual.first.to_string(), expected["dev"].as_str().unwrap());
    assert_eq!(actual.second.to_string(), expected["ino"].as_str().unwrap());
    path
}

#[test]
#[ignore = "exact owned schema2 receipt allowlist; root must wrap preservation checks"]
fn owned_schema2_query_stage_profile() {
    let (receipt_path, receipt_sha, nonce, dimensions) =
        match std::env::var("DEVMAP_QUERY_PROFILE_FIXTURE") {
            Err(std::env::VarError::NotPresent) => {
                (RECEIPT, SHA, "e063c6bfb9bc0a9400251e092cd78bb1", (2, 2, 6))
            }
            Ok(value) if value == "tiny" => {
                (RECEIPT, SHA, "e063c6bfb9bc0a9400251e092cd78bb1", (2, 2, 6))
            }
            Ok(value) if value == "scale" => (
                SCALE_RECEIPT,
                SCALE_SHA,
                "4e10e571ab7fa425789f127100258231",
                (20, 100, 100000),
            ),
            other => panic!("unsupported explicit profiler fixture selector: {other:?}"),
        };
    let bytes = crate::fs_security::checked_file(Path::new(receipt_path), false, false)
        .and_then(|mut file| {
            use std::io::Read;
            assert!(file.metadata()?.len() <= 1024 * 1024);
            let mut bytes = Vec::new();
            (&mut file).take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            assert!(bytes.len() <= 1024 * 1024);
            Ok(bytes)
        })
        .unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), receipt_sha);
    let receipt: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(receipt["schema"], "devmap/benchmark-fixture/1");
    assert_eq!(receipt["schema_version"], 2);
    assert_eq!(receipt["exclusive_creation"], true);
    assert_eq!(receipt["nonce"], nonce);
    assert_eq!(receipt["dimensions"]["worktrees"], dimensions.0);
    assert_eq!(receipt["dimensions"]["sessions"], dimensions.1);
    assert_eq!(receipt["dimensions"]["events"], dimensions.2);
    println!("profile_fixture_nonce={nonce}");
    let allocation = crate::fs_security::checked_canonical_directory(Path::new(
        receipt["allocation_root"].as_str().unwrap(),
    ))
    .unwrap();
    checked_owned(
        receipt["allocation_root"].as_str().unwrap(),
        &allocation,
        &receipt["allocation_identity"],
    );
    let owned: Vec<_> = receipt["owned_directories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            checked_owned(
                entry["path"].as_str().unwrap(),
                &allocation,
                &entry["identity"],
            )
        })
        .collect();
    let source = crate::fs_security::checked_canonical_directory(Path::new(
        receipt["source"].as_str().unwrap(),
    ))
    .unwrap();
    let common = crate::fs_security::checked_canonical_directory(Path::new(
        receipt["common"].as_str().unwrap(),
    ))
    .unwrap();
    assert!(owned.contains(&source) && owned.contains(&common));
    let id = crate::runtime::identity(&source).unwrap();
    assert_eq!(id.common, common);
    assert!(owned.contains(&id.git_dir));
    let workspace = SourceGitInspector::open(&source)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    assert_eq!(
        crate::worktrees::repository_id(&workspace),
        receipt["repository_id"].as_str().unwrap()
    );
    let store = crate::store::RepositoryStore::open_existing(&workspace)
        .unwrap()
        .unwrap();
    assert!(crate::store::is_active(store.connection()).unwrap());
    let version: i64 = store
        .connection()
        .query_row(
            "SELECT schema_version FROM store_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 2);
    match std::env::var("DEVMAP_QUERY_PROFILE_MODE") {
        Ok(mode) if mode == "record_parse_batches" => {
            crate::store::snapshot::profile_sql_inputs(
                store.connection(),
                &workspace,
                dimensions.1 as usize,
                dimensions.2 as u64,
                |_, _, _| {},
            )
            .unwrap();
            crate::journal::parse_profile::run(store.connection(), dimensions.2 as u64).unwrap();
            return;
        }
        Ok(mode) if mode == "sql_inputs" => {
            crate::store::snapshot::profile_sql_inputs(
                store.connection(),
                &workspace,
                dimensions.1 as usize,
                dimensions.2 as u64,
                |stage, iteration, wall_us| {
                    println!(
                        "{}",
                        serde_json::json!({"diagnostic":"sql-input-stages/1",
                            "stage":stage,"iteration":iteration,"wall_us":wall_us})
                    );
                },
            )
            .unwrap();
            return;
        }
        Ok(mode) if mode == "inventory_workers" => {
            crate::store::migration::profile_inventory_workers(&workspace, store.connection())
                .unwrap();
            return;
        }
        Err(std::env::VarError::NotPresent) => {}
        other => panic!("unsupported explicit profiler mode: {other:?}"),
    }
    drop(store);
    let query = crate::application::ClientView::new(workspace.clone())
        .query_input()
        .unwrap();
    let mut harness = QueryHarness::new(id, query);
    let (first, _) = measured("sealed_query_initial", 0, || harness.call());
    assert_eq!(
        first.model.current_worktree_id,
        receipt["current_worktree_id"].as_str().unwrap()
    );
    // A controlled hot window isolates non-Git cost. This is NOT the production
    // two-second TTL, transport/concurrency timing, or an acceptance measurement.
    harness.max_age(Duration::from_secs(60));
    let mut reader = crate::store::snapshot::InputReader::new();
    for iteration in 0..5 {
        measured("connection_origin_validate", iteration, || {
            harness.origin.validate().unwrap()
        });
        let proof = harness
            .state
            .sources
            .values()
            .next()
            .expect("plain fixture must retain proof");
        measured("source_workspace_validate", iteration, || {
            proof.workspace().unwrap()
        });
        assert!(measured("configuration_recheck", iteration, || proof
            .configuration
            .recheck()
            .unwrap()));
        let (generation, _) = measured("independent_storage_read", iteration, || {
            reader.read(&workspace).unwrap()
        });
        assert!(generation.is_some());
        let diagnostic_store = measured("sql_open_existing_independent", iteration, || {
            crate::store::RepositoryStore::open_existing(&workspace)
                .unwrap()
                .unwrap()
        });
        crate::store::migration::profile_frozen_read_stages(
            &workspace,
            diagnostic_store.connection(),
            iteration,
            |stage, wall_us, git_starts| {
                println!(
                    "{}",
                    serde_json::json!({
                        "diagnostic":"query-stage/1", "stage":stage, "iteration":iteration,
                        "wall_us":wall_us, "git_starts":git_starts
                    })
                )
            },
        )
        .unwrap();
        drop(diagnostic_store);
        let projected = measured("application_verified_aggregate", iteration, || {
            harness
                .app
                .as_mut()
                .unwrap()
                .project_verified_query(proof, &harness.query, OffsetDateTime::now_utc())
                .unwrap()
        });
        assert_eq!(projected.git_cycle, first.git_cycle);
        let (snapshot, starts) = measured("sealed_query_hot", iteration, || harness.call());
        // Record the real count, including conservative proof fallbacks;
        // this diagnostic is not a zero-Git oracle.
        println!("active_sql_hot_git_starts={starts}");
        assert_eq!(snapshot.git_cycle, first.git_cycle);
        assert_eq!(snapshot.git_observed_at, first.git_observed_at);
        assert_eq!(snapshot.model.lanes, first.model.lanes);
        assert_eq!(snapshot.model.counts, first.model.counts);
        assert_eq!(
            snapshot.model.development_target,
            first.model.development_target
        );
    }
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(receipt_path).unwrap())),
        receipt_sha
    );
}
