use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Output};

use serde::Serialize;

use crate::error::DevMapError;
use crate::git::SourceWorkspace;
use crate::worktrees::WorktreeDescriptor;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSource {
    Config,
    LocalDev,
    LocalDevelop,
    RemoteDefault,
    LocalMain,
    LocalMaster,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DevelopmentTarget {
    pub name: String,
    pub ref_name: String,
    pub source: TargetSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitRelationship {
    pub base_target: Option<String>,
    pub merge_target: Option<String>,
    pub merged: Option<bool>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub dirty: bool,
    pub changed_file_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRelationshipWarning {
    pub code: &'static str,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRelationshipReport {
    pub target: Option<DevelopmentTarget>,
    pub by_worktree_id: BTreeMap<String, GitRelationship>,
    pub warnings: Vec<GitRelationshipWarning>,
}

pub struct GitRelationshipResolver;

impl GitRelationshipResolver {
    pub fn resolve(
        workspace: &SourceWorkspace,
        worktrees: &[WorktreeDescriptor],
    ) -> Result<GitRelationshipReport, DevMapError> {
        let mut warnings = Vec::new();
        let target = select_target(workspace, &mut warnings)?;
        let mut by_worktree_id = BTreeMap::new();

        let mut unique = BTreeMap::<(std::path::PathBuf, String), Vec<&WorktreeDescriptor>>::new();
        for worktree in worktrees {
            unique
                .entry((worktree.root.clone(), worktree.head.clone()))
                .or_default()
                .push(worktree);
        }
        let unique = unique.into_values().collect::<Vec<_>>();
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .saturating_mul(2)
            .min(unique.len().max(1));
        let chunk_size = unique.len().div_ceil(worker_count);
        let resolved = std::thread::scope(|scope| {
            unique
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(|| {
                        chunk
                            .iter()
                            .map(|matches| {
                                (
                                    matches.clone(),
                                    relationship_for(matches[0], target.as_ref()),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|worker| worker.join().expect("Git relationship worker panicked"))
                .collect::<Vec<_>>()
        });

        for (matches, result) in resolved {
            for worktree in matches {
                let relationship = match &result {
                    Ok(relationship) => relationship.clone(),
                    Err(_) => {
                        warnings.push(GitRelationshipWarning {
                            code: "git_relationship_unavailable",
                            worktree_id: Some(worktree.worktree_id.clone()),
                        });
                        unknown_relationship(target.as_ref())
                    }
                };
                by_worktree_id.insert(worktree.worktree_id.clone(), relationship);
            }
        }

        Ok(GitRelationshipReport {
            target,
            by_worktree_id,
            warnings,
        })
    }
}

fn select_target(
    workspace: &SourceWorkspace,
    warnings: &mut Vec<GitRelationshipWarning>,
) -> Result<Option<DevelopmentTarget>, DevMapError> {
    if let Some(configured) = optional_text(
        &workspace.root,
        ["config", "--get", "devmap.developmentTarget"],
    )? {
        let ref_name = configured_ref(&configured);
        if ref_name
            .as_deref()
            .is_some_and(|candidate| ref_exists(&workspace.root, candidate).unwrap_or(false))
        {
            return Ok(Some(DevelopmentTarget {
                name: configured,
                ref_name: ref_name.expect("validated configured ref"),
                source: TargetSource::Config,
            }));
        }
        warnings.push(GitRelationshipWarning {
            code: "configured_development_target_unavailable",
            worktree_id: None,
        });
    }

    for (name, source) in [
        ("dev", TargetSource::LocalDev),
        ("develop", TargetSource::LocalDevelop),
    ] {
        let ref_name = format!("refs/heads/{name}");
        if ref_exists(&workspace.root, &ref_name)? {
            return Ok(Some(DevelopmentTarget {
                name: name.into(),
                ref_name,
                source,
            }));
        }
    }

    if let Some(ref_name) = optional_text(
        &workspace.root,
        ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )? && ref_exists(&workspace.root, &ref_name)?
    {
        let name = ref_name
            .strip_prefix("refs/remotes/origin/")
            .unwrap_or(&ref_name)
            .to_owned();
        return Ok(Some(DevelopmentTarget {
            name,
            ref_name,
            source: TargetSource::RemoteDefault,
        }));
    }

    for (name, source) in [
        ("main", TargetSource::LocalMain),
        ("master", TargetSource::LocalMaster),
    ] {
        let ref_name = format!("refs/heads/{name}");
        if ref_exists(&workspace.root, &ref_name)? {
            return Ok(Some(DevelopmentTarget {
                name: name.into(),
                ref_name,
                source,
            }));
        }
    }

    Ok(None)
}

fn configured_ref(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('-')
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return None;
    }
    Some(if value.starts_with("refs/") {
        value.to_owned()
    } else {
        format!("refs/heads/{value}")
    })
}

fn relationship_for(
    worktree: &WorktreeDescriptor,
    target: Option<&DevelopmentTarget>,
) -> Result<GitRelationship, DevMapError> {
    let (dirty, changed_file_count) = dirty_state(&worktree.root)?;
    let Some(target) = target else {
        return Ok(GitRelationship {
            base_target: None,
            merge_target: None,
            merged: None,
            ahead: None,
            behind: None,
            dirty,
            changed_file_count,
        });
    };

    let counts = required_text(
        &worktree.root,
        [
            OsString::from("rev-list"),
            OsString::from("--left-right"),
            OsString::from("--count"),
            OsString::from(format!("{}...{}", target.ref_name, worktree.head)),
        ],
    )?;
    let mut fields = counts.split_whitespace();
    let behind = parse_count(fields.next())?;
    let ahead = parse_count(fields.next())?;
    if fields.next().is_some() {
        return Err(malformed_git("git rev-list --left-right --count"));
    }

    Ok(GitRelationship {
        base_target: Some(target.name.clone()),
        merge_target: Some(target.name.clone()),
        merged: Some(ahead == 0),
        ahead: Some(ahead),
        behind: Some(behind),
        dirty,
        changed_file_count,
    })
}

fn unknown_relationship(target: Option<&DevelopmentTarget>) -> GitRelationship {
    GitRelationship {
        base_target: target.map(|value| value.name.clone()),
        merge_target: target.map(|value| value.name.clone()),
        merged: None,
        ahead: None,
        behind: None,
        dirty: false,
        changed_file_count: 0,
    }
}

fn dirty_state(root: &Path) -> Result<(bool, u32), DevMapError> {
    let output = required_output(
        root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )?;
    let mut fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut count = 0_u32;
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(malformed_git("git status --porcelain=v1 -z"));
        }
        count = count.saturating_add(1);
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            fields
                .next()
                .ok_or_else(|| malformed_git("git status --porcelain=v1 -z"))?;
        }
    }
    Ok((count > 0, count))
}

fn ref_exists(root: &Path, ref_name: &str) -> Result<bool, DevMapError> {
    let output = git_output(root, ["show-ref", "--verify", "--quiet", ref_name])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(command_failure("git show-ref --verify --quiet", &output)),
    }
}

fn optional_text<I, S>(root: &Path, args: I) -> Result<Option<String>, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Ok(None);
    }
    output_text(&output, &display_command(&arguments)).map(Some)
}

fn required_text<I, S>(root: &Path, args: I) -> Result<String, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Err(command_failure(&display_command(&arguments), &output));
    }
    output_text(&output, &display_command(&arguments))
}

fn required_output<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Err(command_failure(&display_command(&arguments), &output));
    }
    Ok(output)
}

fn git_output<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?)
}

fn output_text(output: &Output, command: &str) -> Result<String, DevMapError> {
    String::from_utf8(output.stdout.clone())
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| DevMapError::NonUtf8GitOutput(command.to_owned()))
}

fn parse_count(value: Option<&str>) -> Result<u32, DevMapError> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.min(u32::MAX as u64) as u32)
        .ok_or_else(|| malformed_git("git rev-list --left-right --count"))
}

fn display_command(args: &[OsString]) -> String {
    format!(
        "git {}",
        args.iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn command_failure(command: &str, output: &Output) -> DevMapError {
    DevMapError::GitCommand {
        command: command.to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn malformed_git(command: &str) -> DevMapError {
    DevMapError::GitCommand {
        command: command.to_owned(),
        stderr: "malformed Git output".into(),
    }
}
