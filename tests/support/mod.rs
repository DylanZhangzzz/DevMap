#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use devmap::canonical::canonical_json;
use devmap::events::{CaptureGrade, EventEnvelope, EventType};
use devmap::journal::JournalRecord;
use serde_json::Value;
use tempfile::TempDir;

pub const SCENARIO_SESSION: &str = "phase-1b-session";
pub const SCENARIO_ROUTE: &str = "route-native-capture";
pub const SCENARIO_MAIN_AGENT: &str = "agent-main";
pub const SCENARIO_CHILD_AGENT: &str = "agent-child";
pub const RAW_HOST_PROMPT: &str = "RAW HOST PROMPT: keep compatibility";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedScenarioEvent {
    pub event_type: EventType,
    pub sequence: u64,
    pub actor: &'static str,
    pub parent: Option<&'static str>,
    pub grade: CaptureGrade,
}

pub fn native_scenario_expectations() -> Vec<ExpectedScenarioEvent> {
    use EventType::*;
    vec![
        expected(
            SessionStarted,
            1,
            SCENARIO_MAIN_AGENT,
            None,
            CaptureGrade::A,
        ),
        expected(
            InstructionObserved,
            2,
            SCENARIO_MAIN_AGENT,
            None,
            CaptureGrade::A,
        ),
        expected(
            AgentStarted,
            3,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            DecisionRecorded,
            4,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            ToolCompleted,
            5,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            MutationObserved,
            6,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            CaptureGap,
            7,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            EvidenceRecorded,
            8,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            ContextCompacting,
            9,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            ContextCompacted,
            10,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::A,
        ),
        expected(
            SessionStopped,
            11,
            SCENARIO_MAIN_AGENT,
            None,
            CaptureGrade::A,
        ),
    ]
}

pub fn generic_scenario_expectations() -> Vec<ExpectedScenarioEvent> {
    use EventType::*;
    vec![
        expected(
            InstructionObserved,
            1,
            SCENARIO_MAIN_AGENT,
            None,
            CaptureGrade::C,
        ),
        expected(
            DecisionRecorded,
            2,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::C,
        ),
        expected(
            EvidenceRecorded,
            3,
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            CaptureGrade::C,
        ),
    ]
}

fn expected(
    event_type: EventType,
    sequence: u64,
    actor: &'static str,
    parent: Option<&'static str>,
    grade: CaptureGrade,
) -> ExpectedScenarioEvent {
    ExpectedScenarioEvent {
        event_type,
        sequence,
        actor,
        parent,
        grade,
    }
}

pub fn assert_scenario(records: &[JournalRecord], expected: &[ExpectedScenarioEvent]) {
    assert_eq!(records.len(), expected.len(), "scenario event count");
    for (record, expected) in records.iter().zip(expected) {
        let event = &record.event;
        assert_eq!(event.event_type(), &expected.event_type);
        assert_eq!(event.sequence(), expected.sequence);
        assert_eq!(event.actor().agent_id(), expected.actor);
        assert_eq!(event.actor().parent_agent_id(), expected.parent);
        assert_eq!(event.context().route_id(), Some(SCENARIO_ROUTE));
        assert_eq!(
            serde_json::from_value::<CaptureGrade>(event.payload()["capture_grade"].clone())
                .expect("capture_grade must be valid"),
            expected.grade
        );
    }
}

pub fn canonical_semantic_bytes(event: &EventEnvelope) -> Vec<u8> {
    let mut value = serde_json::to_value(event).expect("serialize event");
    let object = value.as_object_mut().expect("event envelope is an object");
    object.remove("host");
    let payload = object["payload"]
        .as_object_mut()
        .expect("event payload is an object");
    payload.remove("host_metadata");
    if let Some(source) = payload
        .get_mut("requirement_trace")
        .and_then(|trace| trace.get_mut("source"))
        .and_then(Value::as_object_mut)
        && source.get("kind").and_then(Value::as_str) == Some("host_prompt_reference")
        && let Some(locator) = source.get("locator").and_then(Value::as_str)
        && let Some((_, semantic_locator)) = locator.split_once(":session:")
    {
        source.insert(
            "locator".into(),
            Value::String(format!("host:session:{semantic_locator}")),
        );
    }
    canonical_json(&value).expect("canonicalize semantic event")
}

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

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn linked_worktree(repo: &Path, branch: &str) -> TempDir {
    let directory = tempfile::tempdir().expect("create linked worktree directory");
    git(
        repo,
        [
            "worktree",
            "add",
            "-b",
            branch,
            directory.path().to_str().expect("worktree path is UTF-8"),
        ],
    );
    directory
}
