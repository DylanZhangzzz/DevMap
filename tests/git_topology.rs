mod support;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use devmap::error::DevMapError;
use devmap::git::{SourceGitInspector, SourceWorkspace};
use devmap::git_topology::{GitTopologyCollector, TopologyGraph};
use devmap::worktrees::WorktreeScanner;

fn commit_file(repo: &Path, name: &str, contents: &str, subject: &str) -> String {
    fs::write(repo.join(name), contents).expect("write fixture file");
    support::git(repo, ["add", "--", name]);
    support::git(repo, ["commit", "-m", subject]);
    support::git(repo, ["rev-parse", "HEAD"])
}

fn scan_read_only(repo: &Path) -> TopologyGraph {
    let workspace = SourceGitInspector::open(repo)
        .expect("open source repository")
        .workspace()
        .expect("inspect source workspace");
    let worktrees = WorktreeScanner::scan(&workspace).expect("scan worktrees");
    let before = support::source_snapshot(repo);
    let graph = GitTopologyCollector::scan(&workspace, &worktrees).expect("scan topology");
    let after = support::source_snapshot(repo);
    assert_eq!(after, before, "topology scan mutated the source repository");
    graph
}

fn assert_edges_are_git_parents(repo: &Path, graph: &TopologyGraph) {
    let parent_rows = support::git(repo, ["rev-list", "--all", "--parents"]);
    let parents_by_child = parent_rows
        .lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            let child = fields.next().expect("rev-list child OID");
            (child, fields.collect::<Vec<_>>())
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for edge in &graph.edges {
        assert!(
            parents_by_child
                .get(edge.to_oid.as_str())
                .expect("edge child is reachable from a fixture ref")
                .contains(&edge.from_oid.as_str()),
            "{} is not a Git parent of {}",
            edge.from_oid,
            edge.to_oid
        );
    }
}

#[test]
fn branch_without_worktree_remains_in_topology() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["branch", "feature/no-worktree"]);

    let graph = scan_read_only(repo.path());

    assert!(
        graph
            .refs
            .iter()
            .any(|reference| reference.ref_name == "refs/heads/feature/no-worktree")
    );
    assert_edges_are_git_parents(repo.path(), &graph);
}

#[test]
fn feature_of_feature_and_merge_preserve_real_parent_edges() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["switch", "-c", "feature/one"]);
    let feature_one = commit_file(repo.path(), "one.txt", "one\n", "feature one");
    support::git(repo.path(), ["switch", "-c", "feature/two"]);
    let feature_two = commit_file(repo.path(), "two.txt", "two\n", "feature two");
    support::git(repo.path(), ["switch", "main"]);
    let main_commit = commit_file(repo.path(), "main.txt", "main\n", "main advance");
    support::git(
        repo.path(),
        ["merge", "--no-ff", "feature/two", "-m", "merge feature two"],
    );
    let merge = support::git(repo.path(), ["rev-parse", "HEAD"]);

    let graph = scan_read_only(repo.path());

    let merge_node = graph
        .commits
        .iter()
        .find(|commit| commit.oid == merge)
        .expect("merge commit is retained");
    assert_eq!(merge_node.parents, vec![main_commit, feature_two.clone()]);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| { edge.from_oid == feature_one && edge.to_oid == feature_two })
    );
    let feature_one_node = graph
        .commits
        .iter()
        .find(|commit| commit.oid == feature_one)
        .expect("feature commit is retained");
    assert_eq!(feature_one_node.subject.as_deref(), Some("feature one"));
    assert!(feature_one_node.authored_at.is_some());
    assert_edges_are_git_parents(repo.path(), &graph);
}

#[test]
fn deleting_a_branch_after_merge_preserves_its_commit_parent_history() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["switch", "-c", "feature/delete-after-merge"]);
    let feature_tip = commit_file(
        repo.path(),
        "deleted-branch.txt",
        "retained\n",
        "work from deleted branch",
    );
    support::git(repo.path(), ["switch", "main"]);
    let main_parent = commit_file(repo.path(), "main.txt", "main\n", "advance main");
    support::git(
        repo.path(),
        [
            "merge",
            "--no-ff",
            "feature/delete-after-merge",
            "-m",
            "merge then delete branch",
        ],
    );
    let merge = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["branch", "-D", "feature/delete-after-merge"]);

    let graph = scan_read_only(repo.path());

    assert!(
        !graph
            .refs
            .iter()
            .any(|reference| reference.ref_name == "refs/heads/feature/delete-after-merge")
    );
    assert_eq!(
        graph
            .commits
            .iter()
            .find(|commit| commit.oid == merge)
            .expect("merge commit remains reachable after branch deletion")
            .parents,
        vec![main_parent, feature_tip.clone()]
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from_oid == feature_tip && edge.to_oid == merge)
    );
    assert_edges_are_git_parents(repo.path(), &graph);
}

#[test]
fn squash_and_cherry_pick_equivalence_never_fabricate_parent_edges() {
    let squash_repo = support::committed_repo();
    let squash_base = support::git(squash_repo.path(), ["rev-parse", "HEAD"]);
    support::git(squash_repo.path(), ["switch", "-c", "feature/squash"]);
    let squash_source = commit_file(
        squash_repo.path(),
        "squash.txt",
        "same patch\n",
        "source squash work",
    );
    support::git(squash_repo.path(), ["switch", "main"]);
    support::git(squash_repo.path(), ["merge", "--squash", "feature/squash"]);
    support::git(
        squash_repo.path(),
        ["commit", "-m", "squashed equivalent work"],
    );
    let squash_result = support::git(squash_repo.path(), ["rev-parse", "HEAD"]);
    assert_ne!(squash_source, squash_result);
    assert_eq!(
        support::git(
            squash_repo.path(),
            ["rev-parse", &format!("{squash_source}^{{tree}}")],
        ),
        support::git(
            squash_repo.path(),
            ["rev-parse", &format!("{squash_result}^{{tree}}")],
        ),
        "fixture must contain tree-equivalent squash commits"
    );
    let squash_graph = scan_read_only(squash_repo.path());
    assert_eq!(
        squash_graph
            .commits
            .iter()
            .find(|commit| commit.oid == squash_result)
            .expect("squash result is retained")
            .parents,
        vec![squash_base]
    );
    assert!(
        !squash_graph
            .edges
            .iter()
            .any(|edge| { edge.from_oid == squash_source && edge.to_oid == squash_result })
    );
    assert_edges_are_git_parents(squash_repo.path(), &squash_graph);

    let cherry_repo = support::committed_repo();
    support::git(cherry_repo.path(), ["switch", "-c", "feature/cherry"]);
    let cherry_source = commit_file(
        cherry_repo.path(),
        "cherry.txt",
        "same patch\n",
        "source cherry work",
    );
    support::git(cherry_repo.path(), ["switch", "main"]);
    let cherry_parent = commit_file(cherry_repo.path(), "main.txt", "main\n", "advance main");
    support::git(cherry_repo.path(), ["cherry-pick", &cherry_source]);
    let cherry_result = support::git(cherry_repo.path(), ["rev-parse", "HEAD"]);
    assert_ne!(cherry_source, cherry_result);
    assert_eq!(
        support::git(
            cherry_repo.path(),
            ["rev-parse", &format!("{cherry_source}:cherry.txt")],
        ),
        support::git(
            cherry_repo.path(),
            ["rev-parse", &format!("{cherry_result}:cherry.txt")],
        ),
        "fixture must contain blob-equivalent cherry-picked commits"
    );
    let cherry_graph = scan_read_only(cherry_repo.path());
    assert_eq!(
        cherry_graph
            .commits
            .iter()
            .find(|commit| commit.oid == cherry_result)
            .expect("cherry-pick result is retained")
            .parents,
        vec![cherry_parent]
    );
    assert!(
        !cherry_graph
            .edges
            .iter()
            .any(|edge| { edge.from_oid == cherry_source && edge.to_oid == cherry_result })
    );
    assert_edges_are_git_parents(cherry_repo.path(), &cherry_graph);
}

#[test]
fn diverging_local_and_remote_refs_keep_their_exact_distinct_tips() {
    let repo = support::committed_repo();
    let base = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["switch", "-c", "remote-seed"]);
    let remote_tip = commit_file(repo.path(), "remote.txt", "remote\n", "remote advance");
    support::git(
        repo.path(),
        ["update-ref", "refs/remotes/origin/main", &remote_tip],
    );
    support::git(repo.path(), ["switch", "main"]);
    let local_tip = commit_file(repo.path(), "local.txt", "local\n", "local advance");
    support::git(repo.path(), ["branch", "-D", "remote-seed"]);

    let graph = scan_read_only(repo.path());

    assert_eq!(
        graph
            .refs
            .iter()
            .find(|reference| reference.ref_name == "refs/heads/main")
            .map(|reference| (reference.oid.as_str(), reference.kind.as_str())),
        Some((local_tip.as_str(), "branch"))
    );
    assert_eq!(
        graph
            .refs
            .iter()
            .find(|reference| reference.ref_name == "refs/remotes/origin/main")
            .map(|reference| (reference.oid.as_str(), reference.kind.as_str())),
        Some((remote_tip.as_str(), "remote"))
    );
    for tip in [&local_tip, &remote_tip] {
        assert_eq!(
            graph
                .commits
                .iter()
                .find(|commit| commit.oid == *tip)
                .expect("both diverged tips are retained")
                .parents,
            vec![base.clone()]
        );
    }
    assert!(!graph.edges.iter().any(|edge| {
        (edge.from_oid == local_tip && edge.to_oid == remote_tip)
            || (edge.from_oid == remote_tip && edge.to_oid == local_tip)
    }));
    assert_edges_are_git_parents(repo.path(), &graph);
}

#[test]
fn fast_forwarded_branches_keep_their_distinct_ref_positions() {
    let repo = support::committed_repo();
    let base = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["branch", "release/base"]);
    let tip = commit_file(repo.path(), "advance.txt", "advance\n", "advance main");

    let graph = scan_read_only(repo.path());

    assert_eq!(
        graph
            .refs
            .iter()
            .find(|reference| reference.ref_name == "refs/heads/release/base")
            .map(|reference| reference.oid.as_str()),
        Some(base.as_str())
    );
    assert_eq!(
        graph
            .refs
            .iter()
            .find(|reference| reference.ref_name == "refs/heads/main")
            .map(|reference| reference.oid.as_str()),
        Some(tip.as_str())
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from_oid == base && edge.to_oid == tip)
    );
}

#[test]
fn multiple_ref_kinds_at_one_commit_keep_the_peeled_commit_oid() {
    let repo = support::committed_repo();
    let tip = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["branch", "feature/shared"]);
    support::git(
        repo.path(),
        ["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    support::git(repo.path(), ["tag", "-a", "v1", "-m", "version one"]);
    let tag_object = support::git(repo.path(), ["rev-parse", "refs/tags/v1"]);
    assert_ne!(tag_object, tip, "fixture must use an annotated tag object");

    let graph = scan_read_only(repo.path());

    let expected = [
        ("refs/heads/feature/shared", "branch"),
        ("refs/heads/main", "branch"),
        ("refs/remotes/origin/main", "remote"),
        ("refs/tags/v1", "tag"),
    ];
    for (ref_name, kind) in expected {
        let reference = graph
            .refs
            .iter()
            .find(|reference| reference.ref_name == ref_name)
            .unwrap_or_else(|| panic!("missing {ref_name}"));
        assert_eq!(reference.oid, tip);
        assert_eq!(reference.kind, kind);
    }
}

#[test]
fn non_commit_tags_are_explicitly_incomplete() {
    let repo = support::committed_repo();
    let blob = support::git(repo.path(), ["hash-object", "README.md"]);
    support::git(repo.path(), ["tag", "lightweight-blob", &blob]);
    support::git(
        repo.path(),
        ["tag", "-a", "annotated-blob", &blob, "-m", "blob tag"],
    );

    let graph = scan_read_only(repo.path());

    assert!(!graph.complete);
    assert!(
        graph
            .boundaries
            .iter()
            .any(|boundary| { boundary.oid == blob && boundary.reason == "missing" })
    );
    assert!(!graph.refs.iter().any(|reference| {
        reference.ref_name == "refs/tags/lightweight-blob"
            || reference.ref_name == "refs/tags/annotated-blob"
    }));
}

#[test]
fn partial_clone_scan_does_not_lazy_fetch_a_promised_tag_target() {
    let source = support::committed_repo();
    let blob = support::git(source.path(), ["hash-object", "README.md"]);
    support::git(source.path(), ["config", "uploadpack.allowFilter", "true"]);
    support::git(
        source.path(),
        ["config", "uploadpack.allowAnySHA1InWant", "true"],
    );

    let probe = promisor_clone_without_blob(source.path(), &blob);
    assert!(git_object_is_missing(probe.path(), &blob));
    support::git(probe.path(), ["cat-file", "-p", &blob]);
    assert!(
        !git_object_is_missing(probe.path(), &blob),
        "installed Git must demonstrate lazy fetching in the probe clone"
    );

    let clone = promisor_clone_without_blob(source.path(), &blob);
    assert!(git_object_is_missing(clone.path(), &blob));
    let before = support::source_snapshot(clone.path());
    let before_objects = support::git(clone.path(), ["count-objects", "-v"]);
    let workspace = SourceGitInspector::open(clone.path())
        .expect("open partial clone")
        .workspace()
        .expect("inspect partial clone");
    let worktrees = WorktreeScanner::scan(&workspace).expect("scan partial clone worktrees");

    let graph = GitTopologyCollector::scan(&workspace, &worktrees).expect("scan partial clone");

    assert!(
        git_object_is_missing(clone.path(), &blob),
        "collector lazily fetched a promised object"
    );
    assert!(!graph.complete);
    assert!(
        graph
            .boundaries
            .iter()
            .any(|boundary| { boundary.oid == blob && boundary.reason == "missing" })
    );
    assert_eq!(
        support::git(clone.path(), ["count-objects", "-v"]),
        before_objects
    );
    assert_eq!(support::source_snapshot(clone.path()), before);
}

#[test]
fn displayed_refs_are_capped_at_256_with_an_omission_boundary() {
    let repo = support::committed_repo();
    create_branches(repo.path(), 256);

    let graph = scan_read_only(repo.path());

    assert_eq!(graph.refs.len(), 256);
    assert!(!graph.complete);
    assert!(
        graph
            .boundaries
            .iter()
            .any(|boundary| boundary.reason == "history_limit")
    );
}

#[test]
fn detached_unique_worktree_head_is_a_traversal_tip() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["switch", "--detach"]);
    let detached = commit_file(repo.path(), "detached.txt", "detached\n", "detached work");

    let graph = scan_read_only(repo.path());

    assert!(graph.commits.iter().any(|commit| commit.oid == detached));
    assert!(!graph.refs.iter().any(|reference| reference.oid == detached));
}

#[test]
fn unrelated_roots_are_explicitly_marked() {
    let repo = support::committed_repo();
    let main_root = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["switch", "--orphan", "isolated"]);
    let isolated_root = commit_file(repo.path(), "isolated.txt", "isolated\n", "isolated root");

    let graph = scan_read_only(repo.path());

    for root in [main_root, isolated_root] {
        assert!(
            graph
                .boundaries
                .iter()
                .any(|boundary| { boundary.oid == root && boundary.reason == "unrelated" })
        );
    }
    assert!(graph.complete);
}

#[test]
fn shallow_history_has_an_explicit_incomplete_boundary() {
    let source = support::committed_repo();
    commit_file(source.path(), "second.txt", "second\n", "second commit");
    let clone_parent = tempfile::tempdir().expect("create clone parent");
    let clone = clone_parent.path().join("shallow");
    support::git(
        source.path(),
        [
            "clone",
            "--quiet",
            "--depth=1",
            "--no-local",
            source.path().to_str().expect("source path is UTF-8"),
            clone.to_str().expect("clone path is UTF-8"),
        ],
    );
    let shallow_tip = support::git(&clone, ["rev-parse", "HEAD"]);

    let graph = scan_read_only(&clone);

    assert!(!graph.complete);
    assert!(
        graph
            .boundaries
            .iter()
            .any(|boundary| { boundary.oid == shallow_tip && boundary.reason == "shallow" })
    );
}

#[test]
fn retained_history_is_bounded_and_marks_omitted_parents() {
    let repo = support::committed_repo();
    append_fast_import_history(repo.path(), 2_049);

    let graph = scan_read_only(repo.path());

    assert_eq!(graph.commits.len(), 2_048);
    assert!(!graph.complete);
    assert!(
        graph
            .boundaries
            .iter()
            .any(|boundary| boundary.reason == "history_limit")
    );
    let retained = graph
        .commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let omitted_parent_edges = graph
        .edges
        .iter()
        .filter(|edge| !retained.contains(edge.from_oid.as_str()))
        .collect::<Vec<_>>();
    assert!(!omitted_parent_edges.is_empty());
    for edge in omitted_parent_edges {
        assert!(graph.boundaries.iter().any(|boundary| {
            boundary.oid == edge.from_oid && boundary.reason == "history_limit"
        }));
    }
    assert_edges_are_git_parents(repo.path(), &graph);
}

#[test]
fn unborn_repository_returns_an_empty_complete_graph() {
    let repo = tempfile::tempdir().expect("create unborn repository");
    support::git(repo.path(), ["init", "-b", "main"]);
    let git_dir = repo
        .path()
        .join(".git")
        .canonicalize()
        .expect("canonical git dir");
    let workspace = SourceWorkspace {
        root: repo.path().canonicalize().expect("canonical repo root"),
        git_dir: git_dir.clone(),
        git_common_dir: git_dir,
        branch: Some("main".into()),
        head: String::new(),
    };
    let before_refs = support::git(repo.path(), ["for-each-ref"]);

    let graph = GitTopologyCollector::scan(&workspace, &[]).expect("scan unborn repository");

    assert!(graph.commits.is_empty());
    assert!(graph.refs.is_empty());
    assert!(graph.edges.is_empty());
    assert!(graph.boundaries.is_empty());
    assert!(graph.complete);
    assert_eq!(support::git(repo.path(), ["for-each-ref"]), before_refs);
}

#[test]
fn git_read_failure_is_returned_without_mutating_the_repository() {
    let repo = support::committed_repo();
    let mut workspace = SourceGitInspector::open(repo.path())
        .expect("open source repository")
        .workspace()
        .expect("inspect source workspace");
    let before = support::source_snapshot(repo.path());
    workspace.root = repo.path().join("missing-root");

    let error = GitTopologyCollector::scan(&workspace, &[]).expect_err("Git read must fail");

    assert!(matches!(error, DevMapError::GitCommand { .. }));
    assert_eq!(support::source_snapshot(repo.path()), before);
}

fn append_fast_import_history(repo: &Path, commit_count: usize) {
    let initial = support::git(repo, ["rev-parse", "HEAD"]);
    let mut input = String::new();
    for index in 1..=commit_count {
        let message = format!("generated commit {index}");
        input.push_str("commit refs/heads/main\n");
        input.push_str(&format!("mark :{index}\n"));
        input.push_str(&format!(
            "author DevMap Test <devmap-test@example.test> {} +0000\n",
            1_700_000_000_u64 + index as u64
        ));
        input.push_str(&format!(
            "committer DevMap Test <devmap-test@example.test> {} +0000\n",
            1_700_000_000_u64 + index as u64
        ));
        input.push_str(&format!("data {}\n{}\n", message.len(), message));
        if index == 1 {
            input.push_str(&format!("from {initial}\n"));
        } else {
            input.push_str(&format!("from :{}\n", index - 1));
        }
        input.push_str(&format!(
            "M 100644 inline history.txt\ndata {}\n{}\n\n",
            index.to_string().len(),
            index
        ));
    }
    let mut child = Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start git fast-import");
    child
        .stdin
        .take()
        .expect("fast-import stdin")
        .write_all(input.as_bytes())
        .expect("write fast-import stream");
    let output = child.wait_with_output().expect("wait for git fast-import");
    assert!(
        output.status.success(),
        "git fast-import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    support::git(repo, ["reset", "--hard", "main"]);
}

fn promisor_clone_without_blob(source: &Path, blob: &str) -> tempfile::TempDir {
    let clone = tempfile::tempdir().expect("create promisor repository");
    support::git(clone.path(), ["init", "-b", "main"]);
    let commit = support::git(source, ["rev-parse", "HEAD"]);
    let tree = support::git(source, ["rev-parse", "HEAD^{tree}"]);
    copy_loose_object(source, clone.path(), &commit);
    copy_loose_object(source, clone.path(), &tree);
    support::git(clone.path(), ["update-ref", "refs/heads/main", &commit]);
    support::git(
        clone.path(),
        [
            "remote",
            "add",
            "origin",
            source.to_str().expect("source path is UTF-8"),
        ],
    );
    support::git(
        clone.path(),
        ["config", "extensions.partialClone", "origin"],
    );
    support::git(clone.path(), ["config", "remote.origin.promisor", "true"]);
    support::git(
        clone.path(),
        ["config", "remote.origin.partialCloneFilter", "blob:none"],
    );
    let tag_dir = clone.path().join(".git").join("refs").join("tags");
    fs::create_dir_all(&tag_dir).expect("create tag ref directory");
    fs::write(tag_dir.join("promised-blob"), format!("{blob}\n")).expect("write promised blob ref");
    clone
}

fn copy_loose_object(source: &Path, target: &Path, oid: &str) {
    let (directory, file) = oid.split_at(2);
    let source_object = source
        .join(".git")
        .join("objects")
        .join(directory)
        .join(file);
    let target_directory = target.join(".git").join("objects").join(directory);
    fs::create_dir_all(&target_directory).expect("create loose object directory");
    fs::copy(source_object, target_directory.join(file)).expect("copy loose Git object");
}

fn git_object_is_missing(repo: &Path, oid: &str) -> bool {
    let output = Command::new("git")
        .args(["rev-list", "--objects", "--missing=print", "HEAD"])
        .current_dir(repo)
        .env("GIT_NO_LAZY_FETCH", "1")
        .output()
        .expect("inspect promised object");
    assert!(
        output.status.success(),
        "inspect promised objects failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8(output.stdout).expect("promised-object listing is UTF-8");
    listing.lines().any(|line| line == format!("?{oid}"))
}

fn create_branches(repo: &Path, count: usize) {
    let mut child = Command::new("git")
        .args(["update-ref", "--stdin"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start git update-ref");
    let input = (0..count)
        .map(|index| format!("create refs/heads/cap/{index:03} HEAD\n"))
        .collect::<String>();
    child
        .stdin
        .take()
        .expect("update-ref stdin")
        .write_all(input.as_bytes())
        .expect("write update-ref transaction");
    let output = child.wait_with_output().expect("wait for git update-ref");
    assert!(
        output.status.success(),
        "git update-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
