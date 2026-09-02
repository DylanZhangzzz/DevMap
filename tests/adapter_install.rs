mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use devmap::adapter::{install_adapter, plan_adapter, uninstall_adapter, verify_adapter};
use devmap::cli::{AdapterHost, Cli};
use devmap::error::DevMapError;
use devmap::events::CaptureGrade;
use serde_json::{Value, json};

use support::{committed_repo, git};

const EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "SessionEnd",
];

#[test]
fn plan_is_read_only_and_emits_executable_official_host_bindings() {
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        let fixture = adapter_fixture(host);
        let before = repository_snapshot(fixture.root.path());
        let config_before = fs::read(&fixture.config_path).unwrap();

        let plan = plan_adapter(fixture.root.path(), host).unwrap();

        assert_eq!(plan.host, host);
        assert_eq!(plan.config_path, fixture.config_path);
        assert_eq!(plan.capture_grade, CaptureGrade::A);
        assert_eq!(
            plan.bindings
                .iter()
                .map(|binding| binding.event.as_str())
                .collect::<Vec<_>>(),
            EVENTS
        );
        for binding in &plan.bindings {
            let host_name = host_name(host);
            assert_eq!(
                binding.binding_id,
                format!("devmap/v1/{host_name}/{}", binding.event)
            );
            assert_eq!(
                binding.command,
                format!(
                    "devmap hook handle --host {host_name} --event {} --binding-id {}",
                    binding.event, binding.binding_id
                )
            );
            assert_eq!(binding.matcher, None);
            assert!(!binding.command.contains(['$', '%', '{', '}']));
            Cli::try_parse_from(binding.command.split_ascii_whitespace())
                .expect("the installed canonical handler command must parse without --source");
        }
        assert_eq!(fs::read(&fixture.config_path).unwrap(), config_before);
        assert_eq!(repository_snapshot(fixture.root.path()), before);
    }
}

#[test]
fn install_merges_realistic_configs_idempotently_without_touching_git_state() {
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        let fixture = adapter_fixture(host);
        let before_git = git_metadata_snapshot(fixture.root.path());
        let before_json: Value =
            serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
        let unrelated = unrelated_handler(&before_json).clone();
        let plan = plan_adapter(fixture.root.path(), host).unwrap();

        let report = install_adapter(plan).unwrap();

        assert!(report.changed);
        assert_eq!(report.added.len(), EVENTS.len());
        let installed_bytes = fs::read(&fixture.config_path).unwrap();
        let installed: Value = serde_json::from_slice(&installed_bytes).unwrap();
        assert_eq!(unrelated_handler(&installed), &unrelated);
        assert_eq!(owned_binding_ids(&installed).len(), EVENTS.len());
        assert_eq!(devmap_handler_count(&installed), EVENTS.len());
        assert!(all_devmap_handlers_use_official_shape(&installed));
        assert_git_unchanged_except_config(
            fixture.root.path(),
            &fixture.relative_config,
            &before_git,
        );

        let second = install_adapter(plan_adapter(fixture.root.path(), host).unwrap()).unwrap();
        assert!(!second.changed);
        assert!(second.added.is_empty());
        assert_eq!(fs::read(&fixture.config_path).unwrap(), installed_bytes);
        assert_git_unchanged_except_config(
            fixture.root.path(),
            &fixture.relative_config,
            &before_git,
        );
    }
}

#[test]
fn malformed_or_unrecognized_hook_structures_are_never_overwritten() {
    for invalid in [
        b"{ not json".as_slice(),
        br#"{"hooks":{"SessionStart":{}}}"#,
        br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"future_kind","command":"run"}]}]}}"#,
    ] {
        let root = committed_repo();
        let path = root.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, invalid).unwrap();
        let before = fs::read(&path).unwrap();

        let error = install_adapter(plan_adapter(root.path(), AdapterHost::Codex).unwrap())
            .expect_err("unsafe existing config must be refused");

        assert!(matches!(error, DevMapError::MalformedAdapterConfig(_)));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_file_name("hooks.json.devmap-tmp").exists());
        assert!(!path.with_file_name("hooks.json.devmap-backup").exists());

        let error = uninstall_adapter(root.path(), AdapterHost::Codex)
            .expect_err("uninstall must not rewrite unsafe existing config");
        assert!(matches!(error, DevMapError::MalformedAdapterConfig(_)));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn install_rejects_a_tampered_plan_without_writing() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    let before = fs::read(&fixture.config_path).unwrap();
    let mut plan = plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();
    plan.bindings[0].command = "powershell -Command Invoke-Anything".into();

    let error = install_adapter(plan).expect_err("only canonical plans may be installed");

    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert_eq!(fs::read(&fixture.config_path).unwrap(), before);
}

#[test]
fn verify_reports_present_missing_and_modified_bindings_with_real_grade() {
    let fixture = adapter_fixture(AdapterHost::Claude);
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Claude).unwrap()).unwrap();

    let healthy = verify_adapter(fixture.root.path(), AdapterHost::Claude).unwrap();
    assert_eq!(healthy.present.len(), EVENTS.len());
    assert!(healthy.missing.is_empty());
    assert!(healthy.modified.is_empty());
    assert_eq!(healthy.kernel_command_path, "devmap hook handle");
    assert_eq!(healthy.capture_grade, CaptureGrade::A);
    assert!(healthy.drift_reasons.is_empty());

    let mut config: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    let handler = config["hooks"]["PreToolUse"].as_array_mut().unwrap()[0]["hooks"]
        .as_array_mut()
        .unwrap()
        .first_mut()
        .unwrap();
    handler["command"] = Value::String(
        "devmap hook handle --host claude --event PreToolUse --binding-id devmap/v1/claude/PreToolUse --tampered"
            .to_owned(),
    );
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    let drifted = verify_adapter(fixture.root.path(), AdapterHost::Claude).unwrap();
    assert_eq!(drifted.modified, vec!["devmap/v1/claude/PreToolUse"]);
    assert_eq!(drifted.capture_grade, CaptureGrade::D);
    assert!(
        drifted
            .drift_reasons
            .iter()
            .any(|reason| reason.contains("modified"))
    );
}

#[test]
fn adapter_verify_cli_reports_the_capability_handshake() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap()).unwrap();

    let output = devmap::run([
        "devmap",
        "adapter",
        "verify",
        "--source",
        fixture.root.path().to_str().unwrap(),
        "--host",
        "codex",
    ])
    .unwrap();

    assert_eq!(output.exit_code, 0);
    assert!(
        output
            .stdout
            .contains("kernel_command_path=devmap hook handle")
    );
    assert!(output.stdout.contains("capabilities={"));
    assert!(output.stdout.contains("\"pre_mutation_blocking\":true"));
    assert!(output.stdout.contains("capture_grade=A"));
}

#[test]
fn uninstall_removes_only_devmap_owned_handlers_and_preserves_mixed_groups() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap()).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    let mixed_group = &mut config["hooks"]["SessionStart"].as_array_mut().unwrap()[0];
    mixed_group["hooks"].as_array_mut().unwrap().push(json!({
        "type": "command",
        "command": "devmap hook handle --host legacy --event SessionStart --binding-id devmap/v1/legacy/SessionStart"
    }));
    let unrelated = mixed_group["hooks"][0].clone();
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    let report = uninstall_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();

    assert!(report.changed);
    assert_eq!(report.removed.len(), EVENTS.len() + 1);
    let uninstalled: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    assert!(owned_binding_ids(&uninstalled).is_empty());
    assert_eq!(unrelated_handler(&uninstalled), &unrelated);
    assert!(
        uninstalled["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["hooks"].as_array().unwrap().contains(&unrelated))
    );

    let second = uninstall_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();
    assert!(!second.changed);
    assert!(second.removed.is_empty());
}

struct AdapterFixture {
    root: tempfile::TempDir,
    relative_config: PathBuf,
    config_path: PathBuf,
}

fn adapter_fixture(host: AdapterHost) -> AdapterFixture {
    let root = committed_repo();
    let (relative_config, config) = match host {
        AdapterHost::Codex => (
            PathBuf::from(".codex/hooks.json"),
            json!({
                "description": "Existing project hooks",
                "hooks": {
                    "SessionStart": [{
                        "matcher": "startup|resume",
                        "hooks": [{
                            "type": "command",
                            "command": "python3 /opt/team/session.py",
                            "statusMessage": "Loading team context",
                            "additionalContextLimit": 4096
                        }]
                    }],
                    "PermissionRequest": [{
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "/opt/team/approve"}]
                    }]
                }
            }),
        ),
        AdapterHost::Claude => (
            PathBuf::from(".claude/settings.json"),
            json!({
                "permissions": {"allow": ["Read"]},
                "hooks": {
                    "SessionStart": [{
                        "matcher": "startup|resume",
                        "hooks": [{
                            "type": "command",
                            "command": "/opt/team/session",
                            "timeout": 12,
                            "statusMessage": "Loading team context"
                        }]
                    }],
                    "Notification": [{
                        "matcher": "idle_prompt",
                        "hooks": [{"type": "command", "command": "/opt/team/notify"}]
                    }]
                }
            }),
        ),
        AdapterHost::GenericMcp => unreachable!(),
    };
    let config_path = root.path().join(&relative_config);
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    git(
        root.path(),
        ["add", "--", relative_config.to_str().unwrap()],
    );
    git(root.path(), ["commit", "-m", "add existing host hooks"]);
    AdapterFixture {
        root,
        relative_config,
        config_path,
    }
}

fn unrelated_handler(config: &Value) -> &Value {
    &config["hooks"]["SessionStart"][0]["hooks"][0]
}

fn owned_binding_ids(config: &Value) -> BTreeSet<String> {
    handlers(config)
        .filter_map(handler_binding_id)
        .filter(|id| id.starts_with("devmap/v1/"))
        .map(str::to_owned)
        .collect()
}

fn devmap_handler_count(config: &Value) -> usize {
    handlers(config)
        .filter_map(handler_binding_id)
        .filter(|id| id.starts_with("devmap/v1/"))
        .count()
}

fn all_devmap_handlers_use_official_shape(config: &Value) -> bool {
    handlers(config)
        .filter(|handler| {
            handler_binding_id(handler).is_some_and(|id| id.starts_with("devmap/v1/"))
        })
        .all(|handler| {
            let object = handler.as_object().unwrap();
            object.len() == 3
                && object.get("type") == Some(&Value::String("command".into()))
                && object.get("command").is_some_and(Value::is_string)
                && object.get("statusMessage").is_some_and(Value::is_string)
        })
}

fn handlers(config: &Value) -> impl Iterator<Item = &Value> {
    config["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .flat_map(|groups| groups.as_array().into_iter().flatten())
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
}

fn handler_binding_id(handler: &Value) -> Option<&str> {
    let mut words = handler.get("command")?.as_str()?.split_ascii_whitespace();
    while let Some(word) = words.next() {
        if word == "--binding-id" {
            return words.next();
        }
        if let Some(value) = word.strip_prefix("--binding-id=") {
            return Some(value);
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct GitMetadataSnapshot {
    head: String,
    branch: String,
    index: String,
    refs: String,
    config: String,
    worktrees: String,
}

fn repository_snapshot(root: &Path) -> (Vec<(PathBuf, Vec<u8>)>, GitMetadataSnapshot) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    (files, git_metadata_snapshot(root))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap();
        if relative.starts_with(".git") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push((relative.to_path_buf(), fs::read(path).unwrap()));
        }
    }
}

fn git_metadata_snapshot(root: &Path) -> GitMetadataSnapshot {
    GitMetadataSnapshot {
        head: git(root, ["rev-parse", "HEAD"]),
        branch: git(root, ["branch", "--show-current"]),
        index: git(root, ["ls-files", "--stage"]),
        refs: git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
        config: git(root, ["config", "--local", "--list", "--show-origin"]),
        worktrees: git(root, ["worktree", "list", "--porcelain"]),
    }
}

fn assert_git_unchanged_except_config(root: &Path, config: &Path, before: &GitMetadataSnapshot) {
    assert_eq!(&git_metadata_snapshot(root), before);
    assert_eq!(
        git(root, ["status", "--porcelain=v1", "--untracked-files=all"]),
        format!("M {}", config.to_string_lossy().replace('\\', "/"))
    );
    assert!(git(root, ["diff", "--cached", "--name-only"]).is_empty());
}

fn host_name(host: AdapterHost) -> &'static str {
    match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => "generic-mcp",
    }
}
