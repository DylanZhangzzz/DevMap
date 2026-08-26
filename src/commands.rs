use std::fs;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::canonical::{canonical_json, sha256_hex};
use crate::cli::InitArgs;
use crate::context::ContextRepo;
use crate::domain::{CommonGroundDraft, HistoricalScope, RequirementTrace};
use crate::error::DevMapError;
use crate::git::SourceGitInspector;
use crate::CommandOutput;

const DRAFT_PATH: &str = "bootstrap/common-ground-draft.json";
const BOOTSTRAP_BRANCH: &str = "bootstrap/initial";

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
    })
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
        if let Some((level, title)) = markdown_heading(line) {
            if slug(title) == requested_anchor {
                matches.push((index, level));
            }
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
