use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::canonical::sha256_hex;
use crate::domain::SourceAnchor;
use crate::error::DevMapError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWorkspace {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub branch: Option<String>,
    pub head: String,
}

#[derive(Debug, Clone)]
pub struct SourceGitInspector {
    root: PathBuf,
}

impl SourceGitInspector {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DevMapError> {
        let requested = path.as_ref();
        let output = git_at(requested, ["rev-parse", "--show-toplevel"])?;
        if !output.status.success() {
            return Err(DevMapError::NotGitRepository(requested.to_path_buf()));
        }

        let root = output_text(&output, "git rev-parse --show-toplevel")?;
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn inspect(&self) -> Result<SourceAnchor, DevMapError> {
        let head_commit = self.required_git(["rev-parse", "HEAD"])?;
        let default_branch = self.optional_git(["symbolic-ref", "--short", "-q", "HEAD"])?;
        let remote_url = self.optional_git(["remote", "get-url", "origin"])?;
        let dirty_at_adoption = !self.required_git(["status", "--porcelain=v1"])?.is_empty();

        let identity = match remote_url.as_deref() {
            Some(remote) => normalize_remote(remote),
            None => format!("{}\n{head_commit}", self.root.to_string_lossy()),
        };
        let repository_fingerprint = format!("sha256-{}", sha256_hex(identity.as_bytes()));

        Ok(SourceAnchor {
            repository_fingerprint,
            remote_url,
            head_commit,
            default_branch,
            dirty_at_adoption,
        })
    }

    pub fn workspace(&self) -> Result<SourceWorkspace, DevMapError> {
        let head = self.required_git(["rev-parse", "HEAD"])?;
        self.workspace_with_head(head)
    }

    pub fn workspace_allow_unborn(&self) -> Result<SourceWorkspace, DevMapError> {
        let head_output = git_at(&self.root, ["rev-parse", "HEAD"])?;
        let head = if head_output.status.success() {
            output_text(&head_output, "git rev-parse HEAD")?
        } else {
            let original = DevMapError::GitCommand {
                command: "git rev-parse HEAD".into(),
                stderr: String::from_utf8_lossy(&head_output.stderr)
                    .trim()
                    .to_owned(),
            };
            let symbolic = git_at(&self.root, ["symbolic-ref", "--quiet", "HEAD"])?;
            if !symbolic.status.success() {
                return Err(original);
            }
            let symbolic = output_text(&symbolic, "git symbolic-ref --quiet HEAD")?;
            if !symbolic.starts_with("refs/heads/") {
                return Err(original);
            }
            let exists = git_at(
                &self.root,
                ["show-ref", "--verify", "--quiet", symbolic.as_str()],
            )?;
            match exists.status.code() {
                Some(1) => String::new(),
                _ => return Err(original),
            }
        };
        self.workspace_with_head(head)
    }

    fn workspace_with_head(&self, head: String) -> Result<SourceWorkspace, DevMapError> {
        let root = self.required_git(["rev-parse", "--show-toplevel"])?;
        let git_dir = self.required_git(["rev-parse", "--git-dir"])?;
        let git_common_dir = self.required_git(["rev-parse", "--git-common-dir"])?;
        let branch = self.optional_git(["symbolic-ref", "--short", "-q", "HEAD"])?;
        let root = PathBuf::from(root);
        let resolve = |value: String| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        };
        let git_dir = resolve(git_dir);
        let git_common_dir =
            crate::fs_security::checked_canonical_directory(&resolve(git_common_dir))?;

        Ok(SourceWorkspace {
            git_dir,
            git_common_dir,
            root,
            branch,
            head,
        })
    }

    fn required_git<I, S>(&self, args: I) -> Result<String, DevMapError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<_> = args
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect();
        let output = git_at(&self.root, &args)?;
        if !output.status.success() {
            return Err(DevMapError::GitCommand {
                command: display_command(&args),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        output_text(&output, &display_command(&args))
    }

    fn optional_git<I, S>(&self, args: I) -> Result<Option<String>, DevMapError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<_> = args
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect();
        let output = git_at(&self.root, &args)?;
        if !output.status.success() {
            return Ok(None);
        }
        output_text(&output, &display_command(&args)).map(Some)
    }
}

fn git_at<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()?)
}

fn output_text(output: &Output, command: &str) -> Result<String, DevMapError> {
    String::from_utf8(output.stdout.clone())
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| DevMapError::NonUtf8GitOutput(command.to_owned()))
}

fn display_command(args: &[std::ffi::OsString]) -> String {
    let arguments = args
        .iter()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {arguments}")
}

fn normalize_remote(remote: &str) -> String {
    remote
        .trim()
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(remote.trim().trim_end_matches('/'))
        .replace('\\', "/")
}
