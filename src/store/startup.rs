//! Empty-only shared-owner startup. Every failure retains its evidence.
use super::*;
use crate::{runtime::platform, store::transition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteBackend {
    ActiveSql,
    LegacyPreserved(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Attempt {
    format: String,
    nonce: String,
    state_root: PathBuf,
    manifest: FrozenManifest,
}

pub fn prepare_first_write(w: &SourceWorkspace) -> Result<WriteBackend, DevMapError> {
    prepare_first_write_for(w, None)
}

pub(crate) fn prepare_first_journal_write(
    w: &SourceWorkspace,
    session_id: &str,
) -> Result<WriteBackend, DevMapError> {
    prepare_first_write_for(w, Some(session_id))
}

fn prepare_first_write_for(
    w: &SourceWorkspace,
    journal_session: Option<&str>,
) -> Result<WriteBackend, DevMapError> {
    let guard = transition::Guard::acquire(w)?;
    let active = if journal_session.is_some() {
        // Startup selects a backend, not journal write authority. Actual open
        // and append independently validate target/session/source admission.
        match RepositoryStore::open_existing(w)? {
            Some(store) if is_active(store.connection())? => {
                validated_activation(w, store.connection())?;
                true
            }
            _ => false,
        }
    } else {
        crate::store::active_existing(w)?.is_some()
    };
    if active {
        return Ok(WriteBackend::ActiveSql);
    }
    let existing = RepositoryStore::open_existing(w)?;
    let pointer = guard.directory.join("attempt.json");
    if safe::checked_metadata(&pointer)?.is_some() {
        let attempt: Attempt = serde_json::from_slice(&read_private(&pointer)?)?;
        let store = existing
            .ok_or_else(|| fail("startup attempt has no database; explicit recovery required"))?;
        store.integrity_check()?;
        let snapshot = validate_attempt(w, &attempt)?;
        let captured = load_snapshot(w, &snapshot)?;
        if captured.manifest != attempt.manifest || !captured.files.is_empty() {
            return Err(fail("startup snapshot differs from owned empty attempt"));
        }
        revalidate(w, &attempt.manifest)?;
        validate_shadow(&store, &attempt.manifest)?;
        drop(store);
        import_shadow_guarded(w, &snapshot, &guard)?;
        activate_guarded(w, &snapshot, &guard)?;
        return Ok(WriteBackend::ActiveSql);
    }
    if existing.is_some() {
        return Err(fail("unowned shadow database requires explicit recovery"));
    }
    let origin_list = origins(w)?;
    if let Some(reason) = nonempty_reason(w, &origin_list)? {
        return Ok(WriteBackend::LegacyPreserved(reason));
    }
    let now = OffsetDateTime::now_utc();
    let manifest = inventory(
        w,
        origin_list,
        now.format(&Rfc3339).map_err(|e| fail(e.to_string()))?,
    )?;
    require_empty(&manifest)?;
    let state_root = match backup_root(w, true) {
        Ok(root) => root,
        Err(DevMapError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // No attempt, shadow or fence was created. Recheck exact source
            // before allowing the requested operation to use legacy storage.
            refuse_missing_database(w)?;
            if RepositoryStore::open_existing(w)?.is_some() {
                return Err(fail("database appeared during startup backup failure"));
            }
            revalidate(w, &manifest)?;
            return Ok(WriteBackend::LegacyPreserved(format!(
                "durable backup unavailable; legacy retained: {error}"
            )));
        }
        Err(error) => return Err(error),
    };
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| fail(e.to_string()))?;
    let nonce = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let attempt = Attempt {
        format: "devmap-empty-startup/1".into(),
        nonce,
        state_root,
        manifest,
    };
    let bytes = serde_json::to_vec(&attempt)?;
    if bytes.len() > 256 * 1024 {
        return Err(fail("startup ownership record limit"));
    }
    let directory = attempt_directory(&attempt);
    if safe::checked_metadata(&directory)?.is_some() {
        return Err(fail("startup attempt directory already exists"));
    }
    platform::private_dir(&directory)?;
    safe::sync_directory(&attempt.state_root)?;
    write_private(&directory.join("owner.json"), &bytes)?;
    // Publish ownership before freeze creates a shadow or a snapshot directory.
    write_private(&pointer, &bytes)?;
    let snapshot = validate_attempt(w, &attempt)?;
    let frozen = freeze_guarded(w, &snapshot, now, &guard)?;
    if frozen != attempt.manifest {
        return Err(fail(
            "startup empty proof changed during freeze; evidence retained",
        ));
    }
    import_shadow_guarded(w, &snapshot, &guard)?;
    activate_guarded(w, &snapshot, &guard)?;
    Ok(WriteBackend::ActiveSql)
}

fn require_empty(manifest: &FrozenManifest) -> Result<(), DevMapError> {
    if !manifest.files.is_empty() || !manifest.directories.is_empty() {
        return Err(fail(
            "startup attempt does not describe strictly empty legacy state",
        ));
    }
    Ok(())
}

fn nonempty_reason(
    w: &SourceWorkspace,
    origins: &[FrozenOrigin],
) -> Result<Option<String>, DevMapError> {
    let common = safe::checked_canonical_directory(&w.git_common_dir)?;
    let mut roots: BTreeSet<PathBuf> = origins.iter().map(|o| o.git_dir.clone()).collect();
    roots.insert(common.clone());
    let mut reason = None;
    for git_dir in roots {
        let root = git_dir.join("devmap");
        if safe::checked_metadata(&root)?.is_none() {
            continue;
        }
        safe::checked_canonical_directory(&root)?;
        for entry in fs::read_dir(&root)? {
            let path = entry?.path();
            let metadata =
                safe::checked_metadata(&path)?.ok_or_else(|| fail("startup inventory changed"))?;
            let name = path.file_name().and_then(|v| v.to_str());
            if name == Some(transition::DIRECTORY) {
                if git_dir != common {
                    return Err(fail("transition metadata outside common Git directory"));
                }
                transition::validate_directory(&path)?;
                continue;
            }
            if git_dir == common && name.is_some_and(operational_bookkeeping) {
                if !metadata.is_file() {
                    return Err(fail("invalid startup operational file"));
                }
                let file = safe::checked_file(&path, false, false)?;
                if crate::store::link_count(&file)? != 1 {
                    return Err(fail("hardlinked startup operational file"));
                }
                continue;
            }
            reason.get_or_insert_with(|| {
                format!(
                    "legacy artifact retained; explicit migration pending: {}",
                    path.display()
                )
            });
        }
    }
    Ok(reason)
}

fn state_base() -> Result<PathBuf, DevMapError> {
    #[cfg(windows)]
    let base = Some(windows_state_base(
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )?);
    #[cfg(unix)]
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".local/state")));
    let base = base.ok_or_else(|| fail("durable user state directory is unavailable"))?;
    if !base.is_absolute()
        || base
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(fail("durable user state directory must be absolute"));
    }
    Ok(base)
}

#[cfg(windows)]
fn windows_state_base(
    local: Option<PathBuf>,
    profile: Option<PathBuf>,
) -> Result<PathBuf, DevMapError> {
    let local = local.ok_or_else(|| fail("durable user state directory is unavailable"))?;
    if !local.is_absolute()
        || local
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(fail("durable user state directory must be absolute"));
    }
    if let Some(profile) = profile.filter(|p| p.is_absolute()) {
        // The conventional AppData ancestor may intentionally authorize other
        // app capabilities. Keep our default durable state under the separately
        // verified user profile. Explicitly relocated LOCALAPPDATA is respected.
        if resolve_future_directory(&local)?
            == resolve_future_directory(&profile.join("AppData/Local"))?
        {
            platform::validate_chain(&profile)?;
            return Ok(profile.join(".devmap-state"));
        }
    }
    Ok(local)
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

fn backup_root(w: &SourceWorkspace, create: bool) -> Result<PathBuf, DevMapError> {
    let base = state_base()?;
    // Resolve the future path through its nearest existing ancestor before
    // creating anything, including when an injected state root is inside Git.
    let resolved = resolve_future_directory(&base)?;
    let common = safe::checked_canonical_directory(&w.git_common_dir)?;
    if resolved.starts_with(&common) || origins(w)?.iter().any(|o| resolved.starts_with(&o.git_dir))
    {
        return Err(fail("durable state root is inside Git administration"));
    }
    if create {
        ensure_state_chain(&base)?;
    }
    platform::validate_chain(&base)?;
    #[cfg(windows)]
    let product = "DevMap";
    #[cfg(unix)]
    let product = "devmap";
    let mut root = base;
    for name in [product, "backups", &worktrees::repository_id(w)] {
        root.push(name);
        if create {
            platform::private_dir(&root)?;
            safe::sync_directory(root.parent().unwrap())?;
        } else {
            platform::validate(&root, true)?;
        }
    }
    safe::checked_canonical_directory(&root)
}

fn resolve_future_directory(path: &Path) -> Result<PathBuf, DevMapError> {
    if safe::checked_metadata(path)?.is_some() {
        return safe::checked_canonical_directory(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| fail("invalid durable state path"))?;
    Ok(resolve_future_directory(parent)?.join(
        path.file_name()
            .ok_or_else(|| fail("invalid durable state component"))?,
    ))
}

fn ensure_state_chain(path: &Path) -> Result<(), DevMapError> {
    if safe::checked_metadata(path)?.is_some() {
        safe::checked_canonical_directory(path)?;
        platform::validate_chain(path)?;
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| fail("invalid state directory"))?;
    ensure_state_chain(parent)?;
    platform::private_dir(path)?;
    safe::sync_directory(parent)?;
    Ok(())
}

fn attempt_directory(attempt: &Attempt) -> PathBuf {
    attempt
        .state_root
        .join(format!("activation-{}", attempt.nonce))
}

fn validate_attempt(w: &SourceWorkspace, attempt: &Attempt) -> Result<PathBuf, DevMapError> {
    require_empty(&attempt.manifest)?;
    if attempt.format != "devmap-empty-startup/1"
        || attempt.nonce.len() != 64
        || !attempt
            .nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || attempt.manifest.repository_id != worktrees::repository_id(w)
        || attempt.manifest.common_dir != safe::checked_canonical_directory(&w.git_common_dir)?
        || attempt.manifest.origins != origins(w)?
    {
        return Err(fail("startup attempt identity mismatch"));
    }
    // A pointer cannot select an arbitrary external restore path.
    let expected = backup_root(w, false)?;
    if attempt.state_root != expected {
        return Err(fail("startup durable state location changed"));
    }
    let directory = attempt_directory(attempt);
    platform::validate_chain(&directory)?;
    platform::validate(&directory, true)?;
    for entry in fs::read_dir(&directory)? {
        if !matches!(entry?.file_name().to_str(), Some("owner.json" | "snapshot")) {
            return Err(fail("unknown artifact in startup backup attempt"));
        }
    }
    let receipt: Attempt = serde_json::from_slice(&read_private(&directory.join("owner.json"))?)?;
    if receipt != *attempt {
        return Err(fail("startup backup ownership receipt mismatch"));
    }
    let canonical = safe::checked_canonical_directory(&directory)?;
    for origin in &attempt.manifest.origins {
        if canonical.starts_with(&origin.git_dir) {
            return Err(fail("startup backup is inside Git administration"));
        }
    }
    if canonical.starts_with(&attempt.manifest.common_dir) {
        return Err(fail("startup backup is inside common Git administration"));
    }
    Ok(directory.join("snapshot"))
}

fn read_private(path: &Path) -> Result<Vec<u8>, DevMapError> {
    platform::validate(path, false)?;
    let file = safe::checked_file(path, false, false)?;
    if crate::store::link_count(&file)? != 1 || file.metadata()?.len() > 256 * 1024 {
        return Err(fail("invalid startup ownership record"));
    }
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(fail("startup ownership record limit"));
    }
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), DevMapError> {
    let mut file = safe::checked_new_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    platform::validate(path, false)?;
    safe::sync_directory(path.parent().unwrap())?;
    Ok(())
}

fn validate_shadow(store: &RepositoryStore, manifest: &FrozenManifest) -> Result<(), DevMapError> {
    if store.generation()? != 0 || is_active(store.connection())? {
        return Err(fail("startup recovery requires an unmodified shadow"));
    }
    let c = store.connection();
    let prior: Option<(String, String, i64, String)> = c
        .query_row(
            "SELECT record_json,source_hash,record_count,outcome FROM migration_sources WHERE source_path=?1",
            [SNAPSHOT],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let imported = prior.is_some();
    if let Some((prior, hash, count, outcome)) = prior {
        if serde_json::from_str::<FrozenManifest>(&prior)? != *manifest
            || hash != sha256_hex(&serde_json::to_vec(manifest)?)
            || count != 0
            || outcome != "validated_shadow"
        {
            return Err(fail("startup shadow provenance mismatch"));
        }
        compare_domains(c, manifest, &parse(manifest, &CapturedFiles::new())?)?;
    }
    // A complete owned freeze may precede import. Both phases require an exact
    // table inventory, including rows that a per-origin comparison cannot see.
    for table in [
        "worktree_registry",
        "journal_sessions",
        "journal_records",
        "route_records",
        "presence_records",
        "binding_records",
        "binding_watermarks",
        "migration_sources",
        "presence_projection",
        "journal_heads",
    ] {
        let count: i64 = c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        let expected = if imported {
            match table {
                "worktree_registry" => manifest.origins.len() as i64,
                "migration_sources" => 1,
                _ => 0,
            }
        } else {
            0
        };
        if count != expected {
            return Err(fail("startup shadow has unowned records"));
        }
    }
    let fence_path = manifest.common_dir.join("devmap").join(FENCE);
    if safe::checked_metadata(&fence_path)?.is_some() {
        if !imported {
            return Err(fail("startup fence requires imported shadow provenance"));
        }
        let fence: ActivationFence = serde_json::from_slice(&read(&fence_path)?)?;
        if fence.format != "devmap-activation-intent/1"
            || fence.repository_id != manifest.repository_id
            || fence.snapshot_sha256 != sha256_hex(&serde_json::to_vec(manifest)?)
        {
            return Err(fail("startup activation fence mismatch"));
        }
    }
    Ok(())
}
