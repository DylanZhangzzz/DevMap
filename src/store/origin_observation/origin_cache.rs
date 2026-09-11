//! Read-only enumeration proof owned by one InputReader. Legacy bytes are never cached here.
use super::*;
use crate::git_relationship::QueryConfiguration;

#[derive(Default)]
pub(crate) struct ReadOriginCache {
    entry: Option<Proof>,
}
struct Proof {
    source: PathBuf,
    config: QueryConfiguration,
    evidence: Evidence,
    origins: Vec<FrozenOrigin>,
}
#[derive(PartialEq, Eq, Default)]
struct Evidence {
    directories: BTreeMap<PathBuf, String>,
    files: BTreeMap<PathBuf, Option<(String, Vec<u8>)>>,
    roots: Vec<FrozenOrigin>,
    bytes: usize,
    nodes: usize,
    path_bytes: usize,
}
fn optional<T>(result: Result<T, DevMapError>) -> Result<Option<T>, DevMapError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e @ DevMapError::GitProcess(_)) => Err(e),
        Err(_) => Ok(None),
    }
}

impl ReadOriginCache {
    pub(crate) fn clear(&mut self) {
        self.entry = None;
    }
    pub(crate) fn observe(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
    ) -> Result<ActiveOriginReport, DevMapError> {
        self.observe_checked(w, c, || {})
    }
    pub(crate) fn observe_checked(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
        after_first: impl FnOnce(),
    ) -> Result<ActiveOriginReport, DevMapError> {
        let result = self.observe_inner(w, c, after_first);
        if result.is_err() {
            self.clear();
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn has_entry(&self) -> bool {
        self.entry.is_some()
    }
    fn observe_inner(
        &mut self,
        w: &SourceWorkspace,
        c: &Connection,
        after_first: impl FnOnce(),
    ) -> Result<ActiveOriginReport, DevMapError> {
        if let Some(proof) = &self.entry
            && !proof.valid(w)?
        {
            self.clear();
        }
        if self.entry.is_none() {
            self.entry = optional(Proof::acquire(w))?.flatten();
        }
        let Some(proof) = &self.entry else {
            return observe_active_origins(w, c, false);
        };
        let mut after_first = Some(after_first);
        // The immutable proof was checked at entry (or just acquired inside
        // its scan sandwich). Recheck it once after the entire observation,
        // before publishing any result. Provider calls reuse that same epoch;
        // they are not independent live scans. Legacy hashes and unavailable
        // path checks inside the observer remain unchanged.
        let result = observe_active_origins_using(w, c, false, || {
            let origins = proof.origins.clone();
            if let Some(probe) = after_first.take() {
                probe();
            }
            Ok(origins)
        })?;
        if !proof.valid(w)? {
            return Err(fail("origin enumeration proof changed after observation"));
        }
        Ok(result)
    }
}
impl Proof {
    fn acquire(w: &SourceWorkspace) -> Result<Option<Self>, DevMapError> {
        let Some(config) = QueryConfiguration::acquire(w)? else {
            return Ok(None);
        };
        let before = Evidence::capture(w)?;
        // Authoritative enumeration occurs inside the physical/config sandwich.
        let origins = current_origins(w)?;
        // Compare physical locations using checked canonical paths, retaining
        // Git's original path spelling in the returned observation.
        let mut physical_origins = origins.clone();
        for origin in &mut physical_origins {
            origin.workspace_path = safe::checked_canonical_directory(&origin.workspace_path)?;
        }
        let after = Evidence::capture(w)?;
        if before != after || before.roots != physical_origins || !config.recheck()? {
            return Ok(None);
        }
        Ok(Some(Self {
            source: w.root.clone(),
            config,
            evidence: before,
            origins,
        }))
    }
    fn valid(&self, w: &SourceWorkspace) -> Result<bool, DevMapError> {
        if self.source != w.root || !self.config.recheck()? {
            return Ok(false);
        }
        Ok(optional(Evidence::capture(w))?.is_some_and(|e| e == self.evidence))
    }
}
impl Evidence {
    fn charge_path(&mut self, path: &Path) -> Result<(), DevMapError> {
        let bytes = self
            .path_bytes
            .checked_add(path.as_os_str().as_encoded_bytes().len())
            .ok_or_else(|| fail("origin proof path byte limit"))?;
        if bytes > 1024 * 1024 {
            return Err(fail("origin proof path byte limit"));
        }
        self.path_bytes = bytes;
        Ok(())
    }
    fn directory(&mut self, path: &Path) -> Result<PathBuf, DevMapError> {
        let path = safe::checked_canonical_directory(path)?;
        self.charge_path(&path)?;
        self.directories.insert(
            path.clone(),
            safe::checked_directory_identity(&path)?.stable_text(),
        );
        if self.directories.len() > 4096 {
            return Err(fail("origin proof directory limit"));
        }
        Ok(path)
    }
    fn file(&mut self, path: &Path) -> Result<(), DevMapError> {
        self.charge_path(path)?;
        self.nodes += 1;
        if self.nodes > 4096 {
            return Err(fail("origin proof file limit"));
        }
        let value = if let Some(meta) = safe::checked_metadata(path)? {
            if !meta.is_file() || meta.len() > 1024 * 1024 {
                return Err(fail("origin proof file unsupported"));
            }
            let file = safe::checked_file(path, false, false)?;
            let identity = safe::file_identity(&file)?.stable_text();
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            self.bytes += bytes.len();
            if bytes.len() > 1024 * 1024 || self.bytes > 4 * 1024 * 1024 {
                return Err(fail("origin proof byte limit"));
            }
            if safe::file_identity(&safe::checked_file(path, false, false)?)?.stable_text()
                != identity
            {
                return Err(fail("origin proof file changed"));
            }
            Some((identity, bytes))
        } else {
            None
        };
        self.files.insert(path.to_owned(), value);
        Ok(())
    }
    fn tree(&mut self, path: &Path, depth: usize) -> Result<(), DevMapError> {
        if depth > 16 {
            return Err(fail("origin proof depth limit"));
        }
        if safe::checked_metadata(path)?.is_none() {
            return self.file(path);
        }
        self.directory(path)?;
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            self.nodes += 1;
            if self.nodes > 4096 {
                return Err(fail("origin proof entry limit"));
            }
            if safe::checked_metadata(&path)?
                .ok_or_else(|| fail("origin proof disappeared"))?
                .is_dir()
            {
                self.tree(&path, depth + 1)?;
            } else {
                self.file(&path)?;
            }
        }
        Ok(())
    }
    fn origin(&mut self, common: &Path, admin: &Path, root: &Path) -> Result<(), DevMapError> {
        let physical_root = self.directory(root)?;
        let admin = self.directory(admin)?;
        let source = SourceWorkspace {
            root: root.to_owned(),
            git_dir: admin.clone(),
            git_common_dir: common.to_owned(),
            head: String::new(),
            branch: None,
        };
        validate_origin_links(&source)?;
        for name in ["HEAD", "packed-refs", "locked", "commondir", "gitdir"] {
            self.file(&admin.join(name))?;
        }
        if safe::checked_metadata(&admin.join("config.worktree"))?.is_some() {
            return Err(fail("origin proof worktree config unsupported"));
        }
        self.file(&admin.join("config.worktree"))?;
        if admin != common {
            self.file(&root.join(".git"))?;
        }
        self.tree(&admin.join("refs"), 0)?;
        self.roots.push(FrozenOrigin {
            worktree_id: worktrees::origin_id(&worktrees::repository_id(&source), &admin),
            git_dir: admin,
            workspace_path: physical_root,
            incarnation: journal::worktree_incarnation(&source)?,
        });
        Ok(())
    }
    fn capture(w: &SourceWorkspace) -> Result<Self, DevMapError> {
        let mut result = Self::default();
        let common = result.directory(&w.git_common_dir)?;
        if common.file_name().and_then(|n| n.to_str()) != Some(".git") {
            return Err(fail("origin proof layout unsupported"));
        }
        let main = common
            .parent()
            .ok_or_else(|| fail("origin proof main missing"))?;
        // Main derivation is only a candidate until compared with original scan.
        result.origin(&common, &common, main)?;
        let objects = result.directory(&common.join("objects"))?;
        let _access = fs::read_dir(objects)?;
        let administration = common.join("worktrees");
        if safe::checked_metadata(&administration)?.is_some() {
            result.directory(&administration)?;
            let mut count = 1usize;
            for entry in fs::read_dir(&administration)? {
                count += 1;
                if count > 256 {
                    return Err(fail("origin proof worktree limit"));
                }
                let admin = result.directory(&entry?.path())?;
                let backlink = pointer_text(&admin.join("gitdir"))?;
                let backlink = PathBuf::from(backlink);
                if !backlink.is_absolute()
                    || backlink.file_name().and_then(|n| n.to_str()) != Some(".git")
                {
                    return Err(fail("origin proof backlink unsupported"));
                }
                let root = backlink
                    .parent()
                    .ok_or_else(|| fail("origin proof root missing"))?;
                result.origin(&common, &admin, root)?;
            }
        } else {
            result.file(&administration)?;
        }
        result.roots.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
        Ok(result)
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn exhausted_path_budget_refuses_existing_directory_and_absent_file() {
        let owned = tempfile::tempdir().unwrap();
        let mut directories = Evidence {
            path_bytes: 1024 * 1024,
            ..Evidence::default()
        };
        assert!(directories.directory(owned.path()).is_err());
        assert!(directories.directories.is_empty());
        let mut files = Evidence {
            path_bytes: 1024 * 1024,
            ..Evidence::default()
        };
        assert!(files.file(&owned.path().join("absent")).is_err());
        assert!(files.files.is_empty());
    }
}
