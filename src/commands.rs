use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CommandOutput;
use crate::adapter::{install_adapter, plan_adapter, uninstall_adapter, verify_adapter};
use crate::canonical::{canonical_json, sha256_hex};
use crate::cli::{
    AdapterHost, AdapterInstallArgs, AdapterPlanArgs, AdapterUninstallArgs, AdapterVerifyArgs,
    ApproveArgs, InitArgs, StatusArgs,
};
use crate::context::ContextRepo;
use crate::domain::{
    ApprovalEvent, CanonicalObjectRef, CommonGround, CommonGroundDraft, CommonGroundManifest,
    CurrentState, HistoricalScope, RequirementTrace, SCHEMA_VERSION,
};
use crate::error::DevMapError;
use crate::git::SourceGitInspector;

const DRAFT_PATH: &str = "bootstrap/common-ground-draft.json";
const BOOTSTRAP_BRANCH: &str = "bootstrap/initial";
const MANIFEST_PATH: &str = "manifests/common-ground.json";
const CURRENT_STATE_PATH: &str = "state/current.json";

pub fn adapter_plan(args: AdapterPlanArgs) -> Result<CommandOutput, DevMapError> {
    let plan = plan_adapter(&args.source, args.host)?;
    let mut stdout = format!(
        "host={}\nconfig_path={}\nkernel_command_path=devmap hook handle\ncapture_grade={}\n",
        host_name(plan.host),
        plan.config_path.display(),
        grade_name(plan.capture_grade)
    );
    for binding in plan.bindings {
        stdout.push_str(&format!(
            "binding={} event={} command={}\n",
            binding.binding_id, binding.event, binding.command
        ));
    }
    Ok(CommandOutput {
        stdout,
        exit_code: 0,
    })
}

pub fn adapter_install(args: AdapterInstallArgs) -> Result<CommandOutput, DevMapError> {
    let report = install_adapter(plan_adapter(&args.source, args.host)?)?;
    Ok(CommandOutput {
        stdout: format!(
            "host={}\nconfig_path={}\nchanged={}\nadded={}\n",
            host_name(report.host),
            report.config_path.display(),
            report.changed,
            report.added.join(",")
        ),
        exit_code: 0,
    })
}

pub fn adapter_verify(args: AdapterVerifyArgs) -> Result<CommandOutput, DevMapError> {
    let hosts = match args.host {
        Some(host) => vec![host],
        None => vec![AdapterHost::Codex, AdapterHost::Claude],
    };
    let mut stdout = String::new();
    let mut exit_code = 0;
    for host in hosts {
        let report = verify_adapter(&args.source, host)?;
        let capabilities = serde_json::to_string(&report.capabilities)?;
        stdout.push_str(&format!(
            "host={}\nconfig_path={}\nkernel_command_path={}\npresent={}\nmissing={}\nmodified={}\ncapabilities={}\ncapture_grade={}\ndrift_reason={}\n",
            host_name(report.host),
            report.config_path.display(),
            report.kernel_command_path,
            report.present.join(","),
            report.missing.join(","),
            report.modified.join(","),
            capabilities,
            grade_name(report.capture_grade),
            report.drift_reasons.join("; ")
        ));
        if report.capture_grade == crate::events::CaptureGrade::D {
            exit_code = 1;
        }
    }
    Ok(CommandOutput { stdout, exit_code })
}

pub fn adapter_uninstall(args: AdapterUninstallArgs) -> Result<CommandOutput, DevMapError> {
    let report = uninstall_adapter(&args.source, args.host)?;
    Ok(CommandOutput {
        stdout: format!(
            "host={}\nconfig_path={}\nchanged={}\nremoved={}\n",
            host_name(report.host),
            report.config_path.display(),
            report.changed,
            report.removed.join(",")
        ),
        exit_code: 0,
    })
}

fn host_name(host: AdapterHost) -> &'static str {
    match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => "generic-mcp",
    }
}

fn grade_name(grade: crate::events::CaptureGrade) -> &'static str {
    match grade {
        crate::events::CaptureGrade::A => "A",
        crate::events::CaptureGrade::B => "B",
        crate::events::CaptureGrade::C => "C",
        crate::events::CaptureGrade::D => "D",
    }
}

pub fn init(args: InitArgs) -> Result<CommandOutput, DevMapError> {
    let inspector = SourceGitInspector::open(&args.source)?;
    let source_root = inspector.root().to_path_buf();
    let context_root = absolute_candidate(&args.context)?;
    ensure_independent_repositories(&source_root, &context_root)?;

    let source = inspector.inspect()?;
    let requirements = args
        .requirement
        .iter()
        .map(|locator| load_requirement(&source_root, locator))
        .collect::<Result<Vec<_>, _>>()?;
    let created_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let proposed = CommonGroundDraft::new(created_at, source, args.goal, requirements)?;

    let context = if context_root.exists() {
        ContextRepo::open(&context_root)?
    } else {
        ContextRepo::create(&context_root)?
    };
    if context.read_owned(CURRENT_STATE_PATH)?.is_some() {
        return Err(DevMapError::CommonGroundAlreadyApproved);
    }
    prepare_bootstrap_branch(&context)?;

    let draft = match context.read_owned(DRAFT_PATH)? {
        Some(bytes) => {
            let existing: CommonGroundDraft = serde_json::from_slice(&bytes)?;
            if !same_draft_intent(&existing, &proposed) {
                return Err(DevMapError::ConflictingCommonGroundDraft);
            }
            existing
        }
        None => {
            let bytes = canonical_json(&proposed)?;
            context.write_owned(DRAFT_PATH, &bytes)?;
            context.commit_all("Draft initial DevMap Common Ground")?;
            proposed
        }
    };

    let draft_bytes = canonical_json(&draft)?;
    let draft_sha256 = sha256_hex(&draft_bytes);
    Ok(CommandOutput {
        stdout: format!(
            "common_ground=draft\ndraft_sha256={draft_sha256}\nadoption_boundary_commit={}\ndirty_at_adoption={}\napprove=devmap common-ground approve --context \"{}\" --actor <human>\n",
            draft.source.head_commit,
            draft.source.dirty_at_adoption,
            context.root().display()
        ),
        exit_code: 0,
    })
}

pub fn approve(args: ApproveArgs) -> Result<CommandOutput, DevMapError> {
    let context = ContextRepo::open(absolute_candidate(&args.context)?)?;
    let branch = context.current_branch()?;
    if branch != BOOTSTRAP_BRANCH {
        return Err(DevMapError::UnexpectedContextBranch(branch));
    }
    context.ensure_clean()?;

    let draft_bytes = context
        .read_owned(DRAFT_PATH)?
        .ok_or(DevMapError::MissingCommonGroundDraft)?;
    let draft: CommonGroundDraft = serde_json::from_slice(&draft_bytes)?;
    let draft_sha256 = sha256_hex(&draft_bytes);
    let approved_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let approval = ApprovalEvent::new(args.actor, approved_at.clone(), draft_sha256)?;
    let approval_object = context.write_canonical("approval", &approval)?;
    let common_ground =
        CommonGround::from_approved_draft(draft, approval_object.id.clone(), approved_at)?;
    let common_ground_object = context.write_canonical("common-ground", &common_ground)?;

    let manifest = CommonGroundManifest {
        schema_version: SCHEMA_VERSION.into(),
        draft_sha256: approval.draft_sha256.clone(),
        common_ground: object_reference(&common_ground_object),
        approval: object_reference(&approval_object),
    };
    context.write_owned(MANIFEST_PATH, &canonical_json(&manifest)?)?;
    let state = CurrentState {
        schema_version: SCHEMA_VERSION.into(),
        lifecycle: "approved".into(),
        manifest_path: MANIFEST_PATH.into(),
        common_ground_id: common_ground_object.id.clone(),
        capture_grade: "C".into(),
    };
    context.write_owned(CURRENT_STATE_PATH, &canonical_json(&state)?)?;
    context.remove_owned(DRAFT_PATH)?;

    context.commit_all("Approve initial DevMap Common Ground")?;
    let context_commit = context.promote_fast_forward(BOOTSTRAP_BRANCH)?;

    Ok(CommandOutput {
        stdout: format!(
            "common_ground=approved\ncommon_ground_id={}\napproval_id={}\ncontext_commit={context_commit}\ncapture_grade=C\n",
            common_ground_object.id, approval_object.id
        ),
        exit_code: 0,
    })
}

#[derive(Debug, serde::Serialize)]
struct StatusReport {
    schema_version: String,
    lifecycle: String,
    adoption_boundary_commit: Option<String>,
    context_commit: Option<String>,
    capture_grade: String,
    capture_grade_reason: String,
    context_dirty: bool,
    object_counts: BTreeMap<String, usize>,
    integrity: IntegrityReport,
}

#[derive(Debug, serde::Serialize)]
struct IntegrityReport {
    valid: bool,
    errors: Vec<String>,
}

pub fn status(args: StatusArgs) -> Result<CommandOutput, DevMapError> {
    let context = ContextRepo::open(absolute_candidate(&args.context)?)?;
    let context_dirty = !context
        .git(["status", "--porcelain=v1", "--untracked-files=all"])?
        .is_empty();
    let context_commit = context.git(["rev-parse", "main"]).ok();
    let draft_bytes = context.read_owned(DRAFT_PATH)?;
    let state_bytes = context.read_owned(CURRENT_STATE_PATH)?;
    let mut errors = Vec::new();
    let mut object_counts = BTreeMap::new();
    let mut adoption_boundary_commit = None;

    let lifecycle = match (&draft_bytes, &state_bytes) {
        (Some(_), Some(_)) => {
            errors.push("ambiguous_lifecycle:draft_and_approved_state_present".into());
            "invalid"
        }
        (Some(bytes), None) => {
            match serde_json::from_slice::<CommonGroundDraft>(bytes) {
                Ok(draft) => adoption_boundary_commit = Some(draft.source.head_commit),
                Err(error) => errors.push(format!("malformed_draft:{error}")),
            }
            "draft"
        }
        (None, Some(bytes)) => {
            verify_approved_state(
                &context,
                bytes,
                &mut adoption_boundary_commit,
                &mut object_counts,
                &mut errors,
            );
            "approved"
        }
        (None, None) => "absent",
    };

    let forbidden_refs = context.git([
        "for-each-ref",
        "--format=%(refname)",
        "refs/devmap",
        "refs/notes",
    ])?;
    for reference in forbidden_refs.lines().filter(|line| !line.is_empty()) {
        errors.push(format!("forbidden_ref:{reference}"));
    }

    errors.sort();
    errors.dedup();
    let valid = errors.is_empty();
    let report = StatusReport {
        schema_version: SCHEMA_VERSION.into(),
        lifecycle: lifecycle.into(),
        adoption_boundary_commit,
        context_commit,
        capture_grade: "C".into(),
        capture_grade_reason: "explicit CLI capture; automatic Agent hooks are not active".into(),
        context_dirty,
        object_counts,
        integrity: IntegrityReport { valid, errors },
    };
    let stdout = if args.json {
        format!(
            "{}\n",
            String::from_utf8(canonical_json(&report)?)
                .map_err(|_| { DevMapError::NonUtf8GitOutput("canonical status report".into()) })?
        )
    } else {
        render_status(&report)
    };

    Ok(CommandOutput {
        stdout,
        exit_code: if valid { 0 } else { 1 },
    })
}

fn verify_approved_state(
    context: &ContextRepo,
    state_bytes: &[u8],
    adoption_boundary_commit: &mut Option<String>,
    object_counts: &mut BTreeMap<String, usize>,
    errors: &mut Vec<String>,
) {
    let state: CurrentState = match serde_json::from_slice(state_bytes) {
        Ok(state) => state,
        Err(error) => {
            errors.push(format!("malformed_state:{error}"));
            return;
        }
    };
    if state.lifecycle != "approved" {
        errors.push(format!("invalid_state_lifecycle:{}", state.lifecycle));
    }
    if state.capture_grade != "C" {
        errors.push(format!("invalid_capture_grade:{}", state.capture_grade));
    }

    let manifest_bytes = match context.read_owned(&state.manifest_path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            errors.push(format!("missing_manifest:{}", state.manifest_path));
            return;
        }
        Err(error) => {
            errors.push(format!("manifest_read_error:{error}"));
            return;
        }
    };
    let manifest: CommonGroundManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            errors.push(format!("malformed_manifest:{error}"));
            return;
        }
    };
    if state.common_ground_id != manifest.common_ground.id {
        errors.push("state_common_ground_mismatch".into());
    }
    if manifest.common_ground.path == manifest.approval.path
        || manifest.common_ground.id == manifest.approval.id
    {
        errors.push("duplicate_manifest_object".into());
    }

    *object_counts.entry("common-ground".into()).or_insert(0) += 1;
    *object_counts.entry("approval".into()).or_insert(0) += 1;
    let common_ground_bytes = verify_object(context, &manifest.common_ground, errors);
    let approval_bytes = verify_object(context, &manifest.approval, errors);

    let common_ground = common_ground_bytes.and_then(|bytes| {
        serde_json::from_slice::<CommonGround>(&bytes)
            .map_err(|error| errors.push(format!("malformed_common_ground:{error}")))
            .ok()
    });
    let approval = approval_bytes.and_then(|bytes| {
        serde_json::from_slice::<ApprovalEvent>(&bytes)
            .map_err(|error| errors.push(format!("malformed_approval:{error}")))
            .ok()
    });

    if let Some(common_ground) = &common_ground {
        *adoption_boundary_commit = Some(common_ground.adoption_boundary_commit.clone());
        if common_ground.approval_id != manifest.approval.id {
            errors.push("common_ground_approval_mismatch".into());
        }
        if common_ground.adoption_boundary_commit != common_ground.source.head_commit {
            errors.push("adoption_boundary_mismatch".into());
        }
    }
    if let Some(approval) = &approval {
        if approval.draft_sha256 != manifest.draft_sha256 {
            errors.push("approval_draft_hash_mismatch".into());
        }
        if !is_sha256(&approval.draft_sha256) {
            errors.push("invalid_draft_hash".into());
        }
    }
}

fn verify_object(
    context: &ContextRepo,
    object: &CanonicalObjectRef,
    errors: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let bytes = match context.read_owned(&object.path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            errors.push(format!("missing_object:{}", object.id));
            return None;
        }
        Err(error) => {
            errors.push(format!("object_read_error:{}:{error}", object.id));
            return None;
        }
    };
    let actual = sha256_hex(&bytes);
    if actual != object.sha256 {
        errors.push(format!(
            "hash_mismatch:{}:expected={}:actual={actual}",
            object.id, object.sha256
        ));
    }
    let kind = object.id.split_once(":sha256-").map(|(kind, _)| kind);
    match kind {
        Some(kind) if object.id == format!("{kind}:sha256-{actual}") => {}
        _ => errors.push(format!("content_id_mismatch:{}", object.id)),
    }
    Some(bytes)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn render_status(report: &StatusReport) -> String {
    let mut output = format!(
        "lifecycle={}\nadoption_boundary_commit={}\ncontext_commit={}\ncapture_grade={}\ncapture_grade_reason={}\ncontext_dirty={}\nintegrity={}\n",
        report.lifecycle,
        report.adoption_boundary_commit.as_deref().unwrap_or("none"),
        report.context_commit.as_deref().unwrap_or("none"),
        report.capture_grade,
        report.capture_grade_reason,
        report.context_dirty,
        if report.integrity.valid {
            "valid"
        } else {
            "invalid"
        }
    );
    for (kind, count) in &report.object_counts {
        output.push_str(&format!("object_count.{kind}={count}\n"));
    }
    for error in &report.integrity.errors {
        output.push_str(&format!("integrity_error={error}\n"));
    }
    output
}

fn object_reference(object: &crate::context::StoredObject) -> CanonicalObjectRef {
    CanonicalObjectRef {
        id: object.id.clone(),
        path: object.relative_path.clone(),
        sha256: object.sha256.clone(),
    }
}

fn prepare_bootstrap_branch(context: &ContextRepo) -> Result<(), DevMapError> {
    let current = context.current_branch()?;
    if current == BOOTSTRAP_BRANCH {
        return Ok(());
    }
    if context.branch_exists(BOOTSTRAP_BRANCH)? {
        context.checkout(BOOTSTRAP_BRANCH)
    } else if current == "main" {
        context.create_branch(BOOTSTRAP_BRANCH)
    } else {
        Err(DevMapError::UnexpectedContextBranch(current))
    }
}

fn same_draft_intent(existing: &CommonGroundDraft, proposed: &CommonGroundDraft) -> bool {
    existing.schema_version == proposed.schema_version
        && existing.source == proposed.source
        && existing.goal == proposed.goal
        && existing.requirements == proposed.requirements
        && existing.historical_scope == HistoricalScope::NotReconstructed
}

fn load_requirement(source_root: &Path, locator: &str) -> Result<RequirementTrace, DevMapError> {
    let (path_text, anchor) = match locator.split_once('#') {
        Some((path, anchor)) if !anchor.trim().is_empty() => (path, Some(anchor.trim())),
        Some(_) => return Err(DevMapError::InvalidRequirementLocator(locator.to_owned())),
        None => (locator, None),
    };
    if path_text.trim().is_empty() {
        return Err(DevMapError::InvalidRequirementLocator(locator.to_owned()));
    }

    let requested = PathBuf::from(path_text);
    let joined = if requested.is_absolute() {
        requested
    } else {
        source_root.join(requested)
    };
    let absolute = joined.canonicalize()?;
    let canonical_source = source_root.canonicalize()?;
    if !absolute.starts_with(&canonical_source) {
        return Err(DevMapError::RequirementOutsideSource(absolute));
    }

    let contents = fs::read_to_string(&absolute)?;
    let quoted_requirement = match anchor {
        Some(anchor) => select_markdown_section(&contents, anchor)?,
        None => contents.trim().to_owned(),
    };
    let relative = absolute
        .strip_prefix(&canonical_source)
        .map_err(|_| DevMapError::RequirementOutsideSource(absolute.clone()))?
        .to_string_lossy()
        .replace('\\', "/");

    RequirementTrace::new(
        Some(relative),
        anchor.map(ToOwned::to_owned),
        quoted_requirement,
    )
}

fn select_markdown_section(contents: &str, requested_anchor: &str) -> Result<String, DevMapError> {
    let lines: Vec<_> = contents.lines().collect();
    let mut matches = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some((level, title)) = markdown_heading(line)
            && slug(title) == requested_anchor
        {
            matches.push((index, level));
        }
    }
    if matches.len() != 1 {
        return Err(DevMapError::RequirementAnchorMatch {
            anchor: requested_anchor.to_owned(),
            matches: matches.len(),
        });
    }

    let (start, level) = matches[0];
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| {
            markdown_heading(line)
                .filter(|(next_level, _)| *next_level <= level)
                .map(|_| index)
        })
        .unwrap_or(lines.len());
    Ok(lines[start..end].join("\n").trim().to_owned())
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level) || trimmed.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    Some((level, trimmed[level + 1..].trim()))
}

fn slug(title: &str) -> String {
    let mut result = String::new();
    let mut pending_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if pending_dash && !result.is_empty() {
                result.push('-');
            }
            pending_dash = false;
            result.push(character);
        } else {
            pending_dash = true;
        }
    }
    result
}

fn absolute_candidate(path: &Path) -> Result<PathBuf, DevMapError> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| DevMapError::InvalidContextPath(path.to_path_buf()))?;
    Ok(parent.canonicalize()?.join(file_name))
}

fn ensure_independent_repositories(source: &Path, context: &Path) -> Result<(), DevMapError> {
    let source = source.canonicalize()?;
    if context.starts_with(&source) || source.starts_with(context) {
        return Err(DevMapError::RepositoriesOverlap {
            source_path: source,
            context_path: context.to_path_buf(),
        });
    }
    Ok(())
}
