use super::*;
mod profiling;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
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
fn isolated(name: &str) -> Option<PathBuf> {
    const CASE: &str = "DEVMAP_QUERY_VALIDATION_CASE";
    const ROOT: &str = "DEVMAP_QUERY_VALIDATION_ROOT";
    if std::env::var(CASE).as_deref() == Ok(name) {
        return Some(std::env::var_os(ROOT).unwrap().into());
    }
    let owned = tempfile::tempdir().unwrap();
    let home = owned.path().join("home");
    let xdg = owned.path().join("xdg");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&xdg).unwrap();
    let stdout = owned.path().join("stdout");
    let stderr = owned.path().join("stderr");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        &format!("runtime::query_validation::tests::{name}"),
        "--nocapture",
        "--test-threads=1",
    ]);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env(CASE, name)
        .env(ROOT, owned.path())
        .env("HOME", home)
        .env("USERPROFILE", owned.path().join("home"))
        .env("XDG_CONFIG_HOME", xdg)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "owned query test deadline");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{name}: {status}\n{}\n{}",
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
    None
}
fn git(root: &Path, args: &[&str]) {
    let out =
        crate::git_process::output(Command::new("git").arg("-C").arg(root).args(args)).unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct QueryHarness {
    id: Identity,
    query: ClientQuery,
    origin: QueryOrigin,
    state: QueryValidation,
    app: Option<RepositoryApplication>,
}
impl QueryHarness {
    fn new(id: Identity, query: ClientQuery) -> Self {
        let origin = QueryOrigin::for_connection(&id);
        Self {
            id,
            query,
            origin,
            state: QueryValidation::default(),
            app: None,
        }
    }
    fn call(&mut self) -> (ApplicationSnapshot, usize) {
        let bytes = serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query {
            query: self.query.clone(),
        })
        .unwrap();
        let before = crate::git_process::test_spawn_count();
        let result = with_query_origin(&bytes, Some(&self.origin), || {
            crate::runtime::executor::execute_with_queries(
                &mut self.app,
                &mut self.state,
                &self.id,
                &bytes,
            )
        })
        .unwrap();
        let crate::runtime::protocol::ApplicationResult::Snapshot { snapshot } = result else {
            panic!("expected Query snapshot")
        };
        (*snapshot, crate::git_process::test_spawn_count() - before)
    }
    fn max_age(&mut self, age: Duration) {
        self.app = Some(self.app.take().unwrap().with_git_max_age(age).unwrap());
    }
    fn warm(&mut self) -> ApplicationSnapshot {
        let (first, _) = self.call();
        // This deliberately controlled fixture checks cache routing, not latency.
        self.max_age(Duration::from_secs(60));
        let (hot, starts) = self.call();
        assert_eq!(starts, 0);
        assert_eq!(hot.git_cycle, first.git_cycle);
        hot
    }
    fn direct(&self) -> ApplicationSnapshot {
        let w = SourceGitInspector::open(&self.id.source)
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        RepositoryApplication::open(&w)
            .unwrap()
            .project(&w, &self.query, OffsetDateTime::now_utc())
            .unwrap()
    }
}

#[test]
fn discovered_missing_global_config_is_observed_then_hot() {
    let Some(root) = isolated("discovered_missing_global_config_is_observed_then_hot") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let out = crate::git_process::output(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["var", "GIT_CONFIG_GLOBAL"]),
    )
    .unwrap();
    assert!(out.status.success());
    let owned = fs::canonicalize(&root).unwrap();
    let candidate = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(PathBuf::from)
        .find(|p| {
            p.is_absolute()
                && !p.exists()
                && p.parent()
                    .and_then(|parent| fs::canonicalize(parent).ok())
                    .is_some_and(|parent| parent.starts_with(&owned))
        })
        .expect("Git must discover an absent candidate inside this isolated home");
    let mut harness = QueryHarness::new(id, q);
    harness.warm();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&candidate)
        .unwrap();
    fs::write(&candidate, b"[devmap]\n\tdevelopmentTarget = alternate\n").unwrap();
    let (changed, starts) = harness.call();
    assert!(starts > 0, "missing-file witness must invalidate");
    let expected = harness.direct();
    assert_eq!(
        changed.model.development_target,
        expected.model.development_target
    );
    assert_eq!(
        changed.model.development_target.as_ref().unwrap().name,
        "alternate"
    );
    assert_eq!(changed.model.lanes, expected.model.lanes);
    let (hot, starts) = harness.call();
    assert_eq!(starts, 0);
    assert_eq!(hot.model.lanes, changed.model.lanes);
}

#[test]
fn removed_objects_directory_is_rejected_by_same_sealed_hot_query() {
    let Some(root) = isolated("removed_objects_directory_is_rejected_by_same_sealed_hot_query")
    else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let objects = id.common.join("objects");
    let mut harness = QueryHarness::new(id, q);
    let prior = harness.warm();
    assert!(!harness.id.common.join("devmap/devmap.db").exists());
    let retained = root.join("retained-objects");
    fs::rename(&objects, &retained).unwrap();
    assert!(retained.is_dir());
    assert!(!objects.exists());

    let direct =
        SourceGitInspector::open(&repo).and_then(|inspector| inspector.workspace_allow_unborn());
    assert!(
        direct.is_err(),
        "fresh source discovery must refuse the invalid repository"
    );
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
        "same sealed hot query must refuse missing objects; original discovery error={:?}, cached cycle={}",
        direct.err(),
        prior.git_cycle
    );
}

#[test]
fn same_bytes_replaced_config_invalidates_identity_then_hot() {
    let Some(root) = isolated("same_bytes_replaced_config_invalidates_identity_then_hot") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let config = id.common.join("config");
    let bytes = fs::read(&config).unwrap();
    let mut harness = QueryHarness::new(id, q);
    let prior = harness.warm();
    fs::rename(&config, root.join("retained-config")).unwrap();
    fs::write(&config, &bytes).unwrap();
    let (changed, starts) = harness.call();
    assert!(starts > 0, "same bytes do not preserve file identity");
    assert_eq!(changed.model.lanes, prior.model.lanes);
    assert_eq!(changed.git_cycle, prior.git_cycle);
    let (hot, starts) = harness.call();
    assert_eq!(starts, 0);
    assert_eq!(hot.model.lanes, prior.model.lanes);
}

fn skeleton_change_control(name: &str) {
    let Some(root) = isolated(name) else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let mut harness = QueryHarness::new(id, q);
    let prior = harness.warm();
    let common = &harness.id.common;
    match name {
        "removed_refs_directory_invalidates_sealed_query" => {
            fs::rename(common.join("refs"), root.join("retained-refs")).unwrap();
        }
        "removed_head_invalidates_sealed_query" => {
            fs::rename(common.join("HEAD"), root.join("retained-head")).unwrap();
        }
        "malformed_head_invalidates_sealed_query" => {
            fs::write(common.join("HEAD"), b"invalid head\n").unwrap();
        }
        "malformed_current_loose_ref_invalidates_sealed_query" => {
            fs::write(common.join("refs/heads/main"), b"not-an-object-id\n").unwrap();
        }
        "replacement_objects_directory_invalidates_sealed_query" => {
            let path = common.join("objects");
            let before = crate::fs_security::checked_directory_identity(&path).unwrap();
            fs::rename(&path, root.join("retained-objects")).unwrap();
            fs::create_dir(&path).unwrap();
            assert_ne!(
                crate::fs_security::checked_directory_identity(&path).unwrap(),
                before
            );
        }
        "replacement_head_same_bytes_invalidates_sealed_query" => {
            let path = common.join("HEAD");
            let bytes = fs::read(&path).unwrap();
            let before =
                crate::fs_security::file_identity(&fs::File::open(&path).unwrap()).unwrap();
            fs::rename(&path, root.join("retained-head")).unwrap();
            fs::write(&path, bytes).unwrap();
            assert_ne!(
                crate::fs_security::file_identity(&fs::File::open(&path).unwrap()).unwrap(),
                before
            );
        }
        _ => panic!("unknown owned skeleton control"),
    }
    // Match actual fresh source discovery: in particular an empty replacement
    // objects directory need not make Git's repository discovery fail.
    let fresh =
        SourceGitInspector::open(&repo).and_then(|inspector| inspector.workspace_allow_unborn());
    let bytes = serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query {
        query: harness.query.clone(),
    })
    .unwrap();
    let before = crate::git_process::test_spawn_count();
    let result = with_query_origin(&bytes, Some(&harness.origin), || {
        crate::runtime::executor::execute_with_queries(
            &mut harness.app,
            &mut harness.state,
            &harness.id,
            &bytes,
        )
    });
    let starts = crate::git_process::test_spawn_count() - before;
    if let Err(error) = fresh {
        assert!(
            result.is_err(),
            "{name}: fresh discovery refused {error}, cached query must refuse"
        );
    } else {
        assert!(
            starts > 0,
            "{name}: changed skeleton must revalidate through fresh Git"
        );
        let crate::runtime::protocol::ApplicationResult::Snapshot { snapshot } = result.unwrap()
        else {
            panic!("{name}: expected snapshot on accepted fresh source");
        };
        assert_eq!(snapshot.git_cycle, prior.git_cycle);
        assert_eq!(snapshot.git_observed_at, prior.git_observed_at);
        assert_eq!(snapshot.model.lanes, prior.model.lanes);
    }
}

macro_rules! skeleton_controls {
    ($($name:ident),+ $(,)?) => {$ (
        #[test]
        fn $name() { skeleton_change_control(stringify!($name)); }
    )+};
}
skeleton_controls!(
    removed_refs_directory_invalidates_sealed_query,
    removed_head_invalidates_sealed_query,
    malformed_head_invalidates_sealed_query,
    malformed_current_loose_ref_invalidates_sealed_query,
    replacement_objects_directory_invalidates_sealed_query,
    replacement_head_same_bytes_invalidates_sealed_query,
);

#[test]
fn introduced_worktree_config_uses_original_per_client_fallback() {
    let Some(root) = isolated("introduced_worktree_config_uses_original_per_client_fallback")
    else {
        return;
    };
    let main = root.join("main");
    fs::create_dir(&main).unwrap();
    fixture(&main);
    let linked = root.join("linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic",
            linked.to_str().unwrap(),
        ],
    );
    let w = SourceGitInspector::open(&linked)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let id = crate::runtime::identity(&linked).unwrap();
    let q = crate::application::ClientView::new(w)
        .query_input()
        .unwrap();
    let mut harness = QueryHarness::new(id, q);
    harness.warm();
    git(&main, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &linked,
        &[
            "config",
            "--worktree",
            "devmap.developmentTarget",
            "alternate",
        ],
    );
    for _ in 0..2 {
        let (actual, starts) = harness.call();
        assert!(starts > 0, "worktreeConfig must keep the original Git path");
        let expected = harness.direct();
        assert_eq!(actual.model.lanes, expected.model.lanes);
        assert_eq!(
            actual.model.development_target,
            expected.model.development_target
        );
        assert_eq!(
            actual.model.development_target.as_ref().unwrap().name,
            "alternate"
        );
    }
}

#[test]
fn head_change_uses_observation_cycle_until_real_ttl_expiry() {
    let Some(root) = isolated("head_change_uses_observation_cycle_until_real_ttl_expiry") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let mut harness = QueryHarness::new(id, q);
    let first = harness.warm();
    let old_head = first
        .model
        .lanes
        .iter()
        .find(|r| r.is_current)
        .unwrap()
        .head
        .clone();
    git(
        &repo,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "after cache",
        ],
    );
    let new_head = SourceGitInspector::open(&repo)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap()
        .head;
    assert_ne!(new_head, old_head);
    let (cached, starts) = harness.call();
    // Changed ref bytes invalidate source discovery, while the independent
    // observation cycle still retains its facts until the configured TTL.
    assert!(starts > 0);
    assert_eq!(cached.git_cycle, first.git_cycle);
    assert_eq!(cached.git_observed_at, first.git_observed_at);
    assert_eq!(
        cached
            .model
            .lanes
            .iter()
            .find(|r| r.is_current)
            .unwrap()
            .head,
        old_head
    );
    let (unchanged, starts) = harness.call();
    assert_eq!(starts, 0, "unchanged source proof must be cached again");
    assert_eq!(unchanged.git_cycle, cached.git_cycle);
    assert_eq!(unchanged.git_observed_at, cached.git_observed_at);
    assert_eq!(unchanged.model.lanes, cached.model.lanes);
    // Use a short real expiry to exercise collection without a 2-second latency claim.
    harness.max_age(Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(2));
    let (fresh, starts) = harness.call();
    assert!(starts > 0);
    assert!(fresh.git_cycle > cached.git_cycle);
    assert_ne!(fresh.git_observed_at, cached.git_observed_at);
    assert_eq!(
        fresh
            .model
            .lanes
            .iter()
            .find(|r| r.is_current)
            .unwrap()
            .head,
        new_head
    );
    let expected = harness.direct();
    assert_eq!(fresh.model.lanes, expected.model.lanes);
}
fn fixture(root: &Path) -> (Identity, ClientQuery) {
    git(root, &["init", "-q", "-b", "main"]);
    git(
        root,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    );
    git(root, &["branch", "dev"]);
    git(root, &["branch", "alternate"]);
    let w = SourceGitInspector::open(root)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let id = Identity {
        source: fs::canonicalize(&w.root).unwrap(),
        git_dir: fs::canonicalize(&w.git_dir).unwrap(),
        common: fs::canonicalize(&w.git_common_dir).unwrap(),
        repository: "unused-query-fixture".into(),
    };
    let query = crate::application::ClientView::new(w)
        .query_input()
        .unwrap();
    (id, query)
}
#[test]
fn plain_hot_query_starts_no_git_processes() {
    let Some(root) = isolated("plain_hot_query_starts_no_git_processes") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let mut state = QueryValidation::default();
    let mut app = None;
    // Same persistent dispatcher used by Executor::start, not just the proof helper.
    let bytes = serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query {
        query: q.clone(),
    })
    .unwrap();
    let hello_origin = QueryOrigin::capture(&id).unwrap();
    let dispatch = |app: &mut Option<RepositoryApplication>, state: &mut QueryValidation| {
        match with_query_origin(&bytes, Some(&hello_origin), || {
            crate::runtime::executor::execute_with_queries(app, state, &id, &bytes)
        })
        .unwrap()
        {
            crate::runtime::protocol::ApplicationResult::Snapshot { snapshot } => *snapshot,
            _ => panic!("query must produce a snapshot"),
        }
    };
    let first = dispatch(&mut app, &mut state);
    // Force a hot fixture despite slow unoptimized validation. Production keeps
    // its two-second TTL; this command-count contract is not latency acceptance.
    app = Some(
        app.take()
            .unwrap()
            .with_git_max_age(Duration::from_secs(60))
            .unwrap(),
    );
    let before = crate::git_process::test_spawn_count();
    let next = dispatch(&mut app, &mut state);
    let calls = crate::git_process::test_spawn_count() - before;
    assert_eq!(
        next.git_cycle, first.git_cycle,
        "fixture must actually exercise a hot observation cycle"
    );
    assert_eq!(next.model.lanes, first.model.lanes);
    assert_eq!(
        calls, 0,
        "plain hot query must avoid actual Git child launches across all worker threads"
    );
}
#[test]
fn hello_origin_replacement_before_first_query_is_rejected() {
    let Some(root) = isolated("hello_origin_replacement_before_first_query_is_rejected") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let hello_origin = QueryOrigin::capture(&id).unwrap();
    fs::rename(&repo, root.join("retained-hello-origin")).unwrap();
    fs::create_dir(&repo).unwrap();
    fixture(&repo);
    let bytes =
        serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query { query: q })
            .unwrap();
    let mut app = None;
    let mut queries = QueryValidation::default();
    let before = crate::git_process::test_spawn_count();
    let result = with_query_origin(&bytes, Some(&hello_origin), || {
        crate::runtime::executor::execute_with_queries(&mut app, &mut queries, &id, &bytes)
    });
    assert!(result.is_err());
    assert!(app.is_none());
    assert_eq!(
        crate::git_process::test_spawn_count(),
        before,
        "replaced Hello source must be refused before invoking Git at occupant"
    );
}

#[test]
fn failed_hello_seal_cannot_rebind_after_backlink_repair() {
    let Some(root) = isolated("failed_hello_seal_cannot_rebind_after_backlink_repair") else {
        return;
    };
    let main = root.join("main");
    fs::create_dir(&main).unwrap();
    fixture(&main);
    let linked = root.join("linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic",
            linked.to_str().unwrap(),
        ],
    );
    let w = SourceGitInspector::open(&linked)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let q = crate::application::ClientView::new(w.clone())
        .query_input()
        .unwrap();
    let pointer = fs::read(linked.join(".git")).unwrap();
    let backlink = w.git_dir.join("gitdir");
    let original = fs::read(&backlink).unwrap();
    fs::write(
        &backlink,
        format!("{}\n", root.join("absent/.git").display()),
    )
    .unwrap();
    // The same Git identity discovery used by Hello still succeeds, while the
    // physical reciprocal proof actually fails. No fabricated failure object.
    let id = crate::runtime::identity(&linked).unwrap();
    assert!(QueryOrigin::capture(&id).is_err());
    let connection = QueryOrigin::for_connection(&id);
    fs::rename(&linked, root.join("retained-old-root")).unwrap();
    fs::create_dir(&linked).unwrap();
    fs::write(linked.join(".git"), pointer).unwrap();
    fs::write(&backlink, original).unwrap();
    assert!(
        QueryOrigin::capture(&id).is_ok(),
        "replacement is now structurally valid but belongs to a new root"
    );
    let bytes =
        serde_json::to_vec(&crate::runtime::protocol::ApplicationRequest::Query { query: q })
            .unwrap();
    let mut app = None;
    let mut queries = QueryValidation::default();
    let result = with_query_origin(&bytes, Some(&connection), || {
        crate::runtime::executor::execute_with_queries(&mut app, &mut queries, &id, &bytes)
    });
    assert!(
        result.is_err(),
        "failed Hello seal must not authorize a repaired replacement occupant"
    );
    assert!(
        app.is_none(),
        "rejected connection must not create an application for the occupant"
    );
}
#[test]
fn changed_and_unsupported_config_match_direct_projection() {
    let Some(root) = isolated("changed_and_unsupported_config_match_direct_projection") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let mut state = QueryValidation::default();
    let mut app = None;
    state
        .project(&mut app, &id, &q, OffsetDateTime::now_utc())
        .unwrap();
    git(&repo, &["config", "devmap.developmentTarget", "alternate"]);
    for complex in [false, true] {
        if complex {
            git(&repo, &["config", "include.path", "missing-extra-config"]);
        }
        let actual = state
            .project(&mut app, &id, &q, OffsetDateTime::now_utc())
            .unwrap();
        let w = SourceGitInspector::open(&repo)
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let expected = RepositoryApplication::open(&w)
            .unwrap()
            .project(&w, &q, OffsetDateTime::now_utc())
            .unwrap();
        assert_eq!(
            actual.model.development_target,
            expected.model.development_target
        );
        assert_eq!(
            actual.model.development_target.as_ref().unwrap().name,
            "alternate"
        );
        assert_eq!(actual.model.lanes, expected.model.lanes);
    }
}
#[test]
fn recreated_source_does_not_inherit_retained_identity() {
    let Some(root) = isolated("recreated_source_does_not_inherit_retained_identity") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, q) = fixture(&repo);
    let mut state = QueryValidation::default();
    let mut app = None;
    state
        .project(&mut app, &id, &q, OffsetDateTime::now_utc())
        .unwrap();
    // Exact owned sibling move retains the old repository; no recursive delete.
    fs::rename(&repo, root.join("retained-original")).unwrap();
    fs::create_dir(&repo).unwrap();
    fixture(&repo);
    assert!(
        state
            .project(&mut app, &id, &q, OffsetDateTime::now_utc())
            .is_err(),
        "same pathname cannot inherit physical repository identity"
    );
}
