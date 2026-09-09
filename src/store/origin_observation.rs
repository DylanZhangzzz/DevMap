//! Read qualification for immutable SQL history; never authorizes a write.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum Availability {
    ObservedUnavailable,
    Replaced,
}
impl Availability {
    pub(crate) fn warning_code(self) -> &'static str {
        match self {
            Self::ObservedUnavailable => "legacy_origin_unavailable",
            Self::Replaced => "legacy_origin_replaced",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OriginObservation {
    pub origin: FrozenOrigin,
    pub availability: Availability,
}
pub(crate) struct ActiveOriginReport {
    pub current: BTreeMap<String, FrozenOrigin>,
    pub unavailable: Vec<OriginObservation>,
    pub fingerprint: String,
}
impl ActiveOriginReport {
    pub(crate) fn replaced_worktrees(&self) -> BTreeSet<String> {
        self.unavailable
            .iter()
            .filter(|observation| observation.availability == Availability::Replaced)
            .map(|observation| observation.origin.worktree_id.clone())
            .collect()
    }
}

fn directory_present(path: &Path) -> Result<bool, DevMapError> {
    if safe::checked_metadata(path)?.is_some() {
        safe::checked_canonical_directory(path)?;
        Ok(true)
    } else {
        // Git can remove the last administration parent too. Walk missing
        // ancestors to a reachable safe directory; never fold access/I/O errors
        // into absence. This qualifies unavailable history, not removal intent.
        let mut parent = path.parent().ok_or_else(|| fail("origin parent missing"))?;
        loop {
            if safe::checked_metadata(parent)?.is_some() {
                safe::checked_canonical_directory(parent)?;
                break;
            }
            parent = parent
                .parent()
                .ok_or_else(|| fail("origin storage root unavailable"))?;
        }
        Ok(false)
    }
}

fn current_origins(w: &SourceWorkspace) -> Result<Vec<FrozenOrigin>, DevMapError> {
    let mut result = Vec::new();
    for row in WorktreeScanner::scan(w)? {
        if !directory_present(&row.root)? {
            continue;
        }
        let origin = SourceWorkspace {
            root: row.root,
            git_dir: safe::checked_canonical_directory(&row.git_dir)?,
            git_common_dir: w.git_common_dir.clone(),
            branch: row.branch,
            head: row.head,
        };

        validate_origin_links(&origin)?;
        result.push(FrozenOrigin {
            git_dir: origin.git_dir.clone(),
            workspace_path: origin.root.clone(),
            worktree_id: row.worktree_id,
            incarnation: journal::worktree_incarnation(&origin)?,
        });
    }
    result.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
    Ok(result)
}

fn pointer_text(path: &Path) -> Result<String, DevMapError> {
    use std::io::Read;
    const LIMIT: u64 = 64 * 1024;
    let file = safe::checked_file(path, false, false)?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(fail("origin pointer exceeds limit"));
    }
    let text = String::from_utf8(bytes).map_err(|_| fail("origin pointer is not UTF8"))?;
    // Git writes a single trailing newline. Preserve embedded newlines in paths.
    let value = text.strip_suffix('\n').unwrap_or(&text);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() {
        return Err(fail("empty origin pointer"));
    }
    Ok(value.to_owned())
}

fn resolve_pointer(base: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn validate_origin_links(origin: &SourceWorkspace) -> Result<(), DevMapError> {
    let common = safe::checked_canonical_directory(&origin.git_common_dir)?;
    let root = safe::checked_canonical_directory(&origin.root)?;
    let dot_git = root.join(".git");
    let metadata = safe::checked_metadata(&dot_git)?.ok_or_else(|| fail("origin .git missing"))?;
    let actual_admin = if metadata.is_dir() {
        safe::checked_canonical_directory(&dot_git)?
    } else {
        let text = pointer_text(&dot_git)?;
        let value = text
            .strip_prefix("gitdir: ")
            .ok_or_else(|| fail("invalid origin .git pointer"))?;
        safe::checked_canonical_directory(&resolve_pointer(&root, value))?
    };
    if actual_admin != origin.git_dir {
        return Err(fail("origin .git pointer changed"));
    }
    let common_pointer = origin.git_dir.join("commondir");
    if origin.git_dir == common {
        if safe::checked_metadata(&common_pointer)?.is_some() {
            return Err(fail("main origin has unexpected commondir"));
        }
        return Ok(());
    }
    let actual_common = safe::checked_canonical_directory(&resolve_pointer(
        &origin.git_dir,
        &pointer_text(&common_pointer)?,
    ))?;
    if actual_common != common {
        return Err(fail("origin common directory mismatch"));
    }
    let admin_parent = safe::checked_canonical_directory(&common.join("worktrees"))?;
    if origin.git_dir.parent() != Some(admin_parent.as_path()) || metadata.is_dir() {
        return Err(fail("linked origin administration mismatch"));
    }
    let backlink = resolve_pointer(
        &origin.git_dir,
        &pointer_text(&origin.git_dir.join("gitdir"))?,
    );
    let parent = backlink
        .parent()
        .ok_or_else(|| fail("origin backlink has no parent"))?;
    if safe::checked_canonical_directory(parent)? != root
        || backlink.file_name() != Some(std::ffi::OsStr::new(".git"))
    {
        return Err(fail("origin gitdir backlink mismatch"));
    }
    safe::checked_file(&backlink, false, false)?;
    Ok(())
}

fn origin_inventory(
    m: &FrozenManifest,
    git: &Path,
) -> (BTreeSet<(String, String, u64)>, BTreeSet<String>) {
    (
        m.files
            .iter()
            .filter(|f| m.origins[f.origin].git_dir == git)
            .map(|f| (f.relative.clone(), f.sha256.clone(), f.bytes))
            .collect(),
        m.directories
            .iter()
            .filter(|(i, _)| m.origins[*i].git_dir == git)
            .map(|(_, path)| path.clone())
            .collect(),
    )
}

pub(crate) fn observe_active_read_origins(
    w: &SourceWorkspace,
    c: &Connection,
) -> Result<ActiveOriginReport, DevMapError> {
    observe_active_origins(w, c, false)
}

pub(crate) fn observe_active_journal_origins(
    w: &SourceWorkspace,
    c: &Connection,
) -> Result<ActiveOriginReport, DevMapError> {
    observe_active_origins(w, c, true)
}

fn observe_active_origins(
    w: &SourceWorkspace,
    c: &Connection,
    journal: bool,
) -> Result<ActiveOriginReport, DevMapError> {
    let activation = validated_activation(w, c)?;
    if journal && activation.is_none() {
        // No frozen legacy source exists. Validate this native journal target
        // using the same reciprocal pointer and physical identity rules, not a
        // repository scan of unrelated origins. Callers repeat this after SQL.
        let mut target = w.clone();
        target.git_dir = safe::checked_canonical_directory(&w.git_dir)?;
        let incarnation = journal::worktree_incarnation(&target)?;
        validate_origin_links(&target)?;
        if journal::worktree_incarnation(&target)? != incarnation {
            return Err(fail("native journal target changed during observation"));
        }
        let origin = FrozenOrigin {
            worktree_id: worktrees::origin_id(&worktrees::repository_id(w), &target.git_dir),
            git_dir: target.git_dir,
            workspace_path: target.root,
            incarnation,
        };
        return Ok(ActiveOriginReport {
            fingerprint: sha256_hex(&serde_json::to_vec(&origin)?),
            current: BTreeMap::from([(origin.worktree_id.clone(), origin)]),
            unavailable: Vec::new(),
        });
    }
    let before = current_origins(w)?;
    let mut unavailable = Vec::new();
    let mut observed_paths = Vec::new();
    if let Some(record) = activation {
        let manifest = &record.manifest;
        let mut inspected = before.clone();
        let mut absent_admin = BTreeSet::new();
        let mut replaced_admin = BTreeSet::new();
        for old in &manifest.origins {
            if before.contains(old) {
                continue;
            }
            if old.git_dir == manifest.common_dir {
                return Err(fail("common origin identity drift"));
            }
            let admin_present = directory_present(&old.git_dir)?;
            let root_present = directory_present(&old.workspace_path)?;
            let (old_admin, old_root) = old
                .incarnation
                .split_once('|')
                .ok_or_else(|| fail("invalid frozen incarnation"))?;
            let admin_identity = if admin_present {
                Some(safe::checked_directory_identity(&old.git_dir)?.stable_text())
            } else {
                None
            };
            let root_identity = if root_present {
                Some(safe::checked_directory_identity(&old.workspace_path)?.stable_text())
            } else {
                None
            };
            let same_admin = admin_identity.as_deref() == Some(old_admin);
            let replacement = before.iter().any(|current| {
                current.git_dir == old.git_dir || current.workspace_path == old.workspace_path
            }) || (admin_present && !same_admin)
                || root_identity
                    .as_deref()
                    .is_some_and(|identity| identity != old_root);
            observed_paths.push((old.git_dir.clone(), admin_identity));
            observed_paths.push((old.workspace_path.clone(), root_identity));
            unavailable.push(OriginObservation {
                origin: old.clone(),
                availability: if replacement {
                    Availability::Replaced
                } else {
                    Availability::ObservedUnavailable
                },
            });
            if !admin_present {
                absent_admin.insert(old.git_dir.clone());
            } else {
                if !same_admin {
                    replaced_admin.insert(old.git_dir.clone());
                }
                // An old admin still accessible after workspace loss must retain
                // every legacy byte; its missing workspace does not waive checks.
                if !inspected
                    .iter()
                    .any(|current| current.git_dir == old.git_dir)
                {
                    inspected.push(old.clone());
                }
            }
        }
        if !unavailable.is_empty() {
            let backup = load_snapshot(w, &record.snapshot_path)?;
            if backup.manifest != *manifest {
                return Err(fail("retained snapshot differs from activation provenance"));
            }
        }
        let current = inventory(w, inspected, manifest.evaluated_at.clone())?;
        // Reuse the exact complete inventory already captured here. Ordinary
        // journal admission retains strict global equality without rescanning
        // Git and hashing every legacy file through check_legacy_drift again.
        if journal && unavailable.is_empty() {
            validate_inventory_equality(&current, manifest)?;
        }
        for old in &manifest.origins {
            if absent_admin.contains(&old.git_dir) {
                continue;
            }
            let actual = origin_inventory(&current, &old.git_dir);
            if replaced_admin.contains(&old.git_dir) && actual.0.is_empty() && actual.1.is_empty() {
                continue;
            }
            if actual != origin_inventory(manifest, &old.git_dir) {
                return Err(fail("legacy source inventory/hash drift"));
            }
        }
        for new in &current.origins {
            if !manifest
                .origins
                .iter()
                .any(|old| old.git_dir == new.git_dir)
            {
                let actual = origin_inventory(&current, &new.git_dir);
                if !actual.0.is_empty() || !actual.1.is_empty() {
                    return Err(fail("new legacy origin has unexplained content"));
                }
            }
        }
        let administration = w.git_common_dir.join("worktrees");
        if safe::checked_metadata(&administration)?.is_some() {
            for entry in fs::read_dir(safe::checked_canonical_directory(&administration)?)? {
                let path = entry?.path();
                if safe::checked_metadata(&path.join("devmap"))?.is_some()
                    && !current.origins.iter().any(|origin| origin.git_dir == path)
                {
                    return Err(fail(
                        "unregistered historical legacy origin; explicit recovery required",
                    ));
                }
            }
        }
    }
    let after = current_origins(w)?;
    if before != after {
        return Err(fail("origin identities changed during observation"));
    }
    for (path, prior) in observed_paths {
        let current = if directory_present(&path)? {
            Some(safe::checked_directory_identity(&path)?.stable_text())
        } else {
            None
        };
        if current != prior {
            return Err(fail("unavailable origin changed during observation"));
        }
    }
    let fingerprint = sha256_hex(&serde_json::to_vec(&(&before, &unavailable))?);
    Ok(ActiveOriginReport {
        current: before
            .into_iter()
            .map(|origin| (origin.worktree_id.clone(), origin))
            .collect(),
        unavailable,
        fingerprint,
    })
}
