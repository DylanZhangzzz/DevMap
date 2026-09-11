use super::*;
mod cold_tests;
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
        fs::read_to_string(&stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
    if name.starts_with("cold_tests::") {
        print!("{}", fs::read_to_string(&stdout).unwrap());
    }
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

fn boundary_frozen_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn boundary_sql_bytes(common: &Path) -> Vec<Option<Vec<u8>>> {
    ["devmap.db", "devmap.db-wal"]
        .iter()
        .map(|name| {
            let path = common.join("devmap").join(name);
            match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("read owned SQL data file: {error}"),
            }
        })
        .collect()
}

#[test]
fn active_sql_same_key_query_uses_two_real_source_captures() {
    let Some(root) = isolated("active_sql_same_key_query_uses_two_real_source_captures") else {
        return;
    };
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, query) = fixture(&repo);
    let workspace = SourceGitInspector::open(&repo)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let event = crate::events::EventEnvelope::new(
        crate::events::EVENT_SCHEMA_VERSION,
        "boundary-event",
        crate::events::EventType::CaptureGap,
        1,
        "2026-09-08T10:00:00Z",
        crate::events::HostIdentity::new("test", "1").unwrap(),
        crate::events::ActorIdentity::new("actor", None).unwrap(),
        crate::events::SessionContext::new("boundary-session", None, "fixture", None, None, None)
            .unwrap(),
        serde_json::json!({"reason":"fixture"}),
    )
    .unwrap();
    crate::journal::JournalStore::open(&workspace, "boundary-session")
        .unwrap()
        .append(event)
        .unwrap();
    let frozen = root.join("frozen");
    crate::store::migration::ensure(&workspace, &frozen).unwrap();
    {
        let store = crate::store::RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap();
        assert!(crate::store::is_active(store.connection()).unwrap());
        let records: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM journal_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            records, 1,
            "fixture must exercise a nonempty active SQL store"
        );
    }
    let legacy = id
        .git_dir
        .join("devmap/sessions/boundary-session/events.ndjson");
    let legacy_before = fs::read(&legacy).unwrap();
    let mut harness = QueryHarness::new(id, query);
    let prior = harness.warm();
    assert!(prior.store_generation.is_some());
    assert_eq!(harness.state.sources.len(), 1);
    let sql_before = boundary_sql_bytes(&harness.id.common);
    assert!(sql_before[0].is_some());
    let frozen_before = boundary_frozen_files(&frozen);
    assert!(!frozen_before.is_empty());

    // Exclude cold acquisition and warm establishment; the real complete query
    // executes inside this counter interval, including both connection seals.
    QueryConfiguration::test_reset_source_capture_count();
    let (mut actual, starts) = harness.call();
    let captures = QueryConfiguration::test_source_capture_count();

    assert_eq!(
        starts, 0,
        "stable existing warm fixture must launch no Git children"
    );
    assert_eq!(boundary_sql_bytes(&harness.id.common), sql_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(boundary_frozen_files(&frozen), frozen_before);
    // generated_at is the per-query evaluation clock. Normalize ONLY that
    // documented volatile field; compare every other serialized snapshot field.
    let parse = |stamp: &str| {
        OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).unwrap()
    };
    assert!(parse(&actual.model.generated_at) >= parse(&prior.model.generated_at));
    actual.model.generated_at = prior.model.generated_at.clone();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&prior).unwrap()
    );
    assert_eq!(
        captures, 2,
        "same-key active SQL query must perform two whole SourceResolutionWitness captures, not four nested rechecks"
    );
}

// Tiny active-SQL setup shared by closing-boundary and application-failure tests.
// The existing initial capture-count test is deliberately unchanged.
fn boundary_active_fixture(root: &Path) -> QueryHarness {
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let (id, query) = fixture(&repo);
    let workspace = SourceGitInspector::open(&repo)
        .unwrap()
        .workspace_allow_unborn()
        .unwrap();
    let event = crate::events::EventEnvelope::new(
        crate::events::EVENT_SCHEMA_VERSION,
        "boundary-event",
        crate::events::EventType::CaptureGap,
        1,
        "2026-09-08T10:00:00Z",
        crate::events::HostIdentity::new("test", "1").unwrap(),
        crate::events::ActorIdentity::new("actor", None).unwrap(),
        crate::events::SessionContext::new("boundary-session", None, "fixture", None, None, None)
            .unwrap(),
        serde_json::json!({"reason":"fixture"}),
    )
    .unwrap();
    crate::journal::JournalStore::open(&workspace, "boundary-session")
        .unwrap()
        .append(event)
        .unwrap();
    crate::store::migration::ensure(&workspace, &root.join("frozen")).unwrap();
    {
        let store = crate::store::RepositoryStore::open_existing(&workspace)
            .unwrap()
            .unwrap();
        assert!(crate::store::is_active(store.connection()).unwrap());
    }
    QueryHarness::new(id, query)
}

fn boundary_call_result(harness: &mut QueryHarness) -> Result<ApplicationSnapshot, DevMapError> {
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
    })?;
    match result {
        crate::runtime::protocol::ApplicationResult::Snapshot { snapshot } => Ok(*snapshot),
        _ => panic!("query must produce a snapshot or error"),
    }
}

fn assert_boundary_model_matches(actual: &ApplicationSnapshot, expected: &ApplicationSnapshot) {
    let mut model = actual.model.clone();
    model.generated_at = expected.model.generated_at.clone();
    assert_eq!(
        model.workspace_facts.len(),
        expected.model.workspace_facts.len()
    );
    for (facts, reference) in model
        .workspace_facts
        .iter_mut()
        .zip(&expected.model.workspace_facts)
    {
        // A separate fresh application has its own collection clock; preserve
        // timestamp presence and compare every other provenance field exactly.
        assert_eq!(
            facts.git_observed_at.is_some(),
            reference.git_observed_at.is_some()
        );
        facts.git_observed_at = reference.git_observed_at.clone();
    }
    assert_eq!(model, expected.model);
    assert_eq!(actual.store_generation, expected.store_generation);
}

fn closing_boundary_drift(name: &str, ref_drift: bool) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let Some(root) = isolated(name) else {
        return;
    };
    let mut harness = boundary_active_fixture(&root);
    if ref_drift {
        git(
            &harness.id.source,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "new head",
            ],
        );
    }
    git(
        &harness.id.source,
        &[
            "config",
            "devmap.developmentTarget",
            if ref_drift { "alternate" } else { "dev" },
        ],
    );
    let prior = harness.warm();
    let mutation_path = harness.id.common.join(if ref_drift {
        "refs/heads/alternate"
    } else {
        "config"
    });
    assert!(
        fs::canonicalize(&mutation_path)
            .unwrap()
            .starts_with(fs::canonicalize(&root).unwrap())
    );
    let before = fs::read(&mutation_path).unwrap();
    let replacement = if ref_drift {
        fs::read(harness.id.common.join("refs/heads/main")).unwrap()
    } else {
        let mut bytes = before.clone();
        bytes.extend_from_slice(b"\n[devmap]\n developmentTarget = alternate\n");
        bytes
    };
    assert_ne!(
        replacement, before,
        "persistent drift must actually change evidence"
    );
    let sql_before = boundary_sql_bytes(&harness.id.common);
    let frozen_before = boundary_frozen_files(&root.join("frozen"));
    let legacy = harness
        .id
        .git_dir
        .join("devmap/sessions/boundary-session/events.ndjson");
    let legacy_before = fs::read(&legacy).unwrap();
    let entered = Arc::new(AtomicUsize::new(0));
    let marker = entered.clone();
    let mutated = mutation_path.clone();
    let expected_replacement = replacement.clone();
    harness.state.test_after_cached_projection = Some(Box::new(move || {
        marker.fetch_add(1, Ordering::SeqCst);
        fs::write(&mutated, &replacement).unwrap();
    }));
    let result = boundary_call_result(&mut harness);
    harness.state.test_after_cached_projection = None;
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "real post-projection hook must run exactly once"
    );
    assert_eq!(fs::read(&mutation_path).unwrap(), expected_replacement);
    harness.origin.validate().unwrap(); // Same Hello seal remains physically valid.
    assert_eq!(boundary_sql_bytes(&harness.id.common), sql_before);
    assert_eq!(boundary_frozen_files(&root.join("frozen")), frozen_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(
        result
            .expect_err("closing origin drift must not publish a query snapshot")
            .to_string(),
        "repository store: origin enumeration proof changed after observation"
    );

    // After the failed candidate, the same connection must recover with fresh
    // facts. Old context cannot survive merely because the normal TTL is warm.
    let expected = harness.direct();
    if ref_drift {
        assert_ne!(prior.model.topology, expected.model.topology);
    } else {
        assert_ne!(
            prior.model.development_target,
            expected.model.development_target
        );
    }
    let (recovered, starts) = harness.call();
    assert!(
        starts > 0,
        "closing failure must invalidate provisional cached context"
    );
    assert_eq!(
        recovered.model.development_target.as_ref().unwrap().name,
        "alternate"
    );
    assert_boundary_model_matches(&recovered, &expected);
    harness.origin.validate().unwrap();
    assert_eq!(boundary_sql_bytes(&harness.id.common), sql_before);
    assert_eq!(boundary_frozen_files(&root.join("frozen")), frozen_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    let (hot, starts) = harness.call();
    assert_eq!(starts, 0);
    assert_boundary_model_matches(&hot, &recovered);
}

#[test]
fn closing_boundary_config_drift_is_rejected_then_fresh_query_recovers() {
    closing_boundary_drift(
        "closing_boundary_config_drift_is_rejected_then_fresh_query_recovers",
        false,
    );
}

#[test]
fn closing_boundary_ref_drift_is_rejected_then_fresh_query_recovers() {
    closing_boundary_drift(
        "closing_boundary_ref_drift_is_rejected_then_fresh_query_recovers",
        true,
    );
}

#[test]
fn same_key_different_authenticated_baselines_use_original_reacquisition() {
    let Some(root) =
        isolated("same_key_different_authenticated_baselines_use_original_reacquisition")
    else {
        return;
    };
    let mut retained = boundary_active_fixture(&root);
    git(
        &retained.id.source,
        &["config", "devmap.developmentTarget", "dev"],
    );
    let old = retained.warm();
    git(
        &retained.id.source,
        &["config", "devmap.developmentTarget", "alternate"],
    );
    let key = (
        retained.id.source.clone(),
        retained.id.git_dir.clone(),
        retained.id.common.clone(),
    );
    let mut donor = QueryHarness::new(
        crate::runtime::identity(&retained.id.source).unwrap(),
        retained.query.clone(),
    );
    let expected = donor.warm();
    assert_ne!(
        old.model.development_target,
        expected.model.development_target
    );
    // Move a genuinely authenticated source proof from independent harness B.
    // Harness A retains its actual old application/origin proof; no fake epoch.
    let new_proof = donor
        .state
        .sources
        .remove(&key)
        .expect("same canonical source key");
    assert!(retained.state.sources.insert(key, new_proof).is_some());
    let sql_before = boundary_sql_bytes(&retained.id.common);
    let frozen_before = boundary_frozen_files(&root.join("frozen"));
    QueryConfiguration::test_reset_source_capture_count();
    let (actual, starts) = retained.call();
    let captures = QueryConfiguration::test_source_capture_count();
    assert!(
        captures > 2,
        "different retained baselines cannot use the two-capture grouped path"
    );
    assert!(
        starts > 0,
        "old origin proof requires actual original reacquisition"
    );
    assert_eq!(
        actual.model.development_target.as_ref().unwrap().name,
        "alternate"
    );
    assert_boundary_model_matches(&actual, &expected);
    assert_eq!(boundary_sql_bytes(&retained.id.common), sql_before);
    assert_eq!(boundary_frozen_files(&root.join("frozen")), frozen_before);
    let (hot, starts) = retained.call();
    assert_eq!(starts, 0);
    assert_boundary_model_matches(&hot, &actual);
}

mod boundary_failure_tests;
