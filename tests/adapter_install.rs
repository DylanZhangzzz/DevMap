mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use devmap::adapter::{
    AdapterPlan, InstallReport, install_adapter as install_reviewed, plan_adapter,
    plan_uninstall_adapter, uninstall_adapter as uninstall_reviewed, verify_adapter,
};
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

fn install_adapter(plan: AdapterPlan) -> Result<InstallReport, DevMapError> {
    let token = plan.plan_digest.clone();
    install_reviewed(plan, &token)
}

fn uninstall_adapter(source: &Path, host: AdapterHost) -> Result<InstallReport, DevMapError> {
    let plan = plan_uninstall_adapter(source, host)?;
    let token = plan.plan_digest.clone();
    uninstall_reviewed(plan, &token)
}

#[test]
fn plan_is_read_only_and_emits_executable_official_host_bindings() {
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        let fixture = adapter_fixture(host);
        let before = repository_snapshot(fixture.root.path());
        let config_before = fs::read(&fixture.config_path).unwrap();

        let plan = plan_adapter(fixture.root.path(), host).unwrap();

        assert_eq!(plan.host, host);
        assert_eq!(plan.config_path, fixture.config_path);
        assert_eq!(plan.capture_grade, CaptureGrade::D);
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
fn generic_mcp_plan_is_read_only_and_names_only_the_stdio_descriptor() {
    let root = committed_repo();
    let before = repository_snapshot(root.path());
    let config_path = devmap::git::SourceGitInspector::open(root.path())
        .unwrap()
        .root()
        .join(".devmap/mcp.json");

    let output = devmap::run([
        "devmap",
        "adapter",
        "plan",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
    ])
    .unwrap();

    assert_eq!(output.exit_code, 0);
    assert!(
        output
            .stdout
            .contains(&format!("config_path={}", config_path.display()))
    );
    assert!(output.stdout.contains("capture_grade=D"));
    assert!(output.stdout.contains("plan_digest=sha256-"));
    assert!(!config_path.exists());
    assert_eq!(repository_snapshot(root.path()), before);
}

#[test]
fn generic_mcp_install_verify_and_uninstall_are_safe_and_idempotent() {
    let root = committed_repo();
    let config_path = root.path().join(".devmap/mcp.json");
    let relative_config = Path::new(".devmap/mcp.json");
    let before_git = git_metadata_snapshot(root.path());
    let first_plan = plan_adapter(root.path(), AdapterHost::GenericMcp).unwrap();
    let first_token = first_plan.plan_digest;

    let install = devmap::run([
        "devmap",
        "adapter",
        "install",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
        "--plan-digest",
        &first_token,
    ])
    .unwrap();
    assert!(install.stdout.contains("changed=true"));
    let installed = fs::read(&config_path).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&installed).unwrap(),
        json!({
            "command": ["devmap", "mcp", "--source", "."],
            "transport": "stdio"
        })
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&installed)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(&git_metadata_snapshot(root.path()), &before_git);
    assert_eq!(
        git(
            root.path(),
            ["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        format!(
            "?? {}",
            relative_config.to_string_lossy().replace('\\', "/")
        )
    );
    assert!(git(root.path(), ["diff", "--cached", "--name-only"]).is_empty());

    let second_plan = plan_adapter(root.path(), AdapterHost::GenericMcp).unwrap();
    let second_token = second_plan.plan_digest;
    let second = devmap::run([
        "devmap",
        "adapter",
        "install",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
        "--plan-digest",
        &second_token,
    ])
    .unwrap();
    assert!(second.stdout.contains("changed=false"));
    assert_eq!(fs::read(&config_path).unwrap(), installed);

    let verify = devmap::run([
        "devmap",
        "adapter",
        "verify",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
    ])
    .unwrap();
    assert_eq!(verify.exit_code, 0);
    assert!(verify.stdout.contains("present=descriptor"));
    assert!(verify.stdout.contains("capture_grade=D"));

    let removal_plan = plan_uninstall_adapter(root.path(), AdapterHost::GenericMcp).unwrap();
    let removal_token = removal_plan.plan_digest;
    let uninstall = devmap::run([
        "devmap",
        "adapter",
        "uninstall",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
        "--plan-digest",
        &removal_token,
    ])
    .unwrap();
    assert!(uninstall.stdout.contains("changed=true"));
    assert!(!config_path.exists());
    let second_removal = plan_uninstall_adapter(root.path(), AdapterHost::GenericMcp).unwrap();
    let second_removal_token = second_removal.plan_digest;
    let second_uninstall = devmap::run([
        "devmap",
        "adapter",
        "uninstall",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
        "--plan-digest",
        &second_removal_token,
    ])
    .unwrap();
    assert!(second_uninstall.stdout.contains("changed=false"));
}

#[test]
fn generic_mcp_never_overwrites_unrecognized_or_stale_config() {
    for bytes in [b"{not json".as_slice(), br#"{"transport":"http"}"#] {
        let root = committed_repo();
        let config_path = root.path().join(".devmap/mcp.json");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, bytes).unwrap();

        let error = plan_adapter(root.path(), AdapterHost::GenericMcp)
            .expect_err("unrecognized Generic MCP config must be preserved");

        assert!(matches!(error, DevMapError::MalformedAdapterConfig(_)));
        assert_eq!(fs::read(&config_path).unwrap(), bytes);
    }

    let root = committed_repo();
    let stale = root.path().join(".devmap/mcp.json.devmap-tmp");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, b"stale").unwrap();
    let plan = plan_adapter(root.path(), AdapterHost::GenericMcp).unwrap();
    let token = plan.plan_digest;
    let error = devmap::run([
        "devmap",
        "adapter",
        "install",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
        "--plan-digest",
        &token,
    ])
    .expect_err("a stale transaction artifact must stop installation");
    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert_eq!(fs::read(stale).unwrap(), b"stale");
}

#[test]
fn generic_mcp_install_refuses_an_existing_symlink_even_when_content_matches() {
    let root = committed_repo();
    let config_path = root.path().join(".devmap/mcp.json");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let external = tempfile::tempdir().unwrap();
    let external_config = external.path().join("mcp.json");
    let descriptor = serde_json::to_vec_pretty(&json!({
        "command": ["devmap", "mcp", "--source", "."],
        "transport": "stdio"
    }))
    .unwrap();
    fs::write(&external_config, &descriptor).unwrap();
    if !create_file_symlink(&external_config, &config_path) {
        return;
    }

    let error = plan_adapter(root.path(), AdapterHost::GenericMcp)
        .expect_err("an existing descriptor symlink must not be trusted");

    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert_eq!(fs::read(&external_config).unwrap(), descriptor);
}

#[test]
fn generic_mcp_verify_rejects_a_symlink_even_when_content_matches() {
    let root = committed_repo();
    let config_path = root.path().join(".devmap/mcp.json");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let external = tempfile::tempdir().unwrap();
    let external_config = external.path().join("mcp.json");
    let descriptor = serde_json::to_vec_pretty(&json!({
        "command": ["devmap", "mcp", "--source", "."],
        "transport": "stdio"
    }))
    .unwrap();
    fs::write(&external_config, &descriptor).unwrap();
    if !create_file_symlink(&external_config, &config_path) {
        return;
    }

    let error = devmap::run([
        "devmap",
        "adapter",
        "verify",
        "--source",
        root.path().to_str().unwrap(),
        "--host",
        "generic-mcp",
    ])
    .expect_err("verify must not trust a descriptor symlink");

    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert_eq!(fs::read(&external_config).unwrap(), descriptor);
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
        assert_eq!(devmap_minimal_group_count(&installed), EVENTS.len());
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

        let error = plan_adapter(root.path(), AdapterHost::Codex)
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
fn validation_is_host_aware_and_checks_documented_optional_field_types() {
    let codex = adapter_fixture(AdapterHost::Codex);
    let codex_http = json!({
        "hooks": {"PreToolUse": [{"hooks": [{
            "type": "http",
            "url": "https://hooks.example.invalid/pre-tool"
        }]}]}
    });
    assert_install_refused_without_rewrite(&codex, &codex_http);

    let invalid_optionals = [
        json!({"type": "command", "command": "/opt/team/run", "timeout": "30"}),
        json!({"type": "command", "command": "/opt/team/run", "statusMessage": false}),
        json!({"type": "command", "command": "/opt/team/run", "async": "yes"}),
        json!({
            "type": "command",
            "command": "/opt/team/run",
            "additionalContextLimit": -1
        }),
    ];
    for handler in invalid_optionals {
        let document = json!({"hooks": {"PreToolUse": [{"hooks": [handler]}]}});
        assert_install_refused_without_rewrite(&codex, &document);
    }
    let codex_session_end_mcp = json!({
        "hooks": {"SessionEnd": [{"hooks": [{
            "type": "mcp_tool", "server": "audit", "tool": "record"
        }]}]}
    });
    assert_install_refused_without_rewrite(&codex, &codex_session_end_mcp);
    let codex_supported_mcp = json!({
        "description": "Existing project hooks",
        "hooks": {"PostToolUse": [{"matcher": "Write|Edit", "hooks": [{
            "type": "mcp_tool",
            "server": "scanner",
            "tool": "scan_patch",
            "input": {"patch": "${tool_input.command}"},
            "timeout": 30,
            "statusMessage": "Scanning edits"
        }]}]}
    });
    fs::write(
        &codex.config_path,
        serde_json::to_vec_pretty(&codex_supported_mcp).unwrap(),
    )
    .unwrap();
    install_adapter(plan_adapter(codex.root.path(), AdapterHost::Codex).unwrap())
        .expect("a documented Codex MCP hook should be preserved");

    let claude = adapter_fixture(AdapterHost::Claude);
    let claude_supported = json!({
        "permissions": {"allow": ["Read"]},
        "hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [{
                "type": "http",
                "url": "https://hooks.example.invalid/pre-tool",
                "headers": {"Authorization": "Bearer $TOKEN"},
                "allowedEnvVars": ["TOKEN"],
                "timeout": 30,
                "statusMessage": "Reviewing command",
                "once": false
            }]}],
            "Stop": [{"hooks": [{
                "type": "prompt",
                "prompt": "Check completion: $ARGUMENTS",
                "model": "haiku",
                "continueOnBlock": true
            }]}],
            "SessionStart": [{"hooks": [{
                "type": "mcp_tool",
                "server": "audit",
                "tool": "record",
                "input": {"source": "${source}"}
            }]}]
        }
    });
    fs::write(
        &claude.config_path,
        serde_json::to_vec_pretty(&claude_supported).unwrap(),
    )
    .unwrap();
    install_adapter(plan_adapter(claude.root.path(), AdapterHost::Claude).unwrap())
        .expect("documented Claude handler variants should be preserved");

    let claude_unsupported_event = json!({
        "hooks": {"SessionStart": [{"hooks": [{
            "type": "agent",
            "prompt": "Inspect the repository"
        }]}]}
    });
    assert_install_refused_without_rewrite(&claude, &claude_unsupported_event);
}

#[test]
fn claude_rejects_known_fields_on_the_wrong_handler_type() {
    let fixture = adapter_fixture(AdapterHost::Claude);
    let mismatches = [
        json!({"type": "http", "url": "https://example.invalid", "command": "/bin/true"}),
        json!({"type": "http", "url": "https://example.invalid", "args": ["--check"]}),
        json!({"type": "http", "url": "https://example.invalid", "async": true}),
        json!({"type": "http", "url": "https://example.invalid", "asyncRewake": true}),
        json!({"type": "http", "url": "https://example.invalid", "shell": "bash"}),
        json!({"type": "command", "command": "/bin/true", "url": "https://example.invalid"}),
        json!({"type": "command", "command": "/bin/true", "headers": {"X-Audit": "on"}}),
        json!({"type": "command", "command": "/bin/true", "allowedEnvVars": ["TOKEN"]}),
        json!({"type": "command", "command": "/bin/true", "server": "audit"}),
        json!({"type": "command", "command": "/bin/true", "tool": "record"}),
        json!({"type": "command", "command": "/bin/true", "input": {"event": "${hook_event_name}"}}),
        json!({"type": "command", "command": "/bin/true", "prompt": "Check this"}),
        json!({"type": "command", "command": "/bin/true", "model": "haiku"}),
        json!({"type": "agent", "prompt": "Check this", "continueOnBlock": true}),
        json!({"type": "command", "command": "/bin/true", "shell": "cmd"}),
    ];

    for handler in mismatches {
        assert_install_refused_without_rewrite(
            &fixture,
            &document_with_handler("PreToolUse", handler),
        );
    }

    let extensible = document_with_handler(
        "PreToolUse",
        json!({
            "type": "command",
            "command": "/bin/true",
            "futureExtension": {"opaque": true}
        }),
    );
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&extensible).unwrap(),
    )
    .unwrap();
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Claude).unwrap())
        .expect("unknown extension fields should remain forward-compatible");
}

#[test]
fn codex_rejects_known_fields_on_the_wrong_handler_type() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    let mismatches = [
        json!({"type": "command", "command": "/bin/true", "server": "audit"}),
        json!({"type": "command", "command": "/bin/true", "tool": "record"}),
        json!({"type": "command", "command": "/bin/true", "input": {"event": "${hook_event_name}"}}),
        json!({"type": "mcp_tool", "server": "audit", "tool": "record", "command": "/bin/true"}),
        json!({"type": "mcp_tool", "server": "audit", "tool": "record", "commandWindows": "cmd /c exit 0"}),
        json!({"type": "mcp_tool", "server": "audit", "tool": "record", "additionalContextLimit": 1024}),
        json!({"type": "mcp_tool", "server": "audit", "tool": "record", "async": true}),
    ];

    for handler in mismatches {
        assert_install_refused_without_rewrite(
            &fixture,
            &document_with_handler("PostToolUse", handler),
        );
    }

    let extensible = document_with_handler(
        "PostToolUse",
        json!({
            "type": "mcp_tool",
            "server": "audit",
            "tool": "record",
            "futureExtension": {"opaque": true}
        }),
    );
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&extensible).unwrap(),
    )
    .unwrap();
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap())
        .expect("unknown extension fields should remain forward-compatible");
}

#[test]
fn claude_accepts_worktree_create_http_but_rejects_prompt_handlers() {
    let fixture = adapter_fixture(AdapterHost::Claude);
    let valid = document_with_handler(
        "WorktreeCreate",
        json!({
            "type": "http",
            "url": "https://hooks.example.invalid/worktree",
            "headers": {"Authorization": "Bearer $TOKEN"},
            "allowedEnvVars": ["TOKEN"]
        }),
    );
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&valid).unwrap(),
    )
    .unwrap();
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Claude).unwrap())
        .expect("Claude documents HTTP handlers for WorktreeCreate");

    let invalid = document_with_handler(
        "WorktreeCreate",
        json!({"type": "prompt", "prompt": "Create a worktree"}),
    );
    assert_install_refused_without_rewrite(&fixture, &invalid);
}

#[test]
fn install_rejects_a_tampered_plan_without_writing() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    let before = fs::read(&fixture.config_path).unwrap();
    let mut plan = plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();
    plan.bindings[0].command = "powershell -Command Invoke-Anything".into();

    let error = install_adapter(plan).expect_err("only canonical plans may be installed");

    assert!(matches!(error, DevMapError::AdapterPlanStale(_)));
    assert_eq!(fs::read(&fixture.config_path).unwrap(), before);
}

#[test]
fn install_refuses_stale_devmap_artifacts_without_changing_any_file() {
    for suffix in [".devmap-tmp", ".devmap-backup"] {
        let fixture = adapter_fixture(AdapterHost::Codex);
        let original = fs::read(&fixture.config_path).unwrap();
        let artifact = fixture
            .config_path
            .with_file_name(format!("hooks.json{suffix}"));
        fs::write(&artifact, b"stale artifact owned by an earlier run").unwrap();

        let error = install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap())
            .expect_err("a stale transaction artifact must stop installation");

        assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
        assert_eq!(fs::read(&fixture.config_path).unwrap(), original);
        assert_eq!(
            fs::read(&artifact).unwrap(),
            b"stale artifact owned by an earlier run"
        );
    }
}

#[test]
fn install_refuses_a_symlinked_config_without_changing_its_target() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    let external = tempfile::tempdir().unwrap();
    let external_config = external.path().join("outside-hooks.json");
    let original = fs::read(&fixture.config_path).unwrap();
    fs::write(&external_config, &original).unwrap();
    fs::remove_file(&fixture.config_path).unwrap();
    if !create_file_symlink(&external_config, &fixture.config_path) {
        return;
    }

    let error = plan_adapter(fixture.root.path(), AdapterHost::Codex)
        .expect_err("a project config symlink must never be replaced");

    assert!(matches!(error, DevMapError::UnsafeInstallerOverwrite(_)));
    assert_eq!(fs::read(&external_config).unwrap(), original);
    assert!(
        fs::symlink_metadata(&fixture.config_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
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
    assert_eq!(healthy.capture_grade, CaptureGrade::D);
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
    assert!(output.stdout.contains("\"pre_mutation_blocking\":false"));
    assert!(output.stdout.contains("capture_grade=D"));
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
    let deceptive_user_handler = json!({
        "type": "command",
        "command": "/opt/team/run --binding-id devmap/v1/not-owned"
    });
    mixed_group["hooks"]
        .as_array_mut()
        .unwrap()
        .push(deceptive_user_handler.clone());
    let unrelated = mixed_group["hooks"][0].clone();
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    let report = uninstall_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();

    assert!(report.changed);
    assert_eq!(report.removed.len(), EVENTS.len());
    let uninstalled: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    assert_eq!(
        owned_binding_ids(&uninstalled),
        BTreeSet::from(["devmap/v1/legacy/SessionStart".to_owned()])
    );
    assert!(handlers(&uninstalled).any(|handler| handler == &deceptive_user_handler));
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

#[test]
fn uninstall_preserves_user_authored_group_after_removing_its_only_devmap_handler() {
    let fixture = adapter_fixture(AdapterHost::Codex);
    install_adapter(plan_adapter(fixture.root.path(), AdapterHost::Codex).unwrap()).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    let devmap_group = &mut config["hooks"]["SessionStart"].as_array_mut().unwrap()[1];
    devmap_group["matcher"] = Value::String("startup".into());
    devmap_group["ownerNote"] = Value::String("user metadata".into());
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    uninstall_adapter(fixture.root.path(), AdapterHost::Codex).unwrap();

    let uninstalled: Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).unwrap()).unwrap();
    let preserved = uninstalled["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group.get("ownerNote").is_some())
        .expect("a user-authored group must not be deleted");
    assert_eq!(preserved["matcher"], "startup");
    assert_eq!(preserved["ownerNote"], "user metadata");
    assert_eq!(preserved["hooks"], json!([]));
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

fn devmap_minimal_group_count(config: &Value) -> usize {
    config["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .flat_map(|groups| groups.as_array().into_iter().flatten())
        .filter(|group| {
            let Some(object) = group.as_object() else {
                return false;
            };
            object.len() == 1
                && object
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.len() == 1
                            && handler_binding_id(&handlers[0])
                                .is_some_and(|id| id.starts_with("devmap/v1/"))
                    })
        })
        .count()
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
    if words.next()? != "devmap" || words.next()? != "hook" || words.next()? != "handle" {
        return None;
    }
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

fn assert_install_refused_without_rewrite(fixture: &AdapterFixture, document: &Value) {
    let bytes = serde_json::to_vec_pretty(document).unwrap();
    fs::write(&fixture.config_path, &bytes).unwrap();

    let error = plan_adapter(fixture.root.path(), fixture_host(&fixture.relative_config))
        .expect_err("unsupported or malformed host configuration must be refused");

    assert!(matches!(error, DevMapError::MalformedAdapterConfig(_)));
    assert_eq!(fs::read(&fixture.config_path).unwrap(), bytes);
}

fn fixture_host(relative_config: &Path) -> AdapterHost {
    if relative_config == Path::new(".codex/hooks.json") {
        AdapterHost::Codex
    } else {
        AdapterHost::Claude
    }
}

fn document_with_handler(event: &str, handler: Value) -> Value {
    json!({
        "description": "Existing project hooks",
        "hooks": {event: [{"hooks": [handler]}]}
    })
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).unwrap();
    true
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => true,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314) =>
        {
            false
        }
        Err(error) => panic!("failed to create test symlink: {error}"),
    }
}
