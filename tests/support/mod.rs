#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

use devmap::events::CaptureGrade;
use devmap::git::SourceWorkspace;
use devmap::journal::{JournalIntegrity, JournalSummary};
use devmap::presence::{Confidence, PresenceRecord, PresenceStatus, StatusSource};
use devmap::worktrees::{WorktreeDescriptor, WorktreeScanner, repository_id};

pub const SCENARIO_SESSION: &str = "phase-1b-session";
pub const SCENARIO_ROUTE: &str = "route-native-capture";
pub const SCENARIO_MAIN_AGENT: &str = "agent-main";
pub const SCENARIO_CHILD_AGENT: &str = "agent-child";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetadataSnapshot {
    pub head: String,
    pub branch: String,
    pub index: String,
    pub refs: String,
    pub config: String,
    pub stash: String,
    pub remotes: String,
    pub worktrees: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub git: GitMetadataSnapshot,
}

pub fn source_snapshot(root: &Path) -> SourceSnapshot {
    let mut files = BTreeMap::new();
    collect_source_files(root, root, &mut files);
    SourceSnapshot {
        files,
        git: GitMetadataSnapshot {
            head: git(root, ["rev-parse", "HEAD"]),
            branch: git(root, ["branch", "--show-current"]),
            index: git(root, ["ls-files", "--stage"]),
            refs: git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
            config: git(root, ["config", "--local", "--list", "--show-origin"]),
            stash: git(root, ["stash", "list", "--format=%gd:%H"]),
            remotes: git(root, ["remote", "-v"]),
            worktrees: git(root, ["worktree", "list", "--porcelain"]),
        },
    }
}

pub fn assert_only_source_paths_changed(
    before: &SourceSnapshot,
    after: &SourceSnapshot,
    expected_paths: &[&Path],
) {
    assert_eq!(after.git, before.git, "source Git metadata changed");
    let changed = before
        .files
        .keys()
        .chain(after.files.keys())
        .filter(|path| before.files.get(*path) != after.files.get(*path))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = expected_paths
        .iter()
        .map(|path| path.to_path_buf())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(changed, expected, "unexpected source-root file changes");
}

fn collect_source_files(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(directory).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("source-relative path");
        if relative.starts_with(".git") {
            continue;
        }
        if path.is_dir() {
            collect_source_files(root, &path, files);
        } else {
            files.insert(
                relative.to_path_buf(),
                fs::read(path).expect("read source file"),
            );
        }
    }
}

pub fn git<I, S>(root: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(root, args);
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

pub fn git_output<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git")
}

pub fn committed_repo() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary repository");
    git(directory.path(), ["init", "-b", "main"]);
    git(directory.path(), ["config", "user.name", "DevMap Test"]);
    git(
        directory.path(),
        ["config", "user.email", "devmap-test@example.test"],
    );
    fs::write(directory.path().join("README.md"), "# Fixture\n").expect("write fixture");
    git(directory.path(), ["add", "--", "README.md"]);
    git(directory.path(), ["commit", "-m", "initial fixture"]);
    directory
}

pub fn linked_worktree(repo: &Path, branch: &str) -> TempDir {
    linked_worktree_from(repo, branch, "HEAD")
}

pub fn linked_worktree_from(repo: &Path, branch: &str, start_point: &str) -> TempDir {
    let directory = tempfile::tempdir().expect("create linked worktree directory");
    git(
        repo,
        [
            "worktree",
            "add",
            "-b",
            branch,
            directory.path().to_str().expect("worktree path is UTF-8"),
            start_point,
        ],
    );
    directory
}

pub struct LiveDockFixture {
    pub repo: TempDir,
    pub agent_a: TempDir,
    pub agent_b: TempDir,
}

pub fn live_dock_fixture() -> LiveDockFixture {
    let repo = committed_repo();
    let agent_a = linked_worktree(repo.path(), "codex/agent-a");
    let agent_b = linked_worktree(repo.path(), "codex/agent-b");
    LiveDockFixture {
        repo,
        agent_a,
        agent_b,
    }
}

pub fn presence_record(status: PresenceStatus) -> PresenceRecord {
    let (status_source, confidence, lease_expires_at) = match status {
        PresenceStatus::Unknown => (StatusSource::GitOnly, Confidence::Unknown, None),
        PresenceStatus::Completed => (StatusSource::CaptureEvent, Confidence::Observed, None),
        PresenceStatus::Stale => (
            StatusSource::Lease,
            Confidence::Leased,
            Some("2026-09-02T12:00:00Z".into()),
        ),
        _ => (
            StatusSource::CaptureEvent,
            Confidence::Observed,
            Some("2026-09-02T12:02:00Z".into()),
        ),
    };
    PresenceRecord {
        schema_version: 1,
        repository_id: format!("sha256-{}", "1".repeat(64)),
        worktree_id: format!("wt-{}", "2".repeat(64)),
        session_id: "session-presence".into(),
        actor_id: "agent-main".into(),
        host: "codex".into(),
        route_id: None,
        branch: Some("main".into()),
        head: "3".repeat(40),
        status,
        status_source,
        confidence,
        capture_grade: CaptureGrade::D,
        last_event_at: "2026-09-02T12:00:00Z".into(),
        lease_expires_at,
        current_activity_id: None,
        current_decision_id: None,
        blocker_count: 0,
        gap_count: 0,
    }
}

pub struct DockReducerFixture {
    pub workspace: SourceWorkspace,
    pub worktrees: Vec<WorktreeDescriptor>,
    pub presence: devmap::presence::PresenceLoadReport,
    pub journals: BTreeMap<String, JournalSummary>,
    pub now: time::OffsetDateTime,
    _repo: TempDir,
    _linked: TempDir,
}

pub fn dock_reducer_fixture() -> DockReducerFixture {
    let repo = committed_repo();
    let linked = linked_worktree(repo.path(), "codex/dock-other");
    let workspace = devmap::git::SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let other = worktrees.iter().find(|row| !row.is_current).unwrap();
    let mut active = presence_record(PresenceStatus::Working);
    active.repository_id = repository_id(&workspace);
    active.worktree_id = other.worktree_id.clone();
    active.session_id = "active-session".into();
    active.branch = other.branch.clone();
    active.head = other.head.clone();
    let journals = BTreeMap::from([(
        active.session_id.clone(),
        JournalSummary {
            session_id: active.session_id.clone(),
            records: 1,
            last_sequence: Some(1),
            last_sha256: Some("a".repeat(64)),
            integrity: JournalIntegrity::Verified,
        },
    )]);
    DockReducerFixture {
        workspace,
        worktrees,
        presence: devmap::presence::PresenceLoadReport {
            records: vec![active],
            warnings: vec![],
            truncated: false,
        },
        journals,
        now: time::OffsetDateTime::parse(
            "2026-09-02T12:00:30Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap(),
        _repo: repo,
        _linked: linked,
    }
}
