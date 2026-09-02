use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde_json::{Map, Value, json};

use crate::canonical::{canonical_json, sha256_hex};
use crate::cli::AdapterHost;
use crate::error::DevMapError;
use crate::events::{CaptureCapabilities, CaptureGrade, host_capabilities};
use crate::fs_security::{
    FileIdentity, checked_directory_identity, checked_file, checked_metadata, checked_new_file,
    ensure_directory, ensure_directory_chain, file_identity, sync_directory,
};
use crate::git::{SourceGitInspector, SourceWorkspace};

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
const OWNED_BINDING_PREFIX: &str = "devmap/v1/";
const KERNEL_COMMAND_PATH: &str = "devmap hook handle";
const GENERIC_DESCRIPTOR_PATH: &str = ".devmap/mcp.json";
const MAX_ADAPTER_CONFIG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanAction {
    Install,
    Uninstall,
}

impl PlanAction {
    fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Uninstall => "uninstall",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParentSnapshot {
    Absent,
    Directory(FileIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetSnapshot {
    Absent,
    File {
        identity: FileIdentity,
        bytes: Vec<u8>,
        unix_mode: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanSnapshot {
    source_identity: FileIdentity,
    parent: ParentSnapshot,
    target: TargetSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterPlan {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub bindings: Vec<HookBinding>,
    pub capabilities: CaptureCapabilities,
    pub capture_grade: CaptureGrade,
    pub plan_digest: String,
    action: PlanAction,
    source_root: PathBuf,
    git_dir: PathBuf,
    snapshot: PlanSnapshot,
    desired_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookBinding {
    pub binding_id: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub present: Vec<String>,
    pub missing: Vec<String>,
    pub modified: Vec<String>,
    pub kernel_command_path: String,
    pub capabilities: CaptureCapabilities,
    pub capture_grade: CaptureGrade,
    pub drift_reasons: Vec<String>,
    pub configured: bool,
    pub activation_verified: bool,
    pub activation_reasons: Vec<String>,
}

pub fn plan_adapter(source: &Path, host: AdapterHost) -> Result<AdapterPlan, DevMapError> {
    build_plan(source, host, PlanAction::Install)
}

pub fn plan_uninstall_adapter(
    source: &Path,
    host: AdapterHost,
) -> Result<AdapterPlan, DevMapError> {
    build_plan(source, host, PlanAction::Uninstall)
}

pub fn install_adapter(
    plan: AdapterPlan,
    approval_token: &str,
) -> Result<InstallReport, DevMapError> {
    execute_plan(plan, approval_token, PlanAction::Install)
}

pub fn verify_adapter(source: &Path, host: AdapterHost) -> Result<VerifyReport, DevMapError> {
    let plan = plan_adapter(source, host)?;
    let mut present = Vec::new();
    let mut missing = Vec::new();
    let mut modified = Vec::new();
    if host == AdapterHost::GenericMcp {
        match target_bytes(&plan.snapshot.target) {
            None => missing.push("descriptor".into()),
            Some(bytes) => {
                let document = parse_generic_document(&plan.config_path, bytes)?;
                if document == generic_descriptor() {
                    present.push("descriptor".into());
                } else {
                    modified.push("descriptor".into());
                }
            }
        }
    } else {
        let document = parse_native_snapshot(&plan)?;
        let occurrences = binding_occurrences(&document)?;
        for binding in &plan.bindings {
            let matching = occurrences
                .iter()
                .filter(|occurrence| occurrence.binding_id == binding.binding_id)
                .collect::<Vec<_>>();
            if matching.is_empty() {
                missing.push(binding.binding_id.clone());
            } else if matching.len() == 1 && occurrence_matches(matching[0], binding) {
                present.push(binding.binding_id.clone());
            } else {
                modified.push(binding.binding_id.clone());
            }
        }
    }

    let mut drift_reasons = Vec::new();
    if !missing.is_empty() {
        drift_reasons.push(format!("missing bindings: {}", missing.join(", ")));
    }
    if !modified.is_empty() {
        drift_reasons.push(format!("modified bindings: {}", modified.join(", ")));
    }
    let configured = missing.is_empty() && modified.is_empty();
    let mut activation_reasons = Vec::new();
    if !executable_reachable("devmap") {
        activation_reasons.push("devmap executable reachability is unresolved".into());
    }
    if matches!(host, AdapterHost::Codex | AdapterHost::Claude) {
        activation_reasons
            .push("host trust and managed-policy permission are not runtime-verifiable".into());
    } else {
        activation_reasons.push("generic MCP host registration is not runtime-verifiable".into());
    }
    let activation_verified = configured && activation_reasons.is_empty();

    Ok(VerifyReport {
        host: plan.host,
        config_path: plan.config_path,
        present,
        missing,
        modified,
        kernel_command_path: if host == AdapterHost::GenericMcp {
            "devmap mcp".into()
        } else {
            KERNEL_COMMAND_PATH.into()
        },
        capabilities: plan.capabilities,
        capture_grade: plan.capture_grade,
        drift_reasons,
        configured,
        activation_verified,
        activation_reasons,
    })
}

pub fn uninstall_adapter(
    plan: AdapterPlan,
    approval_token: &str,
) -> Result<InstallReport, DevMapError> {
    execute_plan(plan, approval_token, PlanAction::Uninstall)
}

fn host_details(host: AdapterHost) -> (&'static str, &'static str) {
    match host {
        AdapterHost::Codex => ("codex", ".codex/hooks.json"),
        AdapterHost::Claude => ("claude", ".claude/settings.json"),
        AdapterHost::GenericMcp => ("generic-mcp", GENERIC_DESCRIPTOR_PATH),
    }
}

struct AdapterInstallLock(File);

impl Drop for AdapterInstallLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn build_plan(
    source: &Path,
    host: AdapterHost,
    action: PlanAction,
) -> Result<AdapterPlan, DevMapError> {
    let workspace = SourceGitInspector::open(source)?.workspace()?;
    let (host_name, relative_config) = host_details(host);
    let config_path = workspace.root.join(relative_config);
    let source_identity = checked_directory_identity(&workspace.root)?;
    let snapshot = PlanSnapshot {
        source_identity,
        ..snapshot_target(&workspace.root, &config_path)?
    };
    let bindings = if host == AdapterHost::GenericMcp {
        Vec::new()
    } else {
        EVENTS
            .iter()
            .map(|event| {
                let binding_id = format!("devmap/v1/{host_name}/{event}");
                HookBinding {
                    binding_id: binding_id.clone(),
                    event: (*event).to_owned(),
                    matcher: None,
                    command: format!(
                        "devmap hook handle --host {host_name} --event {event} --binding-id {binding_id}"
                    ),
                }
            })
            .collect::<Vec<_>>()
    };
    let desired_bytes = desired_result(host, action, &config_path, &bindings, &snapshot.target)?;
    let capabilities = host_capabilities(host);
    let capture_grade = capabilities.grade();
    let plan_digest = plan_digest(
        host,
        action,
        &workspace,
        &config_path,
        &snapshot,
        desired_bytes.as_deref(),
    )?;
    Ok(AdapterPlan {
        host,
        config_path,
        bindings,
        capabilities,
        capture_grade,
        plan_digest,
        action,
        source_root: workspace.root,
        git_dir: workspace.git_dir,
        snapshot,
        desired_bytes,
    })
}

fn snapshot_target(root: &Path, path: &Path) -> Result<PlanSnapshot, DevMapError> {
    if path.parent().and_then(Path::parent) != Some(root) {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    let source_identity = checked_directory_identity(root)?;
    let parent = path
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    let parent_snapshot = match checked_metadata(parent)? {
        None => ParentSnapshot::Absent,
        Some(metadata) if metadata.is_dir() => {
            ParentSnapshot::Directory(checked_directory_identity(parent)?)
        }
        Some(_) => return Err(DevMapError::UnsafeInstallerOverwrite(parent.to_path_buf())),
    };
    let target = if matches!(parent_snapshot, ParentSnapshot::Absent) {
        if checked_metadata(path)?.is_some() {
            return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
        }
        TargetSnapshot::Absent
    } else {
        read_target(path)?
    };
    match &parent_snapshot {
        ParentSnapshot::Absent if checked_metadata(parent)?.is_some() => {
            return Err(DevMapError::UnsafeInstallerOverwrite(parent.to_path_buf()));
        }
        ParentSnapshot::Directory(identity) if checked_directory_identity(parent)? != *identity => {
            return Err(DevMapError::UnsafeInstallerOverwrite(parent.to_path_buf()));
        }
        _ => {}
    }
    if checked_directory_identity(root)? != source_identity {
        return Err(DevMapError::UnsafeInstallerOverwrite(root.to_path_buf()));
    }
    Ok(PlanSnapshot {
        source_identity,
        parent: parent_snapshot,
        target,
    })
}

fn read_target(path: &Path) -> Result<TargetSnapshot, DevMapError> {
    let Some(metadata) = checked_metadata(path)? else {
        return Ok(TargetSnapshot::Absent);
    };
    if !metadata.is_file() {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    if metadata.len() > MAX_ADAPTER_CONFIG_BYTES as u64 {
        return Err(DevMapError::ResourceLimit {
            resource: "adapter configuration",
            limit: MAX_ADAPTER_CONFIG_BYTES,
        });
    }
    let mut file = checked_file(path, false, false)?;
    let identity = file_identity(&file)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAX_ADAPTER_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ADAPTER_CONFIG_BYTES {
        return Err(DevMapError::ResourceLimit {
            resource: "adapter configuration",
            limit: MAX_ADAPTER_CONFIG_BYTES,
        });
    }
    if file_identity(&file)? != identity {
        return Err(DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()));
    }
    #[cfg(unix)]
    let unix_mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(file.metadata()?.permissions().mode())
    };
    #[cfg(not(unix))]
    let unix_mode = None;
    Ok(TargetSnapshot::File {
        identity,
        bytes,
        unix_mode,
    })
}

fn desired_result(
    host: AdapterHost,
    action: PlanAction,
    path: &Path,
    bindings: &[HookBinding],
    target: &TargetSnapshot,
) -> Result<Option<Vec<u8>>, DevMapError> {
    if host == AdapterHost::GenericMcp {
        return desired_generic(action, path, target);
    }
    let mut document = match target_bytes(target) {
        Some(bytes) => parse_native_document(path, bytes, host)?,
        None => json!({}),
    };
    match action {
        PlanAction::Install => {
            let occurrences = binding_occurrences(&document)?;
            let all_exact = bindings.iter().all(|binding| {
                let matching = occurrences
                    .iter()
                    .filter(|occurrence| occurrence.binding_id == binding.binding_id)
                    .collect::<Vec<_>>();
                matching.len() == 1 && occurrence_matches(matching[0], binding)
            });
            if !all_exact {
                remove_owned_bindings(&mut document, bindings)?;
                for binding in bindings {
                    append_binding(&mut document, binding)?;
                }
            }
        }
        PlanAction::Uninstall => {
            remove_owned_bindings(&mut document, bindings)?;
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    bytes.push(b'\n');
    if target_bytes(target).is_some_and(|prior| prior == bytes) {
        Ok(Some(prior_bytes(target).to_vec()))
    } else {
        Ok(Some(bytes))
    }
}

fn desired_generic(
    action: PlanAction,
    path: &Path,
    target: &TargetSnapshot,
) -> Result<Option<Vec<u8>>, DevMapError> {
    let existing = match target_bytes(target) {
        Some(bytes) => Some(parse_generic_document(path, bytes)?),
        None => None,
    };
    match action {
        PlanAction::Install => {
            let mut desired = serde_json::to_vec_pretty(&generic_descriptor())?;
            desired.push(b'\n');
            if existing.as_ref() == Some(&generic_descriptor()) {
                Ok(Some(prior_bytes(target).to_vec()))
            } else {
                Ok(Some(desired))
            }
        }
        PlanAction::Uninstall if existing.as_ref() == Some(&generic_descriptor()) => Ok(None),
        PlanAction::Uninstall => Ok(target_bytes(target).map(ToOwned::to_owned)),
    }
}

fn parse_native_snapshot(plan: &AdapterPlan) -> Result<Value, DevMapError> {
    match target_bytes(&plan.snapshot.target) {
        Some(bytes) => parse_native_document(&plan.config_path, bytes, plan.host),
        None => Ok(json!({})),
    }
}

fn parse_native_document(
    path: &Path,
    bytes: &[u8],
    host: AdapterHost,
) -> Result<Value, DevMapError> {
    let document: Value = serde_json::from_slice(bytes).map_err(|error| {
        DevMapError::MalformedAdapterConfig(format!("{}: {error}", path.display()))
    })?;
    validate_document(&document, path, host)?;
    Ok(document)
}

fn parse_generic_document(path: &Path, bytes: &[u8]) -> Result<Value, DevMapError> {
    let document: Value = serde_json::from_slice(bytes).map_err(|error| {
        DevMapError::MalformedAdapterConfig(format!("{}: {error}", path.display()))
    })?;
    let Some(object) = document.as_object() else {
        return Err(malformed(path, "top level must be an object"));
    };
    if !object.get("command").is_some_and(|command| {
        command.as_array().is_some_and(|arguments| {
            !arguments.is_empty() && arguments.iter().all(Value::is_string)
        })
    }) || !object.get("transport").is_some_and(Value::is_string)
    {
        return Err(malformed(
            path,
            "Generic MCP command must be a non-empty string array and transport must be a string",
        ));
    }
    Ok(document)
}

fn generic_descriptor() -> Value {
    json!({
        "command": ["devmap", "mcp", "--source", "."],
        "transport": "stdio"
    })
}

fn target_bytes(target: &TargetSnapshot) -> Option<&[u8]> {
    match target {
        TargetSnapshot::Absent => None,
        TargetSnapshot::File { bytes, .. } => Some(bytes),
    }
}

fn prior_bytes(target: &TargetSnapshot) -> &[u8] {
    target_bytes(target).expect("prior bytes requested only for an existing target")
}

fn plan_digest(
    host: AdapterHost,
    action: PlanAction,
    workspace: &SourceWorkspace,
    config_path: &Path,
    snapshot: &PlanSnapshot,
    desired: Option<&[u8]>,
) -> Result<String, DevMapError> {
    let relative = config_path
        .strip_prefix(&workspace.root)
        .map_err(|_| DevMapError::UnsafeInstallerOverwrite(config_path.to_path_buf()))?;
    let prior = match &snapshot.target {
        TargetSnapshot::Absent => json!({"state": "absent"}),
        TargetSnapshot::File {
            identity,
            bytes,
            unix_mode,
        } => json!({
            "state": "file",
            "identity": identity.stable_text(),
            "bytes_hex": hex_bytes(bytes),
            "unix_mode": unix_mode,
        }),
    };
    let parent = match &snapshot.parent {
        ParentSnapshot::Absent => json!({"state": "absent"}),
        ParentSnapshot::Directory(identity) => {
            json!({"state": "directory", "identity": identity.stable_text()})
        }
    };
    let digest_input = json!({
        "schema": "devmap/adapter-plan/1",
        "action": action.name(),
        "host": host_name(host),
        "source": {
            "root": workspace.root.to_string_lossy(),
            "identity": snapshot.source_identity.stable_text(),
        },
        "target": relative.to_string_lossy(),
        "parent": parent,
        "prior": prior,
        "desired": desired.map(hex_bytes),
    });
    Ok(format!(
        "sha256-{}",
        sha256_hex(&canonical_json(&digest_input)?)
    ))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn execute_plan(
    plan: AdapterPlan,
    approval_token: &str,
    expected_action: PlanAction,
) -> Result<InstallReport, DevMapError> {
    if plan.action != expected_action || approval_token != plan.plan_digest {
        return Err(DevMapError::AdapterApprovalMismatch);
    }
    let _lock = acquire_install_lock(&plan.git_dir)?;
    let current = build_plan(&plan.source_root, plan.host, plan.action)?;
    if current != plan {
        return Err(DevMapError::AdapterPlanStale(format!(
            "{} changed after review",
            plan.config_path.display()
        )));
    }
    let report = planned_report(&plan)?;
    if report.changed {
        commit_plan(&plan)?;
    }
    Ok(report)
}

fn acquire_install_lock(git_dir: &Path) -> Result<AdapterInstallLock, DevMapError> {
    let directory = ensure_directory_chain(git_dir, &["devmap"])?;
    let path = directory.join("adapter-install.lock");
    let existed = checked_metadata(&path)?.is_some();
    let file = checked_file(&path, true, true)?;
    if !existed {
        sync_directory(&directory)?;
    }
    file.lock_exclusive()?;
    Ok(AdapterInstallLock(file))
}

fn planned_report(plan: &AdapterPlan) -> Result<InstallReport, DevMapError> {
    let changed = match (&plan.snapshot.target, &plan.desired_bytes) {
        (TargetSnapshot::Absent, None) => false,
        (TargetSnapshot::File { bytes, .. }, Some(desired)) => bytes != desired,
        _ => true,
    };
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut unchanged = Vec::new();
    if plan.host == AdapterHost::GenericMcp {
        match plan.action {
            PlanAction::Install if changed => added.push("descriptor".into()),
            PlanAction::Install => unchanged.push("descriptor".into()),
            PlanAction::Uninstall if changed => removed.push("descriptor".into()),
            PlanAction::Uninstall => {}
        }
    } else {
        let document = parse_native_snapshot(plan)?;
        let existing = binding_occurrences(&document)?
            .into_iter()
            .map(|occurrence| occurrence.binding_id.to_owned())
            .collect::<BTreeSet<_>>();
        match plan.action {
            PlanAction::Install => {
                for binding in &plan.bindings {
                    if existing.contains(&binding.binding_id) {
                        unchanged.push(binding.binding_id.clone());
                    } else {
                        added.push(binding.binding_id.clone());
                    }
                }
            }
            PlanAction::Uninstall => {
                let mut copy = document;
                removed = remove_owned_bindings(&mut copy, &plan.bindings)?;
            }
        }
    }
    Ok(InstallReport {
        host: plan.host,
        config_path: plan.config_path.clone(),
        added,
        removed,
        unchanged,
        changed,
    })
}

fn commit_plan(plan: &AdapterPlan) -> Result<(), DevMapError> {
    if checked_directory_identity(&plan.source_root)? != plan.snapshot.source_identity {
        return Err(DevMapError::AdapterPlanStale(
            "source repository identity changed".into(),
        ));
    }
    let parent = plan
        .config_path
        .parent()
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(plan.config_path.clone()))?;
    let expected_parent = match (&plan.snapshot.parent, &plan.desired_bytes) {
        (ParentSnapshot::Absent, Some(_)) => {
            ensure_directory(parent)?;
            sync_directory(&plan.source_root)?;
            ParentSnapshot::Directory(checked_directory_identity(parent)?)
        }
        (parent, _) => parent.clone(),
    };
    assert_snapshot(plan, &expected_parent)?;
    match &plan.desired_bytes {
        Some(bytes) => write_planned_config(plan, &expected_parent, bytes),
        None => remove_planned_config(plan, &expected_parent),
    }
}

fn assert_snapshot(
    plan: &AdapterPlan,
    expected_parent: &ParentSnapshot,
) -> Result<(), DevMapError> {
    let current = snapshot_target(&plan.source_root, &plan.config_path)?;
    if current.source_identity != plan.snapshot.source_identity
        || &current.parent != expected_parent
        || current.target != plan.snapshot.target
    {
        return Err(DevMapError::AdapterPlanStale(format!(
            "{} changed at commit time",
            plan.config_path.display()
        )));
    }
    Ok(())
}

fn write_planned_config(
    plan: &AdapterPlan,
    expected_parent: &ParentSnapshot,
    bytes: &[u8],
) -> Result<(), DevMapError> {
    let temporary = suffixed_path(&plan.config_path, ".devmap-tmp")?;
    let backup = suffixed_path(&plan.config_path, ".devmap-backup")?;
    for artifact in [&temporary, &backup] {
        if checked_metadata(artifact)?.is_some() {
            return Err(DevMapError::UnsafeInstallerOverwrite(artifact.clone()));
        }
    }
    let mut file = checked_new_file(&temporary)?;
    #[cfg(unix)]
    if let TargetSnapshot::File {
        unix_mode: Some(mode),
        ..
    } = &plan.snapshot.target
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(*mode))?;
    }
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        return Err(transaction_error(&plan.config_path, error, &temporary));
    }
    drop(file);
    if let Err(error) = assert_snapshot(plan, expected_parent) {
        let _ = remove_temporary_file(&temporary);
        return Err(error);
    }

    let result = match &plan.snapshot.target {
        TargetSnapshot::Absent => commit_new_target(&temporary, &plan.config_path, bytes),
        TargetSnapshot::File { .. } => commit_replacement(
            &plan.config_path,
            &temporary,
            &backup,
            bytes,
            &plan.snapshot.target,
        ),
    };
    if let Err(error) = result {
        return Err(transaction_error(&plan.config_path, error, &temporary));
    }
    sync_directory(
        plan.config_path
            .parent()
            .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(plan.config_path.clone()))?,
    )?;
    Ok(())
}

fn commit_new_target(temporary: &Path, path: &Path, expected: &[u8]) -> std::io::Result<()> {
    fs::hard_link(temporary, path)?;
    match fs::read(path) {
        Ok(actual) if actual == expected => {}
        Ok(_) => {
            return Err(std::io::Error::other(
                "named adapter config did not match reviewed bytes",
            ));
        }
        Err(error) => return Err(error),
    }
    fs::remove_file(temporary)
}

fn remove_planned_config(
    plan: &AdapterPlan,
    expected_parent: &ParentSnapshot,
) -> Result<(), DevMapError> {
    if matches!(plan.snapshot.target, TargetSnapshot::Absent) {
        return Ok(());
    }
    let backup = suffixed_path(&plan.config_path, ".devmap-backup")?;
    if checked_metadata(&backup)?.is_some() {
        return Err(DevMapError::UnsafeInstallerOverwrite(backup));
    }
    assert_snapshot(plan, expected_parent)?;
    move_noreplace(&plan.config_path, &backup)?;
    if let Err(error) = verify_snapshot_file(&backup, &plan.snapshot.target) {
        let _ = move_noreplace(&backup, &plan.config_path);
        return Err(error);
    }
    sync_directory(
        plan.config_path
            .parent()
            .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(plan.config_path.clone()))?,
    )?;
    // A second read narrows the unlocked-writer window and ensures late edits are preserved as
    // a recovery artifact instead of being silently discarded.
    verify_snapshot_file(&backup, &plan.snapshot.target)?;
    fs::remove_file(&backup)?;
    sync_directory(
        plan.config_path
            .parent()
            .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(plan.config_path.clone()))?,
    )
}

fn verify_snapshot_file(path: &Path, expected: &TargetSnapshot) -> Result<(), DevMapError> {
    let actual = read_target(path)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(DevMapError::AdapterPlanStale(format!(
            "{} changed during the transaction",
            path.display()
        )))
    }
}

#[cfg(windows)]
fn commit_replacement(
    path: &Path,
    temporary: &Path,
    backup: &Path,
    expected: &[u8],
    prior: &TargetSnapshot,
) -> std::io::Result<()> {
    windows_atomic_replace(temporary, path, backup, expected, prior)
}

#[cfg(target_os = "linux")]
fn commit_replacement(
    path: &Path,
    temporary: &Path,
    backup: &Path,
    expected: &[u8],
    prior: &TargetSnapshot,
) -> std::io::Result<()> {
    move_noreplace(path, backup)?;
    if let Err(error) = verify_snapshot_file(backup, prior) {
        let _ = move_noreplace(backup, path);
        return Err(std::io::Error::other(error.to_string()));
    }
    if let Err(error) = move_noreplace(temporary, path) {
        let _ = move_noreplace(backup, path);
        return Err(error);
    }
    match fs::read(path) {
        Ok(actual) if actual == expected => {}
        Ok(_) => return Err(std::io::Error::other("replacement read-back mismatch")),
        Err(error) => return Err(error),
    }
    if let Err(error) = verify_snapshot_file(backup, prior) {
        return Err(std::io::Error::other(error.to_string()));
    }
    fs::remove_file(backup)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn commit_replacement(
    _path: &Path,
    _temporary: &Path,
    _backup: &Path,
    _expected: &[u8],
    _prior: &TargetSnapshot,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe no-replace adapter transactions are unsupported on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn move_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // SAFETY: both C strings are NUL-terminated and live for the duration of the syscall.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn move_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: the paths are owned, NUL-terminated UTF-16 buffers.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn move_noreplace(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe no-replace adapter transactions are unsupported on this platform",
    ))
}

fn executable_reachable(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    executable_reachable_on_path(name, &path)
}

fn executable_reachable_on_path(name: &str, search_path: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    let candidates = [format!("{name}.exe")];
    #[cfg(not(windows))]
    let candidates = [name.to_owned()];
    std::env::split_paths(search_path).any(|directory| {
        candidates
            .iter()
            .any(|candidate| executable_candidate(&directory.join(candidate)))
    })
}

#[cfg(windows)]
fn executable_candidate(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn executable_candidate(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(any(unix, windows)))]
fn executable_candidate(_path: &Path) -> bool {
    false
}

fn host_name(host: AdapterHost) -> &'static str {
    match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => "generic-mcp",
    }
}

fn validate_document(document: &Value, path: &Path, host: AdapterHost) -> Result<(), DevMapError> {
    let root = document
        .as_object()
        .ok_or_else(|| malformed(path, "top level must be an object"))?;
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object()
        .ok_or_else(|| malformed(path, "hooks must be an object"))?;
    for (event, groups) in hooks {
        validate_event(path, host, event)?;
        let groups = groups
            .as_array()
            .ok_or_else(|| malformed(path, format!("hooks.{event} must be an array")))?;
        for (group_index, group) in groups.iter().enumerate() {
            let group = group.as_object().ok_or_else(|| {
                malformed(
                    path,
                    format!("hooks.{event}[{group_index}] must be an object"),
                )
            })?;
            if group
                .get("matcher")
                .is_some_and(|matcher| !matcher.is_string())
            {
                return Err(malformed(
                    path,
                    format!("hooks.{event}[{group_index}].matcher must be a string"),
                ));
            }
            let handlers = group
                .get("hooks")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    malformed(
                        path,
                        format!("hooks.{event}[{group_index}].hooks must be an array"),
                    )
                })?;
            for (handler_index, handler) in handlers.iter().enumerate() {
                validate_handler(path, host, event, group_index, handler_index, handler)?;
            }
        }
    }
    Ok(())
}

fn validate_handler(
    path: &Path,
    host: AdapterHost,
    event: &str,
    group_index: usize,
    handler_index: usize,
    handler: &Value,
) -> Result<(), DevMapError> {
    let location = format!("hooks.{event}[{group_index}].hooks[{handler_index}]");
    let handler = handler
        .as_object()
        .ok_or_else(|| malformed(path, format!("{location} must be an object")))?;
    let handler_type = handler
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(path, format!("{location}.type must be a string")))?;
    if !handler_type_supported(host, event, handler_type) {
        return Err(malformed(
            path,
            format!("{location}.type is not supported for {host:?} {event}: {handler_type}"),
        ));
    }
    let required_strings: &[&str] = match handler_type {
        "command" => &["command"],
        "http" => &["url"],
        "mcp_tool" => &["server", "tool"],
        "prompt" | "agent" => &["prompt"],
        _ => unreachable!("unsupported handler types are rejected above"),
    };
    for field in required_strings {
        if !handler.get(*field).is_some_and(Value::is_string) {
            return Err(malformed(
                path,
                format!("{location}.{field} must be a string"),
            ));
        }
    }
    validate_optional_string(path, handler, &location, "statusMessage")?;
    validate_optional_number(path, handler, &location, "timeout")?;
    match host {
        AdapterHost::Codex => validate_codex_handler(path, handler, &location, handler_type),
        AdapterHost::Claude => validate_claude_handler(path, handler, &location, handler_type),
        AdapterHost::GenericMcp => unreachable!("generic MCP has no native adapter config"),
    }
}

fn validate_event(path: &Path, host: AdapterHost, event: &str) -> Result<(), DevMapError> {
    let supported = match host {
        AdapterHost::Codex => matches!(
            event,
            "SessionStart"
                | "UserPromptSubmit"
                | "PreToolUse"
                | "PermissionRequest"
                | "PostToolUse"
                | "PreCompact"
                | "PostCompact"
                | "SubagentStart"
                | "SubagentStop"
                | "Stop"
                | "SessionEnd"
        ),
        AdapterHost::Claude => matches!(
            event,
            "Setup"
                | "SessionStart"
                | "UserPromptSubmit"
                | "UserPromptExpansion"
                | "PreToolUse"
                | "PermissionRequest"
                | "PermissionDenied"
                | "PostToolUse"
                | "PostToolUseFailure"
                | "PostToolBatch"
                | "SubagentStart"
                | "SubagentStop"
                | "TaskCreated"
                | "TaskCompleted"
                | "Stop"
                | "StopFailure"
                | "TeammateIdle"
                | "PreCompact"
                | "PostCompact"
                | "SessionEnd"
                | "Elicitation"
                | "ElicitationResult"
                | "WorktreeCreate"
                | "WorktreeRemove"
                | "Notification"
                | "ConfigChange"
                | "InstructionsLoaded"
                | "CwdChanged"
                | "FileChanged"
                | "DirectoryAdded"
                | "PreModelSwitch"
                | "PostModelSwitch"
                | "MessageDisplay"
        ),
        AdapterHost::GenericMcp => false,
    };
    if supported {
        Ok(())
    } else {
        Err(malformed(
            path,
            format!("hooks.{event} is not supported for {host:?}"),
        ))
    }
}

fn handler_type_supported(host: AdapterHost, event: &str, handler_type: &str) -> bool {
    match host {
        AdapterHost::Codex => {
            handler_type == "command" || (handler_type == "mcp_tool" && event != "SessionEnd")
        }
        AdapterHost::Claude => match event {
            "SessionStart" | "Setup" => matches!(handler_type, "command" | "mcp_tool"),
            "PermissionDenied"
            | "PermissionRequest"
            | "PostToolBatch"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "PreToolUse"
            | "Stop"
            | "SubagentStop"
            | "TaskCompleted"
            | "TaskCreated"
            | "TeammateIdle"
            | "UserPromptExpansion"
            | "UserPromptSubmit" => matches!(
                handler_type,
                "command" | "http" | "mcp_tool" | "prompt" | "agent"
            ),
            _ => matches!(handler_type, "command" | "http" | "mcp_tool"),
        },
        AdapterHost::GenericMcp => false,
    }
}

fn validate_codex_handler(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
) -> Result<(), DevMapError> {
    const KNOWN_FIELDS: &[&str] = &[
        "command",
        "commandWindows",
        "additionalContextLimit",
        "async",
        "server",
        "tool",
        "input",
    ];
    match handler_type {
        "command" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &[
                    "command",
                    "commandWindows",
                    "additionalContextLimit",
                    "async",
                ],
            )?;
            validate_optional_string(path, handler, location, "commandWindows")?;
            validate_optional_bool(path, handler, location, "async")?;
            validate_optional_unsigned_integer(path, handler, location, "additionalContextLimit")
        }
        "mcp_tool" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["server", "tool", "input"],
            )?;
            validate_optional_object(path, handler, location, "input")
        }
        _ => unreachable!("Codex handler type was validated"),
    }
}

fn validate_claude_handler(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
) -> Result<(), DevMapError> {
    const KNOWN_FIELDS: &[&str] = &[
        "command",
        "args",
        "async",
        "asyncRewake",
        "shell",
        "url",
        "headers",
        "allowedEnvVars",
        "server",
        "tool",
        "input",
        "prompt",
        "model",
        "continueOnBlock",
    ];
    validate_optional_string(path, handler, location, "if")?;
    validate_optional_bool(path, handler, location, "once")?;
    match handler_type {
        "command" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["command", "args", "async", "asyncRewake", "shell"],
            )?;
            validate_optional_string_array(path, handler, location, "args")?;
            validate_optional_bool(path, handler, location, "async")?;
            validate_optional_bool(path, handler, location, "asyncRewake")?;
            validate_optional_shell(path, handler, location)
        }
        "http" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["url", "headers", "allowedEnvVars"],
            )?;
            validate_optional_string_map(path, handler, location, "headers")?;
            validate_optional_string_array(path, handler, location, "allowedEnvVars")
        }
        "mcp_tool" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["server", "tool", "input"],
            )?;
            validate_optional_object(path, handler, location, "input")
        }
        "prompt" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["prompt", "model", "continueOnBlock"],
            )?;
            validate_optional_string(path, handler, location, "model")?;
            validate_optional_bool(path, handler, location, "continueOnBlock")
        }
        "agent" => {
            validate_allowed_type_fields(
                path,
                handler,
                location,
                handler_type,
                KNOWN_FIELDS,
                &["prompt", "model"],
            )?;
            validate_optional_string(path, handler, location, "model")
        }
        _ => unreachable!("Claude handler type was validated"),
    }
}

fn validate_allowed_type_fields(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    handler_type: &str,
    known_fields: &[&str],
    allowed_fields: &[&str],
) -> Result<(), DevMapError> {
    if let Some(field) = known_fields
        .iter()
        .find(|field| handler.contains_key(**field) && !allowed_fields.contains(field))
    {
        Err(malformed(
            path,
            format!("{location}.{field} is not supported for type {handler_type}"),
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_shell(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        "shell",
        "bash or powershell",
        |value| matches!(value.as_str(), Some("bash" | "powershell")),
    )
}

fn validate_optional(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
    expected: &str,
    predicate: impl FnOnce(&Value) -> bool,
) -> Result<(), DevMapError> {
    if handler.get(field).is_none_or(predicate) {
        Ok(())
    } else {
        Err(malformed(
            path,
            format!("{location}.{field} must be {expected}"),
        ))
    }
}

fn validate_optional_string(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(path, handler, location, field, "a string", Value::is_string)
}

fn validate_optional_number(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(path, handler, location, field, "a number", Value::is_number)
}

fn validate_optional_bool(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "a boolean",
        Value::is_boolean,
    )
}

fn validate_optional_object(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an object",
        Value::is_object,
    )
}

fn validate_optional_unsigned_integer(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "a non-negative integer",
        |value| value.as_u64().is_some(),
    )
}

fn validate_optional_string_array(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an array of strings",
        |value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
        },
    )
}

fn validate_optional_string_map(
    path: &Path,
    handler: &Map<String, Value>,
    location: &str,
    field: &str,
) -> Result<(), DevMapError> {
    validate_optional(
        path,
        handler,
        location,
        field,
        "an object of strings",
        |value| {
            value
                .as_object()
                .is_some_and(|entries| entries.values().all(Value::is_string))
        },
    )
}

fn malformed(path: &Path, reason: impl AsRef<str>) -> DevMapError {
    DevMapError::MalformedAdapterConfig(format!("{}: {}", path.display(), reason.as_ref()))
}

fn append_binding(document: &mut Value, binding: &HookBinding) -> Result<(), DevMapError> {
    let root = document
        .as_object_mut()
        .expect("validated adapter document is an object");
    if !root.contains_key("hooks") {
        root.insert("hooks".into(), Value::Object(Map::new()));
    }
    let hooks = root["hooks"]
        .as_object_mut()
        .expect("validated hooks value is an object");
    if !hooks.contains_key(&binding.event) {
        hooks.insert(binding.event.clone(), Value::Array(Vec::new()));
    }
    let groups = hooks[&binding.event]
        .as_array_mut()
        .expect("validated hook event is an array");
    groups.push(expected_group(binding));
    Ok(())
}

fn expected_group(binding: &HookBinding) -> Value {
    let mut group = Map::new();
    if let Some(matcher) = &binding.matcher {
        group.insert("matcher".into(), Value::String(matcher.clone()));
    }
    group.insert(
        "hooks".into(),
        Value::Array(vec![expected_handler(binding)]),
    );
    Value::Object(group)
}

fn expected_handler(binding: &HookBinding) -> Value {
    json!({
        "type": "command",
        "command": binding.command,
        "statusMessage": "Recording DevMap lifecycle",
    })
}

#[derive(Debug)]
struct BindingOccurrence<'a> {
    binding_id: &'a str,
    event: &'a str,
    matcher: Option<&'a str>,
    handler: &'a Value,
}

fn binding_occurrences(document: &Value) -> Result<Vec<BindingOccurrence<'_>>, DevMapError> {
    let Some(hooks) = document.get("hooks").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut occurrences = Vec::new();
    for (event, groups) in hooks {
        for group in groups.as_array().expect("validated hook event is an array") {
            let matcher = group.get("matcher").and_then(Value::as_str);
            for handler in group["hooks"]
                .as_array()
                .expect("validated handler list is an array")
            {
                if let Some(binding_id) = handler_binding_id(handler) {
                    occurrences.push(BindingOccurrence {
                        binding_id,
                        event,
                        matcher,
                        handler,
                    });
                }
            }
        }
    }
    Ok(occurrences)
}

fn occurrence_matches(occurrence: &BindingOccurrence<'_>, binding: &HookBinding) -> bool {
    occurrence.event == binding.event
        && occurrence.matcher == binding.matcher.as_deref()
        && occurrence.handler == &expected_handler(binding)
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
        if let Some(binding_id) = word.strip_prefix("--binding-id=") {
            return Some(binding_id);
        }
    }
    None
}

fn remove_owned_bindings(
    document: &mut Value,
    bindings: &[HookBinding],
) -> Result<Vec<String>, DevMapError> {
    let Some(hooks) = document.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(Vec::new());
    };
    let mut removed = Vec::new();
    let owned_ids = bindings
        .iter()
        .map(|binding| binding.binding_id.as_str())
        .collect::<BTreeSet<_>>();
    let events = hooks.keys().cloned().collect::<Vec<_>>();
    for event in events {
        let groups = hooks[&event]
            .as_array_mut()
            .expect("validated hook event is an array");
        let mut removed_from_event = false;
        groups.retain_mut(|group| {
            let devmap_generated = bindings
                .iter()
                .filter(|binding| binding.event == event)
                .any(|binding| group == &expected_group(binding));
            let handlers = group["hooks"]
                .as_array_mut()
                .expect("validated handler list is an array");
            let mut removed_from_group = false;
            handlers.retain(|handler| {
                let Some(binding_id) = handler_binding_id(handler) else {
                    return true;
                };
                if !binding_id.starts_with(OWNED_BINDING_PREFIX) || !owned_ids.contains(binding_id)
                {
                    return true;
                }
                removed.push(binding_id.to_owned());
                removed_from_group = true;
                removed_from_event = true;
                false
            });
            !(devmap_generated && removed_from_group && handlers.is_empty())
        });
        if removed_from_event && groups.is_empty() {
            hooks.remove(&event);
        }
    }
    removed.sort();
    Ok(removed)
}

fn suffixed_path(path: &Path, suffix: &str) -> Result<PathBuf, DevMapError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(path.to_path_buf()))?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}

fn transaction_error(
    path: &Path,
    operation_error: std::io::Error,
    temporary: &Path,
) -> DevMapError {
    let cleanup = match remove_temporary_file(temporary) {
        Ok(()) => "succeeded".to_owned(),
        Err(cleanup_error) => format!("failed: {cleanup_error}"),
    };
    DevMapError::AdapterConfigTransaction {
        path: path.to_path_buf(),
        operation_error: operation_error.to_string(),
        cleanup,
    }
}

fn remove_temporary_file(temporary: &Path) -> std::io::Result<()> {
    if !temporary
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".devmap-tmp"))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to clean a non-DevMap temporary path",
        ));
    }
    match fs::symlink_metadata(temporary) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(temporary)
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "DevMap temporary path is not a file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(any(windows, test))]
fn finalize_verified_replacement(
    path: &Path,
    expected: &[u8],
    restore: impl FnOnce() -> std::io::Result<()>,
    cleanup_backup: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let verification_error = match fs::read(path) {
        Ok(actual) if actual == expected => None,
        Ok(_) => Some(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "named adapter config did not match the serialized bytes",
        )),
        Err(error) => Some(std::io::Error::other(format!(
            "reading back the named adapter config failed: {error}"
        ))),
    };
    if let Some(verification_error) = verification_error {
        return match restore() {
            Ok(()) => Err(verification_error),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "{verification_error}; restoring the original failed: {restore_error}"
            ))),
        };
    }

    if let Err(cleanup_error) = cleanup_backup() {
        return match restore() {
            Ok(()) => Err(std::io::Error::other(format!(
                "cleaning replacement backup failed and the replacement was rolled back: {cleanup_error}"
            ))),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "cleaning replacement backup failed: {cleanup_error}; restoring the original failed: {restore_error}"
            ))),
        };
    }
    Ok(())
}

#[cfg(windows)]
fn windows_atomic_replace(
    temporary: &Path,
    path: &Path,
    backup: &Path,
    expected: &[u8],
    prior: &TargetSnapshot,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut core::ffi::c_void,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn move_file(source: &Path, destination: &Path, replace: bool) -> std::io::Result<()> {
        let source = wide(source);
        let destination = wide(destination);
        let flags = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        // SAFETY: both path pointers refer to owned NUL-terminated buffers.
        let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
        if moved == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    if !path.try_exists()? {
        move_file(temporary, path, false)?;
        return finalize_verified_replacement(
            path,
            expected,
            || move_file(path, temporary, false),
            || Ok(()),
        );
    }

    let temporary_wide = wide(temporary);
    let path_wide = wide(path);
    let backup_wide = wide(backup);
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain
    // alive for the call; the remaining optional pointers are documented null values.
    let replaced = unsafe {
        ReplaceFileW(
            path_wide.as_ptr(),
            temporary_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        let replace_error = std::io::Error::last_os_error();
        if backup.try_exists()?
            && let Err(restore_error) = move_file(backup, path, true)
        {
            return Err(std::io::Error::other(format!(
                "{replace_error}; restoring the original from {} failed: {restore_error}",
                backup.display()
            )));
        }
        return Err(replace_error);
    }

    if let Err(verification_error) = verify_snapshot_file(backup, prior) {
        return match move_file(backup, path, true) {
            Ok(()) => Err(std::io::Error::other(format!(
                "the replaced adapter config changed after review and was restored: {verification_error}"
            ))),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "the replaced adapter config changed after review: {verification_error}; restoring it failed: {restore_error}"
            ))),
        };
    }

    finalize_verified_replacement(
        path,
        expected,
        || move_file(backup, path, true),
        || {
            verify_snapshot_file(backup, prior)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            fs::remove_file(backup)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[cfg(windows)]
    #[test]
    fn executable_reachability_does_not_accept_a_bare_non_executable_file_on_windows() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("devmap"), b"not an executable").unwrap();
        let search_path = std::env::join_paths([root.path()]).unwrap();

        assert!(!executable_reachable_on_path("devmap", &search_path));

        fs::write(
            root.path().join("devmap.exe"),
            b"test executable placeholder",
        )
        .unwrap();
        assert!(executable_reachable_on_path("devmap", &search_path));
    }

    #[cfg(unix)]
    #[test]
    fn executable_reachability_requires_an_executable_mode_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("devmap");
        fs::write(&executable, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o600)).unwrap();
        let search_path = std::env::join_paths([root.path()]).unwrap();

        assert!(!executable_reachable_on_path("devmap", &search_path));

        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(executable_reachable_on_path("devmap", &search_path));
    }

    #[test]
    fn replacement_readback_mismatch_restores_before_backup_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::write(&path, b"unexpected replacement").unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                fs::write(&path, b"original")
            },
            || {
                cleanup_called.set(true);
                Ok(())
            },
        )
        .expect_err("a byte mismatch must fail the transaction");

        assert!(restored.get());
        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(error.to_string().contains("did not match"));
    }

    #[test]
    fn replacement_readback_failure_restores_before_backup_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::create_dir(&path).unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                fs::remove_dir(&path)?;
                fs::write(&path, b"original")
            },
            || {
                cleanup_called.set(true);
                Ok(())
            },
        )
        .expect_err("a read-back failure must fail the transaction");

        assert!(restored.get());
        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(error.to_string().contains("reading back"));
    }

    #[test]
    fn replacement_backup_cleanup_runs_only_after_exact_readback() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        fs::write(&path, b"expected replacement").unwrap();
        let restored = Cell::new(false);
        let cleanup_called = Cell::new(false);

        finalize_verified_replacement(
            &path,
            b"expected replacement",
            || {
                restored.set(true);
                Ok(())
            },
            || {
                assert_eq!(fs::read(&path).unwrap(), b"expected replacement");
                cleanup_called.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!restored.get());
        assert!(cleanup_called.get());
    }

    #[test]
    fn replacement_readback_restore_failure_preserves_the_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hooks.json");
        let backup = root.path().join("hooks.json.devmap-backup");
        fs::write(&path, b"unexpected replacement").unwrap();
        fs::write(&backup, b"original").unwrap();
        let cleanup_called = Cell::new(false);

        let error = finalize_verified_replacement(
            &path,
            b"expected replacement",
            || Err(std::io::Error::other("injected restore failure")),
            || {
                cleanup_called.set(true);
                fs::remove_file(&backup)
            },
        )
        .expect_err("a restore failure must propagate");

        assert!(!cleanup_called.get());
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert!(error.to_string().contains("injected restore failure"));
    }
}
