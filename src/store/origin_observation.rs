//! Read qualification for immutable SQL history; never authorizes a write.
use super::*;
#[path = "origin_observation/origin_cache.rs"]
mod origin_cache;
pub(crate) use origin_cache::ReadOriginCache;

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
    current: BTreeMap<String, FrozenOrigin>,
    pub unavailable: Vec<OriginObservation>,
    pub fingerprint: String,
}
/// Constructed only from reciprocally verified current origins. The saved root
/// pathname is historical; relocation requires the complete physical identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedCurrentOrigin(FrozenOrigin);
impl std::ops::Deref for VerifiedCurrentOrigin {
    type Target = FrozenOrigin;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl VerifiedCurrentOrigin {
    pub(crate) fn matches(&self, historical: &FrozenOrigin) -> bool {
        self.worktree_id == historical.worktree_id
            && self.git_dir == historical.git_dir
            && self.incarnation == historical.incarnation
    }
}
impl ActiveOriginReport {
    pub(crate) fn current(&self) -> &BTreeMap<String, FrozenOrigin> {
        &self.current
    }
    pub(crate) fn current_origin(&self, id: &str) -> Option<VerifiedCurrentOrigin> {
        self.current.get(id).cloned().map(VerifiedCurrentOrigin)
    }
    pub(crate) fn replaced_worktrees(&self) -> BTreeSet<String> {
        self.unavailable
            .iter()
            .filter(|observation| observation.availability == Availability::Replaced)
            .map(|observation| observation.origin.worktree_id.clone())
            .collect()
    }
}

/// Establish an application anchor without accepting caller-supplied identity.
pub(crate) fn application_anchor(
    w: &SourceWorkspace,
) -> Result<VerifiedCurrentOrigin, DevMapError> {
    let mut current = w.clone();
    current.root = safe::checked_canonical_directory(&w.root)?;
    current.git_dir = safe::checked_canonical_directory(&w.git_dir)?;
    let incarnation = journal::worktree_incarnation(&current)?;
    validate_origin_links(&current)?;
    if journal::worktree_incarnation(&current)? != incarnation {
        return Err(fail("application anchor changed during verification"));
    }
    Ok(VerifiedCurrentOrigin(FrozenOrigin {
        worktree_id: worktrees::origin_id(&worktrees::repository_id(w), &current.git_dir),
        git_dir: current.git_dir,
        workspace_path: current.root,
        incarnation,
    }))
}
impl VerifiedCurrentOrigin {
    /// Resolve the retained admin's backlink before looking at an old pathname's
    /// occupant. A returned location has already passed reciprocal verification.
    pub(crate) fn application_location(&self, common: &Path) -> Result<Option<Self>, DevMapError> {
        if !directory_present(&self.git_dir)? {
            return if !directory_present(&self.workspace_path)? {
                Ok(None)
            } else {
                Err(fail("application anchor administration disappeared"))
            };
        }
        let admin_identity = safe::checked_directory_identity(&self.git_dir)?.stable_text();
        if self.incarnation.split_once('|').map(|v| v.0) != Some(admin_identity.as_str()) {
            return Err(fail("application anchor administration replaced"));
        }
        let root = if self.git_dir == common {
            self.workspace_path.clone()
        } else {
            let backlink =
                resolve_pointer(&self.git_dir, &pointer_text(&self.git_dir.join("gitdir"))?);
            if backlink.file_name() != Some(std::ffi::OsStr::new(".git")) {
                return Err(fail("application anchor backlink basename mismatch"));
            }
            backlink
                .parent()
                .ok_or_else(|| fail("application anchor backlink has no parent"))?
                .to_owned()
        };
        let candidate = SourceWorkspace {
            root,
            git_dir: self.git_dir.clone(),
            git_common_dir: common.to_owned(),
            branch: None,
            head: String::new(),
        };
        let verified = application_anchor(&candidate)?;
        if !verified.matches(self) {
            return Err(fail("application anchor physical identity changed"));
        }
        Ok(Some(verified))
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

/// Independent read-only calls, not additive spans of one production operation.
/// Only the exact-receipt diagnostic calls this; no unavailable/moved fixture shortcut.
#[cfg(test)]
pub(crate) fn profile_inventory_workers(
    w: &SourceWorkspace,
    c: &Connection,
) -> Result<(), DevMapError> {
    let activation =
        validated_activation(w, c)?.ok_or_else(|| fail("profiling requires frozen activation"))?;
    if current_origins(w)? != activation.manifest.origins {
        return Err(fail("profiling requires exact current/frozen origins"));
    }
    inventory_parallel::profile_worker_counts(w, &activation.manifest)
}

#[cfg(test)]
pub(crate) fn profile_frozen_read_stages(
    w: &SourceWorkspace,
    c: &Connection,
    iteration: usize,
    mut report: impl FnMut(&'static str, u128, usize),
) -> Result<(), DevMapError> {
    fn stage<T>(
        label: &'static str,
        report: &mut impl FnMut(&'static str, u128, usize),
        action: impl FnOnce() -> Result<T, DevMapError>,
    ) -> Result<T, DevMapError> {
        let count = crate::git_process::test_spawn_count();
        let start = std::time::Instant::now();
        let result = action();
        report(
            label,
            start.elapsed().as_micros(),
            crate::git_process::test_spawn_count() - count,
        );
        result
    }
    let activation = stage("validated_activation", &mut report, || {
        validated_activation(w, c)
    })?
    .ok_or_else(|| fail("profiling requires frozen activation"))?;
    let origins = stage("current_origins_single", &mut report, || current_origins(w))?;
    if origins != activation.manifest.origins {
        return Err(fail("profiling requires exact current/frozen origins"));
    }
    // Alternate order to avoid always giving the candidate the second read.
    // These remain independent calls, not additive spans or an atomic snapshot.
    let mut serial = None;
    let mut captured = None;
    for use_serial in [iteration.is_multiple_of(2), !iteration.is_multiple_of(2)] {
        if use_serial {
            serial = Some(stage(
                "legacy_inventory_serial_full_hash",
                &mut report,
                || inventory_serial(w, origins.clone(), activation.manifest.evaluated_at.clone()),
            )?);
        } else {
            captured = Some(stage(
                "legacy_inventory_complete_hash",
                &mut report,
                || inventory(w, origins.clone(), activation.manifest.evaluated_at.clone()),
            )?);
        }
    }
    let captured = captured.expect("candidate inventory measured once");
    if serial.as_ref() != Some(&captured) {
        return Err(fail("profile serial and candidate inventories differ"));
    }
    validate_inventory_equality(&captured, &activation.manifest)?;
    let metadata = stage("legacy_inventory_metadata_only", &mut report, || {
        inventory_collect(
            w,
            origins.clone(),
            activation.manifest.evaluated_at.clone(),
            false,
            inventory_parallel::Limits::default(),
            None,
        )
    })?;
    let mut expected_metadata = captured.clone();
    for file in &mut expected_metadata.files {
        file.sha256.clear();
    }
    if metadata != expected_metadata {
        return Err(fail(
            "profile metadata inventory differs from full inventory",
        ));
    }
    origin_cache::profile_proof_stages(w, iteration, &mut report)?;
    profile_manifest_file_io(&captured, &mut report)?;
    let observed = stage("observe_active_origins_full", &mut report, || {
        observe_active_origins(w, c, false)
    })?;
    if !observed.unavailable.is_empty()
        || observed.current.len() != activation.manifest.origins.len()
        || activation
            .manifest
            .origins
            .iter()
            .any(|old| observed.current.get(&old.worktree_id) != Some(old))
    {
        return Err(fail(
            "profiling origin observation differs from frozen fixture",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn profile_manifest_file_io(
    manifest: &FrozenManifest,
    report: &mut impl FnMut(&'static str, u128, usize),
) -> Result<(), DevMapError> {
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};
    if manifest.files.len() > MAX_FILES {
        return Err(fail("profiling file limit"));
    }
    let start_all = Instant::now();
    let mut opening = Duration::ZERO;
    let mut parent_validation = Duration::ZERO;
    let mut file_validation = Duration::ZERO;
    let mut link_validation = Duration::ZERO;
    let mut reading = Duration::ZERO;
    let mut hashing = Duration::ZERO;
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    for expected in &manifest.files {
        let origin = manifest
            .origins
            .get(expected.origin)
            .ok_or_else(|| fail("profiling origin index"))?;
        let relative = Path::new(&expected.relative);
        if relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(fail("profiling relative path"));
        }
        let path = origin.git_dir.join("devmap").join(relative);
        let start = Instant::now();
        safe::checked_canonical_directory(path.parent().ok_or_else(|| fail("profiling parent"))?)?;
        parent_validation += start.elapsed();
        let file_start = Instant::now();
        let mut file = safe::checked_file(&path, false, false)?;
        file_validation += file_start.elapsed();
        let link_start = Instant::now();
        if super::super::link_count(&file)? != 1 {
            return Err(fail("hard-linked legacy artifact refused"));
        }
        link_validation += link_start.elapsed();
        opening += start.elapsed();
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        loop {
            let remaining = MAX_BYTES - total;
            let length = (remaining.min(buffer.len() as u64) as usize).max(1);
            let start = Instant::now();
            let result = file.read(&mut buffer[..length]);
            reading += start.elapsed();
            let count = match result {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if count == 0 {
                break;
            }
            if count as u64 > remaining {
                return Err(fail("profiling byte limit"));
            }
            bytes += count as u64;
            total += count as u64;
            let start = Instant::now();
            hash.update(&buffer[..count]);
            hashing += start.elapsed();
        }
        let start = Instant::now();
        let digest = format!("{:x}", hash.finalize());
        hashing += start.elapsed();
        if bytes != expected.bytes || digest != expected.sha256 {
            return Err(fail("profiling full file bytes/hash mismatch"));
        }
    }
    let elapsed = start_all.elapsed();
    report(
        "manifest_files_checked_open_linkcount",
        opening.as_micros(),
        0,
    );
    report("manifest_files_stream_read", reading.as_micros(), 0);
    // Nested serial observations, not additional production-query costs.
    report(
        "manifest_files_parent_validation",
        parent_validation.as_micros(),
        0,
    );
    report(
        "manifest_files_file_validation",
        file_validation.as_micros(),
        0,
    );
    report(
        "manifest_files_link_validation",
        link_validation.as_micros(),
        0,
    );
    report("manifest_files_sha_update_finalize", hashing.as_micros(), 0);
    report("manifest_files_independent_total", elapsed.as_micros(), 0);
    // Contains iteration, timing overhead, comparisons and close costs. It is
    // NOT a measurement of production walk metadata or a subtraction across runs.
    report(
        "manifest_files_other_residual",
        elapsed
            .saturating_sub(opening + reading + hashing)
            .as_micros(),
        0,
    );
    println!(
        "profile_manifest_files={} profile_manifest_bytes={total}",
        manifest.files.len()
    );
    #[cfg(windows)]
    if std::env::var("DEVMAP_QUERY_PROFILE_NOREPARSE").as_deref() == Ok("1") {
        profile_noreparse_files(manifest, report)?;
    }
    Ok(())
}

/// Same full-file corpus and hashing loop, alternating original/prototype ABBA.
/// Called only after the independent serial manifest validation above succeeds.
#[cfg(all(test, windows))]
fn profile_noreparse_files(
    manifest: &FrozenManifest,
    report: &mut impl FnMut(&'static str, u128, usize),
) -> Result<(), DevMapError> {
    for native in [false, true, true, false] {
        let start = std::time::Instant::now();
        let mut total = 0u64;
        for expected in &manifest.files {
            let path = manifest.origins[expected.origin]
                .git_dir
                .join("devmap")
                .join(&expected.relative);
            let (bytes, hash) = if native {
                let file = probe_inventory_file(&path)?;
                inventory_hash_reader(file, MAX_BYTES - total)?
            } else {
                inventory_hash(&path, MAX_BYTES - total)?
            };
            if bytes != expected.bytes || hash != expected.sha256 {
                return Err(fail("noreparse profile full file bytes/hash mismatch"));
            }
            total += bytes;
        }
        report(
            if native {
                "manifest_files_noreparse_open_full_hash"
            } else {
                "manifest_files_original_open_full_hash"
            },
            start.elapsed().as_micros(),
            0,
        );
    }
    Ok(())
}

#[cfg(all(test, windows))]
fn probe_inventory_file(path: &Path) -> Result<fs::File, DevMapError> {
    let file = safe::read_no_reparse::checked_file(path)?;
    if super::super::link_count(&file)? != 1 {
        return Err(fail("hard-linked legacy artifact refused"));
    }
    Ok(file)
}

#[cfg(all(test, windows))]
#[test]
fn noreparse_inventory_probe_rejects_both_names_of_a_hard_link() {
    let owned = tempfile::tempdir().unwrap();
    let path = owned.path().join("payload");
    let alias = owned.path().join("alias");
    fs::write(&path, b"same file").unwrap();
    assert!(probe_inventory_file(&path).is_ok());
    fs::hard_link(&path, &alias).unwrap();
    for name in [path, alias] {
        assert_eq!(
            probe_inventory_file(&name).unwrap_err().to_string(),
            fail("hard-linked legacy artifact refused").to_string()
        );
    }
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
    observe_active_origins_using(w, c, journal, || current_origins(w))
}
fn observe_active_origins_using(
    w: &SourceWorkspace,
    c: &Connection,
    journal: bool,
    mut enumerate: impl FnMut() -> Result<Vec<FrozenOrigin>, DevMapError>,
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
    let before = enumerate()?;
    let mut unavailable = Vec::new();
    let mut observed_paths = Vec::new();
    if let Some(record) = activation {
        let manifest = &record.manifest;
        let mut inspected = before.clone();
        let mut absent_admin = BTreeSet::new();
        let mut replaced_admin = BTreeSet::new();
        for old in &manifest.origins {
            if before
                .iter()
                .any(|current| VerifiedCurrentOrigin(current.clone()).matches(old))
            {
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
    let after = enumerate()?;
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
