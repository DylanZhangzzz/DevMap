//! Filesystem-rechecked plain configuration for read-only query projection.
//! Shares low-level bounded witnesses, not relationship/ref caching behavior.
use super::*;

// A capture-local directory sandwich. Every path still traverses checked
// metadata/canonicalization. Retain the first identity, never overwrite it with
// a later identity, then reopen every distinct ancestor after all file reads.
pub(super) fn source_directory(
    path: &Path,
    evidence: &mut Evidence,
) -> Result<PathBuf, DevMapError> {
    let canonical = checked_canonical_directory(path)?;
    for ancestor in canonical.ancestors() {
        if !evidence.directories.contains_key(ancestor) {
            evidence
                .directories
                .insert(ancestor.to_owned(), checked_directory_identity(ancestor)?);
        }
    }
    Ok(canonical)
}

fn source_witness(path: &Path, evidence: &mut Evidence) -> Result<(), DevMapError> {
    witness_using_directory(path, evidence, source_directory)
}

pub(super) fn close_source_directories(evidence: &Evidence) -> Result<(), DevMapError> {
    for (path, before) in &evidence.directories {
        if &checked_directory_identity(path)? != before {
            return Err(decline());
        }
    }
    Ok(())
}

#[cfg(test)]
mod source_directory_tests {
    use super::*;

    #[test]
    fn captures_the_original_complete_file_and_ancestor_evidence() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().canonicalize().unwrap();
        let refs = root.join("refs/heads");
        std::fs::create_dir_all(&refs).unwrap();
        let mut original = blank(Vec::new());
        let mut candidate = blank(Vec::new());
        for index in 0..24 {
            let path = refs.join(format!("topic-{index}"));
            std::fs::write(&path, format!("{index:040x}\n")).unwrap();
            witness(&path, &mut original).unwrap();
            source_witness(&path, &mut candidate).unwrap();
        }
        let missing = refs.join("absent");
        witness(&missing, &mut original).unwrap();
        source_witness(&missing, &mut candidate).unwrap();
        close_source_directories(&candidate).unwrap();
        assert!(original == candidate);
    }

    #[test]
    fn later_reads_cannot_overwrite_a_replaced_ancestor_identity() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().canonicalize().unwrap();
        let refs = root.join("refs");
        std::fs::create_dir(&refs).unwrap();
        let path = refs.join("HEAD");
        std::fs::write(&path, b"same bytes\n").unwrap();
        let mut evidence = blank(Vec::new());
        source_witness(&path, &mut evidence).unwrap();
        let before = evidence.directories[&refs].clone();
        std::fs::rename(&refs, root.join("old-refs")).unwrap();
        std::fs::create_dir(&refs).unwrap();
        std::fs::write(&path, b"same bytes\n").unwrap();
        source_witness(&path, &mut evidence).unwrap();
        assert_eq!(evidence.directories[&refs], before);
        assert!(close_source_directories(&evidence).is_err());
    }

    #[test]
    fn closing_capture_rejects_a_removed_directory() {
        let owned = tempfile::tempdir().unwrap();
        let refs = owned.path().join("refs");
        std::fs::create_dir(&refs).unwrap();
        let mut evidence = blank(Vec::new());
        source_directory(&refs, &mut evidence).unwrap();
        std::fs::rename(&refs, owned.path().join("moved-refs")).unwrap();
        assert!(close_source_directories(&evidence).is_err());
    }
}

#[cfg(test)]
thread_local! {
    // All source/configuration rechecks currently run on the query caller;
    // origin-evidence workers do not execute SourceResolutionWitness::capture.
    static SOURCE_CAPTURE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone)]
pub(crate) struct QueryConfiguration {
    evidence: std::sync::Arc<Evidence>,
    value: Option<String>,
    source: std::sync::Arc<SourceResolutionWitness>,
    root: PathBuf,
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
        #[cfg(test)]
        SOURCE_CAPTURE_COUNT.with(|count| count.set(count.get() + 1));
        let mut evidence = blank(Vec::new());
        source_directory(common, &mut evidence)?;
        source_directory(admin, &mut evidence)?;
        let objects = source_directory(&common.join("objects"), &mut evidence)?;
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
            source_directory(path, evidence)?;
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
                    source_witness(&path, evidence)?;
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
                source_witness(&refs, &mut evidence)?;
            }
        }
        for root in [common, admin] {
            source_witness(&root.join("HEAD"), &mut evidence)?;
            source_witness(&root.join("packed-refs"), &mut evidence)?;
        }
        close_source_directories(&evidence)?;
        Ok(Self {
            common: common.to_owned(),
            admin: admin.to_owned(),
            evidence,
        })
    }
    fn recheck_against(&self, peer: Option<&Self>) -> Result<bool, DevMapError> {
        let current = Self::capture(&self.common, &self.admin)?;
        let own_matches = current.evidence == self.evidence;
        let peer_matches = peer.is_none_or(|other| current.evidence == other.evidence);
        Ok(own_matches && peer_matches)
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
    #[cfg(test)]
    pub(crate) fn test_reset_source_capture_count() {
        SOURCE_CAPTURE_COUNT.with(|count| count.set(0));
    }
    #[cfg(test)]
    pub(crate) fn test_source_capture_count() -> usize {
        SOURCE_CAPTURE_COUNT.with(std::cell::Cell::get)
    }

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
        (evidence.system, evidence.global) = configuration_paths::capture(&workspace.root)?;
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
        let mut development_target = None;
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
            // Git emits the effective scope order and normalizes variable names.
            // For these admitted string values, --get selects the last occurrence.
            // Empty strings and spaces are significant; control characters and
            // valueless entries already decline to the original projection path.
            if key == "devmap.developmenttarget" {
                development_target = Some(value.to_owned());
            }
        }
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
            evidence: std::sync::Arc::new(evidence),
            value: development_target,
            source: std::sync::Arc::new(source),
            root,
        };
        if !proof.recheck()? {
            return Err(decline());
        }
        Ok(proof)
    }
    pub(crate) fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
    pub(crate) fn matches_workspace(&self, workspace: &SourceWorkspace) -> bool {
        // Windows inspectors and checked paths can spell the same location
        // differently (including the verbatim prefix). Qualify every supplied
        // directory before comparing; a different linked worktree cannot match.
        [
            (&self.root, &workspace.root),
            (&self.source.admin, &workspace.git_dir),
            (&self.source.common, &workspace.git_common_dir),
        ]
        .into_iter()
        .all(|(expected, supplied)| {
            checked_canonical_directory(supplied).is_ok_and(|actual| actual == *expected)
        })
    }
    // Eligibility only. Callers separately bind exact workspace keys/seals.
    pub(crate) fn same_baseline(&self, other: &Self) -> bool {
        self.root == other.root
            && self.value == other.value
            && self.evidence == other.evidence
            && self.source.common == other.source.common
            && self.source.admin == other.source.admin
            && self.source.evidence == other.source.evidence
    }
    // Each fresh stage is compared to both retained baselines, never to a
    // previously returned successful boolean. Source bytes die before config IO.
    pub(crate) fn recheck_pair(&self, other: &Self) -> Result<bool, DevMapError> {
        if !self.same_baseline(other) {
            return Ok(false);
        }
        Ok(optional(self.recheck_observed_against(Some(other), |_| {}))?.unwrap_or(false))
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
        completed: impl FnMut(&'static str),
    ) -> Result<bool, DevMapError> {
        self.recheck_observed_against(None, completed)
    }
    fn recheck_observed_against(
        &self,
        peer: Option<&Self>,
        mut completed: impl FnMut(&'static str),
    ) -> Result<bool, DevMapError> {
        if !self
            .source
            .recheck_against(peer.map(|other| other.source.as_ref()))?
        {
            return Ok(false);
        }
        completed("origin_proof_configuration_component_source_resolution");
        let current = environment()?;
        let own_matches = current == self.evidence.environment;
        let peer_matches = peer.is_none_or(|other| current == other.evidence.environment);
        if !own_matches || !peer_matches {
            return Ok(false);
        }
        completed("origin_proof_configuration_component_environment");
        for (path, identity) in &self.evidence.directories {
            let current = checked_directory_identity(path)?;
            let own_matches = current == *identity;
            let peer_matches =
                peer.is_none_or(|other| other.evidence.directories.get(path) == Some(&current));
            if !own_matches || !peer_matches {
                return Ok(false);
            }
        }
        completed("origin_proof_configuration_component_directories");
        let mut after = blank(current);
        for path in self.evidence.files.keys() {
            witness(path, &mut after)?;
        }
        let matches = |baseline: &Evidence| {
            after.files == baseline.files
                && after
                    .directories
                    .iter()
                    .all(|(p, id)| baseline.directories.get(p) == Some(id))
        };
        let own_matches = matches(&self.evidence);
        let peer_matches = peer.is_none_or(|other| matches(&other.evidence));
        let valid = own_matches && peer_matches;
        if valid {
            completed("origin_proof_configuration_component_config_witnesses");
        }
        Ok(valid)
    }
}
