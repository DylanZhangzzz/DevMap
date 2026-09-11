//! Filesystem-rechecked plain configuration for read-only query projection.
//! Shares low-level bounded witnesses, not relationship/ref caching behavior.
use super::*;

pub(crate) struct QueryConfiguration {
    evidence: Evidence,
    value: Option<String>,
    source: SourceResolutionWitness,
}

// Discovery dependencies, not a replacement Git ref parser. A successful fresh
// inspector is required after capture before this proof can authorize a query.
struct SourceResolutionWitness {
    common: PathBuf,
    admin: PathBuf,
    evidence: Evidence,
}
impl SourceResolutionWitness {
    fn capture(common: &Path, admin: &Path) -> Result<Self, DevMapError> {
        let mut evidence = blank(Vec::new());
        directory(common, &mut evidence)?;
        directory(admin, &mut evidence)?;
        let objects = directory(&common.join("objects"), &mut evidence)?;
        // Git discovery requires accessible objects/refs directories. Opening an
        // iterator checks access without scanning the potentially huge object DB.
        let _objects_access = std::fs::read_dir(objects)?;
        let mut nodes = 0usize;
        let mut path_bytes = 0usize;
        fn refs_tree(
            path: &Path,
            depth: usize,
            evidence: &mut Evidence,
            nodes: &mut usize,
            path_bytes: &mut usize,
        ) -> Result<(), DevMapError> {
            if depth > 16 {
                return Err(decline());
            }
            directory(path, evidence)?;
            for entry in std::fs::read_dir(path)? {
                let path = entry?.path();
                *nodes += 1;
                *path_bytes = path_bytes.saturating_add(path.as_os_str().len());
                if *nodes > 2048 || *path_bytes > MAX_BYTES {
                    return Err(decline());
                }
                let meta = checked_metadata(&path)?.ok_or_else(decline)?;
                if meta.is_dir() {
                    refs_tree(&path, depth + 1, evidence, nodes, path_bytes)?;
                } else {
                    witness(&path, evidence)?;
                }
            }
            Ok(())
        }
        refs_tree(
            &common.join("refs"),
            0,
            &mut evidence,
            &mut nodes,
            &mut path_bytes,
        )?;
        if admin != common {
            let refs = admin.join("refs");
            if checked_metadata(&refs)?.is_some() {
                refs_tree(&refs, 0, &mut evidence, &mut nodes, &mut path_bytes)?;
            } else {
                witness(&refs, &mut evidence)?;
            }
        }
        for root in [common, admin] {
            witness(&root.join("HEAD"), &mut evidence)?;
            witness(&root.join("packed-refs"), &mut evidence)?;
        }
        Ok(Self {
            common: common.to_owned(),
            admin: admin.to_owned(),
            evidence,
        })
    }
    fn recheck(&self) -> Result<bool, DevMapError> {
        Ok(Self::capture(&self.common, &self.admin)?.evidence == self.evidence)
    }
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
        let source = SourceResolutionWitness::capture(&common, &admin)?;
        let retained_bytes = [&evidence, &source.evidence]
            .into_iter()
            .flat_map(|part| part.files.values())
            .map(|entry| match entry {
                Witness::Missing => 0,
                Witness::File { bytes, .. } => bytes.len(),
            })
            .sum::<usize>();
        if retained_bytes > 4 * MAX_BYTES {
            return Err(decline());
        }
        let proof = Self {
            evidence,
            value,
            source,
        };
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
    /// Component spans of one recheck, nested inside the caller's total.
    /// Reports only static labels, elapsed microseconds, and actual Git starts.
    #[cfg(test)]
    pub(crate) fn profile_recheck_components(
        &self,
        mut report: impl FnMut(&'static str, u128, usize),
    ) -> Result<bool, DevMapError> {
        let mut count = crate::git_process::test_spawn_count();
        let mut start = std::time::Instant::now();
        let result = self.recheck_inner_observed(|stage| {
            let elapsed = start.elapsed().as_micros();
            let starts = crate::git_process::test_spawn_count() - count;
            report(stage, elapsed, starts);
            // Exclude report callback work from the next component span.
            count = crate::git_process::test_spawn_count();
            start = std::time::Instant::now();
        });
        Ok(optional(result)?.unwrap_or(false))
    }
    fn recheck_inner(&self) -> Result<bool, DevMapError> {
        self.recheck_inner_observed(|_| {})
    }
    // One shared validation path: production uses an inlinable no-op observer.
    // A declined/errored stage does not emit a completed component; the caller
    // must reject the diagnostic unless the entire proof recheck returns true.
    fn recheck_inner_observed(
        &self,
        mut completed: impl FnMut(&'static str),
    ) -> Result<bool, DevMapError> {
        if !self.source.recheck()? {
            return Ok(false);
        }
        completed("origin_proof_configuration_component_source_resolution");
        let current = environment()?;
        if current != self.evidence.environment {
            return Ok(false);
        }
        completed("origin_proof_configuration_component_environment");
        for (path, identity) in &self.evidence.directories {
            if checked_directory_identity(path)? != *identity {
                return Ok(false);
            }
        }
        completed("origin_proof_configuration_component_directories");
        let mut after = blank(current);
        for path in self.evidence.files.keys() {
            witness(path, &mut after)?;
        }
        let valid = after.files == self.evidence.files
            && after
                .directories
                .iter()
                .all(|(p, id)| self.evidence.directories.get(p) == Some(id));
        if valid {
            completed("origin_proof_configuration_component_config_witnesses");
        }
        Ok(valid)
    }
}
