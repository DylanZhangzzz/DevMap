use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::canonical::sha256_hex;
use crate::error::DevMapError;
use crate::fs_security::{checked_canonical_directory, checked_file, checked_metadata};
use crate::git::SourceWorkspace;

const MAX_WORKTREES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeDescriptor {
    pub worktree_id: String,
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    pub is_current: bool,
    pub is_bare: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

#[derive(Default)]
struct PorcelainRecord {
    root: Option<PathBuf>,
    head: Option<String>,
    branch: Option<String>,
    detached: bool,
    bare: bool,
    locked: bool,
    prunable: bool,
}

pub struct WorktreeScanner;

pub fn repository_id(workspace: &SourceWorkspace) -> String {
    format!(
        "sha256-{}",
        sha256_hex(normalized_path(&workspace.git_common_dir).as_bytes())
    )
}

impl WorktreeScanner {
    pub fn scan(workspace: &SourceWorkspace) -> Result<Vec<WorktreeDescriptor>, DevMapError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .args(["worktree", "list", "--porcelain", "-z"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;
        if !output.status.success() {
            return Err(DevMapError::GitCommand {
                command: "git worktree list --porcelain -z".into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let records = parse_porcelain(&output.stdout)?;
        let repository_id = repository_id(workspace);
        let current_root = checked_canonical_directory(&workspace.root)?;
        let mut seen_git_dirs = BTreeSet::new();
        let mut rows = Vec::with_capacity(records.len());

        for record in records {
            let root = record.root.ok_or_else(malformed)?;
            let canonical_root = match checked_canonical_directory(&root) {
                Ok(path) => Some(path),
                Err(DevMapError::UnsafeInstallerOverwrite(_)) if record.prunable => None,
                Err(error) => return Err(error),
            };
            let git_dir = if record.prunable && canonical_root.is_none() {
                resolve_prunable_git_dir(&root, &workspace.git_common_dir)?
            } else {
                resolve_git_dir(&root, record.bare)?
            };
            let normalized_git_dir = normalized_path(&git_dir);
            if !seen_git_dirs.insert(normalized_git_dir.clone()) {
                return Err(malformed());
            }
            let worktree_id = format!(
                "wt-{}",
                sha256_hex(format!("{repository_id}\0{normalized_git_dir}").as_bytes())
            );
            rows.push(WorktreeDescriptor {
                worktree_id,
                is_current: canonical_root.as_ref() == Some(&current_root),
                root,
                git_dir,
                branch: record.branch,
                head: record.head.ok_or_else(malformed)?,
                is_bare: record.bare,
                is_locked: record.locked,
                is_prunable: record.prunable,
            });
        }
        rows.sort_by(|left, right| left.worktree_id.cmp(&right.worktree_id));
        Ok(rows)
    }
}

fn parse_porcelain(bytes: &[u8]) -> Result<Vec<PorcelainRecord>, DevMapError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| DevMapError::NonUtf8GitOutput("git worktree list --porcelain -z".into()))?;
    let mut records = Vec::new();
    for raw_record in text.split("\0\0").filter(|record| !record.is_empty()) {
        if records.len() == MAX_WORKTREES {
            return Err(DevMapError::ResourceLimit {
                resource: "worktrees",
                limit: MAX_WORKTREES,
            });
        }
        let mut record = PorcelainRecord::default();
        let mut seen = BTreeSet::new();
        for field in raw_record.split('\0').filter(|field| !field.is_empty()) {
            let (name, value) = field.split_once(' ').unwrap_or((field, ""));
            if !seen.insert(name) {
                return Err(malformed());
            }
            match name {
                "worktree" if !value.is_empty() => record.root = Some(PathBuf::from(value)),
                "HEAD" if !value.is_empty() => record.head = Some(value.to_owned()),
                "branch" if !value.is_empty() => {
                    record.branch = Some(
                        value
                            .strip_prefix("refs/heads/")
                            .unwrap_or(value)
                            .to_owned(),
                    )
                }
                "detached" if value.is_empty() => record.detached = true,
                "bare" if value.is_empty() => record.bare = true,
                "locked" => record.locked = true,
                "prunable" => record.prunable = true,
                _ => return Err(malformed()),
            }
        }
        if record.root.is_none()
            || record.head.is_none()
            || (record.branch.is_none() && !record.detached && !record.bare)
            || (record.branch.is_some() && record.detached)
        {
            return Err(malformed());
        }
        records.push(record);
    }
    if records.is_empty() {
        return Err(malformed());
    }
    Ok(records)
}

fn resolve_git_dir(root: &Path, bare: bool) -> Result<PathBuf, DevMapError> {
    if bare {
        return checked_canonical_directory(root);
    }
    let dot_git = root.join(".git");
    let metadata = checked_metadata(&dot_git)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(dot_git.clone()))?;
    if metadata.is_dir() {
        return checked_canonical_directory(&dot_git);
    }
    if !metadata.is_file() {
        return Err(DevMapError::UnsafeInstallerOverwrite(dot_git));
    }
    let mut file = checked_file(&dot_git, false, false)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::InvalidData => {
                DevMapError::NonUtf8GitOutput("linked worktree .git file".into())
            }
            _ => error.into(),
        })?;
    let value = text.trim().strip_prefix("gitdir: ").ok_or_else(malformed)?;
    let path = PathBuf::from(value);
    let resolved = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    checked_canonical_directory(&resolved)
}

fn resolve_prunable_git_dir(root: &Path, git_common_dir: &Path) -> Result<PathBuf, DevMapError> {
    let administration_root = git_common_dir.join("worktrees");
    let metadata = checked_metadata(&administration_root)?
        .ok_or_else(|| DevMapError::UnsafeInstallerOverwrite(administration_root.clone()))?;
    if !metadata.is_dir() {
        return Err(DevMapError::UnsafeInstallerOverwrite(administration_root));
    }
    let expected = normalized_path(&root.join(".git"));
    let mut entries = std::fs::read_dir(&administration_root)?;
    for _ in 0..MAX_WORKTREES {
        let Some(entry) = entries.next() else {
            break;
        };
        let entry = entry?;
        let admin_dir = entry.path();
        let Some(metadata) = checked_metadata(&admin_dir)? else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let gitdir_file = admin_dir.join("gitdir");
        let Some(metadata) = checked_metadata(&gitdir_file)? else {
            continue;
        };
        if !metadata.is_file() {
            return Err(DevMapError::UnsafeInstallerOverwrite(gitdir_file));
        }
        let mut file = checked_file(&gitdir_file, false, false)?;
        let mut candidate = String::new();
        file.read_to_string(&mut candidate).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                DevMapError::NonUtf8GitOutput("worktree administration gitdir".into())
            } else {
                error.into()
            }
        })?;
        if normalized_path(Path::new(candidate.trim())) == expected {
            return checked_canonical_directory(&admin_dir);
        }
    }
    Err(malformed())
}

fn normalized_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn malformed() -> DevMapError {
    DevMapError::MalformedWorktreePorcelain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_duplicate_fields() {
        let input = b"worktree /repo\0HEAD 0123\0HEAD 4567\0branch refs/heads/main\0\0";
        assert!(matches!(
            parse_porcelain(input),
            Err(DevMapError::MalformedWorktreePorcelain)
        ));
    }

    #[test]
    fn parser_rejects_non_utf8_output() {
        assert!(matches!(
            parse_porcelain(&[0xff, 0]),
            Err(DevMapError::NonUtf8GitOutput(_))
        ));
    }

    #[test]
    fn parser_enforces_the_worktree_limit() {
        let input = (0..=MAX_WORKTREES)
            .map(|index| {
                format!("worktree /repo/{index}\0HEAD 0123\0branch refs/heads/b{index}\0\0")
            })
            .collect::<String>();
        assert!(matches!(
            parse_porcelain(input.as_bytes()),
            Err(DevMapError::ResourceLimit {
                resource: "worktrees",
                limit: MAX_WORKTREES
            })
        ));
    }

    #[test]
    fn parser_preserves_detached_locked_and_prunable_markers() {
        let input =
            b"worktree /repo\0HEAD 0123\0detached\0locked maintenance\0prunable missing\0\0";
        let record = parse_porcelain(input).unwrap().pop().unwrap();
        assert!(record.detached);
        assert!(record.locked);
        assert!(record.prunable);
        assert_eq!(record.branch, None);
    }
}
