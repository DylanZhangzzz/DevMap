//! Filesystem-rechecked plain configuration for read-only query projection.
//! Shares low-level bounded witnesses, not relationship/ref caching behavior.
use super::*;

pub(crate) struct QueryConfiguration {
    evidence: Evidence,
    value: Option<String>,
}
fn environment() -> Result<Vec<(OsString, OsString)>, DevMapError> {
    let mut values: Vec<_> = std::env::vars_os().collect();
    values.sort();
    for (key, value) in &values {
        let key = key.to_string_lossy().to_ascii_uppercase();
        if matches!(key.as_str(), "HOME" | "USERPROFILE" | "XDG_CONFIG_HOME")
            && value
                .to_str()
                .is_none_or(|v| v.chars().any(char::is_control))
        {
            return Err(decline());
        }
        if key.starts_with("GIT_")
            && !match key.as_str() {
                "GIT_TERMINAL_PROMPT" => value == "0",
                "GIT_NO_LAZY_FETCH" | "GIT_NO_REPLACE_OBJECTS" => value == "1",
                "GIT_PAGER" => value.is_empty() || value == "cat",
                // Keep the existing reviewed diagnostic-only policy; no query hook.
                "GIT_TRACE" | "GIT_TRACE2" | "GIT_TRACE2_EVENT" | "GIT_TRACE2_PERF" => true,
                _ => false,
            }
        {
            return Err(decline());
        }
    }
    Ok(values)
}
fn blank(environment: Vec<(OsString, OsString)>) -> Evidence {
    Evidence {
        environment,
        directories: BTreeMap::new(),
        files: BTreeMap::new(),
        system: Vec::new(),
        global: Vec::new(),
        config: Vec::new(),
        refs: Vec::new(),
        remote_head: Vec::new(),
    }
}
impl QueryConfiguration {
    pub(crate) fn acquire(workspace: &SourceWorkspace) -> Result<Option<Self>, DevMapError> {
        optional(Self::capture(workspace))
    }
    fn capture(workspace: &SourceWorkspace) -> Result<Self, DevMapError> {
        let mut evidence = blank(environment()?);
        let common = directory(&workspace.git_common_dir, &mut evidence)?;
        let admin = directory(&workspace.git_dir, &mut evidence)?;
        let root = directory(&workspace.root, &mut evidence)?;
        // Includes and worktreeConfig are deliberately unsupported initially.
        absent(&common.join("config.worktree"), &mut evidence)?;
        absent(&admin.join("config.worktree"), &mut evidence)?;
        evidence.system = probe(&workspace.root, &["var", "GIT_CONFIG_SYSTEM"])?;
        evidence.global = probe(&workspace.root, &["var", "GIT_CONFIG_GLOBAL"])?;
        let mut candidates: Vec<_> = candidate_paths(&evidence.system)?
            .into_iter()
            .map(|p| (p, "system"))
            .chain(
                candidate_paths(&evidence.global)?
                    .into_iter()
                    .map(|p| (p, "global")),
            )
            .collect();
        candidates.push((common.join("config"), "local"));
        for (file, _) in &candidates {
            witness(file, &mut evidence)?;
        }
        evidence.config = probe(
            &workspace.root,
            &[
                "config",
                "--no-includes",
                "--null",
                "--list",
                "--show-origin",
                "--show-scope",
            ],
        )?;
        let text = std::str::from_utf8(&evidence.config).map_err(|_| decline())?;
        let fields: Vec<_> = text.split_terminator('\0').collect();
        if !evidence.config.ends_with(&[0])
            || !fields.len().is_multiple_of(3)
            || fields.len() > 6144
        {
            return Err(decline());
        }
        for row in fields.chunks_exact(3) {
            let origin = row[1].strip_prefix("file:").ok_or_else(decline)?;
            let origin = if origin == ".git/config" {
                root.join(origin)
            } else {
                PathBuf::from(origin)
            };
            if !origin.is_absolute() {
                return Err(decline());
            }
            let canonical = std::fs::canonicalize(&origin)?;
            if !candidates.iter().any(|(p, scope)| {
                *scope == row[0] && std::fs::canonicalize(p).ok().as_ref() == Some(&canonical)
            }) {
                return Err(decline());
            }
            let (key, value) = row[2].split_once('\n').ok_or_else(decline)?;
            if key.len() > 512
                || value.len() > 32768
                || value.chars().any(char::is_control)
                || !(key == "devmap.developmenttarget" || admitted_key(key, value))
            {
                return Err(decline());
            }
        }
        let value = super::super::GitRelationshipResolver::development_configuration(workspace)?;
        let proof = Self { evidence, value };
        if !proof.recheck()? {
            return Err(decline());
        }
        Ok(proof)
    }
    pub(crate) fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
    /// No Git commands: changed or inaccessible witnesses force the original path.
    pub(crate) fn recheck(&self) -> Result<bool, DevMapError> {
        Ok(optional(self.recheck_inner())?.unwrap_or(false))
    }
    fn recheck_inner(&self) -> Result<bool, DevMapError> {
        let current = environment()?;
        if current != self.evidence.environment {
            return Ok(false);
        }
        for (path, identity) in &self.evidence.directories {
            if checked_directory_identity(path)? != *identity {
                return Ok(false);
            }
        }
        let mut after = blank(current);
        for path in self.evidence.files.keys() {
            witness(path, &mut after)?;
        }
        Ok(after.files == self.evidence.files
            && after
                .directories
                .iter()
                .all(|(p, id)| self.evidence.directories.get(p) == Some(id)))
    }
}
