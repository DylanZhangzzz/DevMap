use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;
use serde_json::json;

use crate::canonical::{canonical_json, content_id, sha256_hex};
use crate::error::DevMapError;

const CONTEXT_MARKER: &str = ".devmap-context.json";
const BOT_NAME: &str = "DevMap Bot";
const BOT_EMAIL: &str = "devmap-bot@localhost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub id: String,
    pub relative_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ContextRepo {
    root: PathBuf,
}

impl ContextRepo {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, DevMapError> {
        let root = path.as_ref().to_path_buf();
        if root.exists() && fs::read_dir(&root)?.next().is_some() {
            return Err(DevMapError::ContextPathNotEmpty(root));
        }
        fs::create_dir_all(&root)?;

        git_checked(&root, ["init", "-b", "main"])?;
        git_checked(&root, ["config", "--local", "user.name", BOT_NAME])?;
        git_checked(&root, ["config", "--local", "user.email", BOT_EMAIL])?;

        let metadata = canonical_json(&json!({
            "schema_version": "devmap/0.1",
            "type": "context_repository"
        }))?;
        fs::write(root.join(CONTEXT_MARKER), metadata)?;
        git_checked(&root, ["add", "--", CONTEXT_MARKER])?;
        git_checked(
            &root,
            ["commit", "-m", "Initialize DevMap context repository"],
        )?;

        Ok(Self { root })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, DevMapError> {
        let root = path.as_ref().to_path_buf();
        if !root.join(".git").is_dir() || !root.join(CONTEXT_MARKER).is_file() {
            return Err(DevMapError::NotContextRepository(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn current_branch(&self) -> Result<String, DevMapError> {
        self.git(["branch", "--show-current"])
    }

    pub fn branch_exists(&self, branch: &str) -> Result<bool, DevMapError> {
        let reference = format!("refs/heads/{branch}");
        let arguments = [
            OsString::from("show-ref"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from(reference),
        ];
        let output = git_output(&self.root, &arguments)?;
        Ok(output.status.success())
    }

    pub fn checkout(&self, branch: &str) -> Result<(), DevMapError> {
        self.git(["checkout", branch])?;
        Ok(())
    }

    pub fn create_branch(&self, branch: &str) -> Result<(), DevMapError> {
        self.git(["checkout", "-b", branch])?;
        Ok(())
    }

    pub fn write_owned(&self, relative_path: &str, bytes: &[u8]) -> Result<(), DevMapError> {
        let normalized = validate_owned_relative_path(relative_path)?;
        let absolute_path = self.root.join(&normalized);
        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(absolute_path, bytes)?;
        Ok(())
    }

    pub fn read_owned(&self, relative_path: &str) -> Result<Option<Vec<u8>>, DevMapError> {
        let normalized = validate_owned_relative_path(relative_path)?;
        let absolute_path = self.root.join(normalized);
        if !absolute_path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(absolute_path)?))
    }

    pub fn remove_owned(&self, relative_path: &str) -> Result<(), DevMapError> {
        let normalized = validate_owned_relative_path(relative_path)?;
        let absolute_path = self.root.join(normalized);
        if absolute_path.exists() {
            fs::remove_file(absolute_path)?;
        }
        Ok(())
    }

    pub fn ensure_clean(&self) -> Result<(), DevMapError> {
        let status = self.git(["status", "--porcelain=v1", "--untracked-files=all"])?;
        if !status.is_empty() {
            return Err(DevMapError::ContextNotClean(status));
        }
        Ok(())
    }

    pub fn promote_fast_forward(&self, branch: &str) -> Result<String, DevMapError> {
        self.git(["checkout", "main"])?;
        self.git(["merge", "--ff-only", branch])?;
        self.git(["branch", "-d", branch])?;
        self.git(["rev-parse", "HEAD"])
    }

    pub fn write_canonical<T: Serialize>(
        &self,
        kind: &str,
        value: &T,
    ) -> Result<StoredObject, DevMapError> {
        validate_kind(kind)?;
        let bytes = canonical_json(value)?;
        let sha256 = sha256_hex(&bytes);
        let id = content_id(kind, &bytes);
        let relative_path = format!("objects/{kind}/{sha256}.json");
        let absolute_path = self.root.join(&relative_path);

        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if absolute_path.exists() {
            let existing = fs::read(&absolute_path)?;
            if existing != bytes {
                return Err(DevMapError::ContentAddressCollision(relative_path));
            }
        } else {
            fs::write(&absolute_path, bytes)?;
        }

        Ok(StoredObject {
            id,
            relative_path,
            sha256,
        })
    }

    pub fn commit_all(&self, message: &str) -> Result<String, DevMapError> {
        if message.trim().is_empty() {
            return Err(DevMapError::InvalidDomain("commit message"));
        }

        let changes = self.changed_paths()?;
        let unexpected: Vec<_> = changes
            .iter()
            .filter(|path| !is_owned_path(path))
            .cloned()
            .collect();
        if !unexpected.is_empty() {
            return Err(DevMapError::UnexpectedContextPaths(unexpected));
        }

        if changes.is_empty() {
            return self.git(["rev-parse", "HEAD"]);
        }

        for path in &changes {
            self.git_os([OsStr::new("add"), OsStr::new("--"), OsStr::new(path)])?;
        }
        self.git(["commit", "-m", message])?;
        self.git(["rev-parse", "HEAD"])
    }

    pub(crate) fn git<I, S>(&self, args: I) -> Result<String, DevMapError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git_os(args)
    }

    fn git_os<I, S>(&self, args: I) -> Result<String, DevMapError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments: Vec<OsString> = args
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect();
        let output = git_output(&self.root, &arguments)?;
        if !output.status.success() {
            return Err(git_failure(&arguments, &output));
        }
        decode_stdout(&arguments, output.stdout)
    }

    fn changed_paths(&self) -> Result<Vec<String>, DevMapError> {
        let arguments = [
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
        ];
        let output = git_output(&self.root, &arguments)?;
        if !output.status.success() {
            return Err(git_failure(&arguments, &output));
        }

        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                if entry.len() < 4 {
                    return Err(DevMapError::MalformedGitStatus);
                }
                String::from_utf8(entry[3..].to_vec())
                    .map(|path| path.replace('\\', "/"))
                    .map_err(|_| DevMapError::NonUtf8GitOutput("git status".into()))
            })
            .collect()
    }
}

fn validate_kind(kind: &str) -> Result<(), DevMapError> {
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(DevMapError::InvalidObjectKind(kind.to_owned()));
    }
    Ok(())
}

fn is_owned_path(path: &str) -> bool {
    path == CONTEXT_MARKER
        || ["objects/", "manifests/", "bootstrap/", "state/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

fn validate_owned_relative_path(path: &str) -> Result<String, DevMapError> {
    let normalized = path.replace('\\', "/");
    let parsed = Path::new(&normalized);
    let contains_only_normal_components = parsed
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)));
    if !contains_only_normal_components || !is_owned_path(&normalized) {
        return Err(DevMapError::UnexpectedContextPaths(vec![normalized]));
    }
    Ok(normalized)
}

fn git_checked<I, S>(root: &Path, args: I) -> Result<String, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Err(git_failure(&arguments, &output));
    }
    decode_stdout(&arguments, output.stdout)
}

fn git_output(root: &Path, arguments: &[OsString]) -> Result<Output, DevMapError> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?)
}

fn git_failure(arguments: &[OsString], output: &Output) -> DevMapError {
    DevMapError::GitCommand {
        command: display_command(arguments),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn decode_stdout(arguments: &[OsString], stdout: Vec<u8>) -> Result<String, DevMapError> {
    String::from_utf8(stdout)
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| DevMapError::NonUtf8GitOutput(display_command(arguments)))
}

fn display_command(arguments: &[OsString]) -> String {
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {arguments}")
}
