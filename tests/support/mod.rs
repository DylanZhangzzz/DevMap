#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use devmap::canonical::canonical_json;
use devmap::cli::AdapterHost;
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

pub fn assert_native_semantic_payloads(records: &[JournalRecord], host: AdapterHost, head: &str) {
    let (locator, prompt_metadata) = match host {
        AdapterHost::Codex => (
            "codex:session:phase-1b-session:event:evt-02",
            serde_json::json!({"cwd": "/workspace/devmap"}),
        ),
        AdapterHost::Claude => (
            "claude:session:phase-1b-session:event:evt-02",
            serde_json::json!({}),
        ),
        AdapterHost::GenericMcp => panic!("native payload assertion requires a native host"),
    };
    assert_eq!(
        scenario_event(records, &EventType::InstructionObserved).payload(),
        &serde_json::json!({
            "capture_grade": "A",
            "host_metadata": prompt_metadata,
            "requirement_trace": {
                "source": {"kind": "host_prompt_reference", "locator": locator},
                "content_digest": "sha256-276b20dc9323c27dbdb3fe16e1f80beb63ccf80b8bc9156dbabe3b052d7a9805",
                "content_stored": false
            }
        })
    );
    assert_eq!(
        scenario_event(records, &EventType::DecisionRecorded).payload(),
        &serde_json::json!({
            "capture_grade": "A",
            "agent_decision": {
                "decision": "Use a compatibility adapter",
                "basis": ["Both supported hosts expose lifecycle hooks"],
                "alternatives": ["Duplicate host-specific capture logic"],
                "rationale": "A shared kernel preserves equivalent semantics",
                "scope": "Phase 1B native capture",
                "authority": "approved Phase 1B plan",
                "revisit_trigger": "A host cannot express the canonical contract"
            }
        })
    );
    assert_eq!(
        scenario_event(records, &EventType::EvidenceRecorded).payload(),
        &serde_json::json!({
            "capture_grade": "A",
            "evidence": {
                "kind": "test",
                "target": format!("commit:{head}"),
                "command": "cargo test --all-targets --all-features",
                "outcome": "passed"
            },
            "provisional": false
        })
    );
}

pub fn assert_generic_semantic_payloads(records: &[JournalRecord], head: &str) {
    assert_eq!(
        scenario_event(records, &EventType::InstructionObserved).payload(),
        &serde_json::json!({
            "capture_grade": "C",
            "requirement_trace": {
                "source": {"kind": "human_instruction", "locator": "turn:1"},
                "approved_quotation": "Approved requirement quotation"
            }
        })
    );
    assert_eq!(
        scenario_event(records, &EventType::DecisionRecorded).payload(),
        &serde_json::json!({
            "capture_grade": "C",
            "agent_decision": {
                "decision": "Use a compatibility adapter",
                "basis": ["Both supported hosts expose lifecycle hooks"],
                "alternatives": ["Duplicate host-specific capture logic"],
                "rationale": "A shared kernel preserves equivalent semantics",
                "scope": "Phase 1B native capture",
                "authority": "approved Phase 1B plan",
                "revisit_trigger": "A host cannot express the canonical contract"
            }
        })
    );
    assert_eq!(
        scenario_event(records, &EventType::EvidenceRecorded).payload(),
        &serde_json::json!({
            "capture_grade": "C",
            "evidence": {
                "kind": "test",
                "target": format!("commit:{head}"),
                "command": "cargo test --all-targets --all-features",
                "outcome": "passed"
            },
            "provisional": false
        })
    );
}

pub fn assert_native_host_representation(records: &[JournalRecord], host: AdapterHost) {
    let (host_name, locator, ordinary_metadata, write_metadata) = match host {
        AdapterHost::Codex => (
            "codex",
            "codex:session:phase-1b-session:event:evt-02",
            serde_json::json!({"cwd": "/workspace/devmap"}),
            serde_json::json!({"cwd": "/workspace/devmap", "tool_input": {"path": "src/lib.rs"}}),
        ),
        AdapterHost::Claude => (
            "claude",
            "claude:session:phase-1b-session:event:evt-02",
            serde_json::json!({}),
            serde_json::json!({"tool_input": {"path": "src/lib.rs"}}),
        ),
        AdapterHost::GenericMcp => panic!("native representation requires a native host"),
    };
    for record in records {
        let event = &record.event;
        assert_eq!(event.host().name(), host_name);
        assert_eq!(event.host().adapter_version(), "devmap-hook/1");
        if matches!(
            event.event_type(),
            EventType::DecisionRecorded | EventType::EvidenceRecorded
        ) {
            assert!(
                event.payload().get("host_metadata").is_none(),
                "explicit semantic records must not gain host metadata"
            );
            continue;
        }
        let expected = if matches!(
            event.event_type(),
            EventType::ToolCompleted | EventType::MutationObserved | EventType::CaptureGap
        ) {
            &write_metadata
        } else {
            &ordinary_metadata
        };
        assert_eq!(
            event.payload().get("host_metadata"),
            Some(expected),
            "unexpected host metadata shape"
        );
    }
    assert_eq!(
        scenario_event(records, &EventType::InstructionObserved).payload()["requirement_trace"]["source"]
            ["locator"],
        locator,
        "native prompt locator must retain the exact allowed host prefix"
    );
}

pub fn shared_semantic_projection(records: &[JournalRecord]) -> Vec<Vec<u8>> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record.event.event_type(),
                EventType::InstructionObserved
                    | EventType::DecisionRecorded
                    | EventType::EvidenceRecorded
            )
        })
        .map(|record| {
            let mut value = serde_json::to_value(&record.event).expect("serialize event");
            let object = value.as_object_mut().expect("event envelope is an object");
            // Native hooks have lifecycle events before and between these records, while Generic
            // MCP sequences only its explicit semantic calls. Both sequence contracts are checked
            // literally before this projection. Host identity and Grade A/C are likewise asserted
            // before removing those documented capability differences here.
            object.remove("host");
            object.remove("sequence");
            let payload = object["payload"]
                .as_object_mut()
                .expect("event payload is an object");
            payload.remove("capture_grade");
            payload.remove("host_metadata");
            if record.event.event_type() == &EventType::InstructionObserved {
                // Native hooks retain a digest/reference and Generic MCP retains only the explicit
                // approved quotation. Their exact, intentionally non-equivalent representations
                // are checked above; the common semantic role is the comparable evidence here.
                *payload =
                    serde_json::json!({"requirement_trace": {"semantic_role": "human_request"}})
                        .as_object()
                        .unwrap()
                        .clone();
            }
            canonical_json(&value).expect("canonicalize shared semantic projection")
        })
        .collect()
}

fn scenario_event<'a>(records: &'a [JournalRecord], event_type: &EventType) -> &'a EventEnvelope {
    let matching = records
        .iter()
        .filter(|record| record.event.event_type() == event_type)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1, "semantic event must occur exactly once");
    &matching[0].event
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
