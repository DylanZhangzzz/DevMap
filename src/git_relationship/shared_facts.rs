//! Guarded successful relationship facts shared only within one operation.
mod configuration_paths;
use crate::git_process::probe_overlap;
mod query_configuration;
pub(crate) use query_configuration::QueryConfiguration;

#[cfg(test)]
mod integration_tests;

use super::{DevMapError, SourceWorkspace, WorktreeDescriptor};

use crate::fs_security::{
    FileIdentity, checked_canonical_directory, checked_directory_identity, checked_file,
    checked_metadata, file_identity,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_BYTES: usize = 1_048_576;
const MAX_REFS: usize = 2048;

// An opening observation, not permission to publish shared results. compute()
// retains every result locally until the complete closing recheck succeeds.
struct SharedFactsCandidate {
    caller: SourceWorkspace,
    worktrees: Vec<WorktreeDescriptor>,
    evidence: Evidence,
}

#[derive(PartialEq, Eq)]
struct Evidence {
    environment: Vec<(OsString, OsString)>,
    directories: BTreeMap<PathBuf, FileIdentity>,
    files: BTreeMap<PathBuf, Witness>,
    system: Vec<u8>,
    global: Vec<u8>,
    config: Vec<u8>,
    refs: Vec<u8>,
    remote_head: Vec<u8>,
}

#[derive(PartialEq, Eq)]
enum Witness {
    Missing,
    File {
        identity: FileIdentity,
        bytes: Vec<u8>,
    },
}

#[track_caller]
fn decline() -> DevMapError {
    #[cfg(test)]
    eprintln!(
        "sharing eligibility declined at {}",
        std::panic::Location::caller()
    );
    DevMapError::MalformedAdapterConfig("relationship sharing is not eligible".into())
}

// Eligibility cannot absorb supervisor failures into an ordinary retry path.
fn optional<T>(result: Result<T, DevMapError>) -> Result<Option<T>, DevMapError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error @ DevMapError::GitProcess(_)) => Err(error),
        Err(error) => {
            #[cfg(test)]
            eprintln!("optional sharing proof declined: {error}");
            let _ = error;
            Ok(None)
        }
    }
}

fn acquire(
    caller: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
) -> Result<Option<SharedFactsCandidate>, DevMapError> {
    crate::git_process::with_operation(|| {
        let Some(evidence) = optional(capture(caller, worktrees))? else {
            return Ok(None);
        };
        // The full closing capture is performed after the representative Git
        // reads and tag witnesses, before any shared result leaves compute().
        // A second pre-computation capture adds no closing coverage to those
        // reads. Opening capture still closes every observed directory identity.
        Ok(Some(SharedFactsCandidate {
            caller: caller.clone(),
            worktrees: worktrees.to_vec(),
            evidence,
        }))
    })
}

impl SharedFactsCandidate {
    fn recheck(&self) -> Result<bool, DevMapError> {
        crate::git_process::with_operation(|| {
            Ok(optional(capture(&self.caller, &self.worktrees))?
                .is_some_and(|now| now == self.evidence))
        })
    }
}

fn probe(root: &Path, args: &[&str]) -> Result<Vec<u8>, DevMapError> {
    let output = super::git_output(root, args)?;
    if !output.status.success() || !output.stderr.is_empty() || output.stdout.len() > MAX_BYTES {
        return Err(decline());
    }
    Ok(output.stdout)
}

fn directory(path: &Path, evidence: &mut Evidence) -> Result<PathBuf, DevMapError> {
    let canonical = checked_canonical_directory(path)?;
    // Keep every existing ancestor, including parents of absent candidates.
    for ancestor in canonical.ancestors() {
        evidence.directories.insert(
            ancestor.to_path_buf(),
            checked_directory_identity(ancestor)?,
        );
    }
    Ok(canonical)
}

fn witness(path: &Path, evidence: &mut Evidence) -> Result<(), DevMapError> {
    witness_using_directory(path, evidence, directory)
}

fn witness_using_directory(
    path: &Path,
    evidence: &mut Evidence,
    directory: fn(&Path, &mut Evidence) -> Result<PathBuf, DevMapError>,
) -> Result<(), DevMapError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(decline());
    }
    let mut parent = path.parent().ok_or_else(decline)?;
    while checked_metadata(parent)?.is_none() {
        parent = parent.parent().ok_or_else(decline)?;
    }
    directory(parent, evidence)?;
    let value = match checked_metadata(path)? {
        None => Witness::Missing,
        Some(meta) if meta.is_file() && meta.len() <= MAX_BYTES as u64 => {
            let file = checked_file(path, false, false)?;
            let identity = file_identity(&file)?;
            let mut bytes = Vec::new();
            file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
            if bytes.len() > MAX_BYTES {
                return Err(decline());
            }
            // Reopen after the read so replacement during reading is not ignored.
            if file_identity(&checked_file(path, false, false)?)? != identity {
                return Err(decline());
            }
            Witness::File { identity, bytes }
        }
        Some(_) => return Err(decline()),
    };
    evidence.files.insert(path.to_path_buf(), value);
    if evidence
        .files
        .values()
        .map(|v| match v {
            Witness::File { bytes, .. } => bytes.len(),
            Witness::Missing => 0,
        })
        .sum::<usize>()
        > 4 * MAX_BYTES
    {
        return Err(decline());
    }
    Ok(())
}

fn absent(path: &Path, evidence: &mut Evidence) -> Result<(), DevMapError> {
    absent_using_directory(path, evidence, directory)
}

fn absent_using_directory(
    path: &Path,
    evidence: &mut Evidence,
    directory: fn(&Path, &mut Evidence) -> Result<PathBuf, DevMapError>,
) -> Result<(), DevMapError> {
    witness_using_directory(path, evidence, directory)?;
    if !matches!(evidence.files.get(path), Some(Witness::Missing)) {
        return Err(decline());
    }
    Ok(())
}

fn pointer_using_directory(
    path: &Path,
    prefix: &str,
    evidence: &mut Evidence,
    directory: fn(&Path, &mut Evidence) -> Result<PathBuf, DevMapError>,
) -> Result<PathBuf, DevMapError> {
    witness_using_directory(path, evidence, directory)?;
    let Some(Witness::File { bytes, .. }) = evidence.files.get(path) else {
        return Err(decline());
    };
    if bytes.len() > 32768 {
        return Err(decline());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| decline())?
        .trim_end_matches(['\r', '\n']);
    let value = text.strip_prefix(prefix).ok_or_else(decline)?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(decline());
    }
    let target = PathBuf::from(value);
    Ok(if target.is_absolute() {
        target
    } else {
        path.parent().ok_or_else(decline)?.join(target)
    })
}

fn candidate_paths(raw: &[u8]) -> Result<Vec<PathBuf>, DevMapError> {
    if raw.len() > 32768 {
        return Err(decline());
    }
    let text = std::str::from_utf8(raw).map_err(|_| decline())?;
    let paths: Vec<_> = text.lines().map(PathBuf::from).collect();
    if paths.is_empty()
        || paths.len() > 8
        || paths.iter().any(|p| {
            !p.is_absolute()
                || p.as_os_str()
                    .to_string_lossy()
                    .chars()
                    .any(char::is_control)
        })
    {
        return Err(decline());
    }
    Ok(paths)
}

fn admitted_key(key: &str, value: &str) -> bool {
    let boolean = || {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "0" | "1"
        )
    };
    match key {
        // Ordinary file-backed repository only; no worktree/ref/object backend extensions.
        "core.repositoryformatversion" => value == "0",
        "core.bare" => matches!(
            value.to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        ),
        // These affect index/worktree handling, which is still independently queried.
        "core.filemode"
        | "core.ignorecase"
        | "core.symlinks"
        | "core.fscache"
        | "core.logallrefupdates"
        | "submodule.recurse" => boolean(),
        "core.autocrlf" => boolean() || value == "input",
        // Identity is irrelevant to immutable object reads; all values remain guarded.
        "user.name" | "user.email" => true,
        // show -s does not produce a diff or invoke textconv; graph reads do not
        // checkout/add files and never execute these LFS filter commands.
        "diff.astextplain.textconv"
        | "filter.lfs.clean"
        | "filter.lfs.smudge"
        | "filter.lfs.process" => true,
        "filter.lfs.required" => boolean(),
        // No fetch/network/authentication, pull, or init occurs in shared facts.
        "http.sslbackend" | "http.sslcainfo" | "credential.helper" | "init.defaultbranch"
        | "remote.origin.url" => true,
        "credential.https://dev.azure.com.usehttppath" => boolean(),
        "pull.rebase" => boolean() || matches!(value, "merges" | "interactive"),
        "remote.origin.fetch" => value == "+refs/heads/*:refs/remotes/origin/*",
        // No broad core.*, remote.*, credential.*, filter.* or include admission.
        _ => false,
    }
}

fn validate_config(
    raw: &[u8],
    candidates: &[(PathBuf, &'static str)],
    root: &Path,
) -> Result<(), DevMapError> {
    let text = std::str::from_utf8(raw).map_err(|_| decline())?;
    let fields: Vec<_> = text.split_terminator('\0').collect();
    if !raw.ends_with(&[0]) || !fields.len().is_multiple_of(3) || fields.len() > 6144 {
        return Err(decline());
    }
    for row in fields.chunks_exact(3) {
        let origin = row[1].strip_prefix("file:").ok_or_else(decline)?;
        // Git uses this exact relative spelling for a main checkout's local
        // config. All other origins must be absolute discovered candidates.
        let origin = if origin == ".git/config" {
            root.join(origin)
        } else {
            PathBuf::from(origin)
        };
        if !origin.is_absolute() {
            return Err(decline());
        }
        let canonical = std::fs::canonicalize(&origin)?;
        if !candidates.iter().any(|(path, scope)| {
            *scope == row[0] && std::fs::canonicalize(path).ok().as_ref() == Some(&canonical)
        }) {
            return Err(decline());
        }
        let (key, value) = row[2].split_once('\n').ok_or_else(decline)?;
        if key.len() > 512
            || value.len() > 32768
            || value.chars().any(char::is_control)
            || !admitted_key(key, value)
        {
            #[cfg(test)]
            eprintln!("unreviewed config key: {key}");
            return Err(decline());
        }
    }
    Ok(())
}

fn capture(
    caller: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
) -> Result<Evidence, DevMapError> {
    capture_using_directory(
        caller,
        worktrees,
        query_configuration::source_directory,
        query_configuration::close_source_directories,
    )
}

fn capture_using_directory(
    caller: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
    directory: fn(&Path, &mut Evidence) -> Result<PathBuf, DevMapError>,
    close: fn(&Evidence) -> Result<(), DevMapError>,
) -> Result<Evidence, DevMapError> {
    let witness =
        |path: &Path, evidence: &mut Evidence| witness_using_directory(path, evidence, directory);
    let absent =
        |path: &Path, evidence: &mut Evidence| absent_using_directory(path, evidence, directory);
    let pointer = |path: &Path, prefix: &str, evidence: &mut Evidence| {
        pointer_using_directory(path, prefix, evidence, directory)
    };
    if worktrees.is_empty() || worktrees.len() > 256 {
        return Err(decline());
    }
    let mut environment: Vec<_> = std::env::vars_os().collect();
    environment.sort();
    for (key, value) in &environment {
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
                // Git explicitly disables its pager for these exact values;
                // neither value names a command to execute or changes facts.
                "GIT_PAGER" => value.is_empty() || value == "cat",
                // Diagnostic sinks do not modify revision traversal/config resolution.
                "GIT_TRACE" | "GIT_TRACE2" | "GIT_TRACE2_EVENT" | "GIT_TRACE2_PERF" => true,
                _ => false,
            }
        {
            return Err(decline());
        }
    }
    let mut evidence = Evidence {
        environment,
        directories: BTreeMap::new(),
        files: BTreeMap::new(),
        system: Vec::new(),
        global: Vec::new(),
        config: Vec::new(),
        refs: Vec::new(),
        remote_head: Vec::new(),
    };
    let common = directory(&caller.git_common_dir, &mut evidence)?;
    let caller_root = directory(&caller.root, &mut evidence)?;
    let caller_admin = directory(&caller.git_dir, &mut evidence)?;
    let mut seen = BTreeSet::new();
    let mut found_caller = false;
    for row in worktrees {
        if row.is_bare || row.is_prunable || !super::valid_object_id(&row.head) {
            return Err(decline());
        }
        let root = directory(&row.root, &mut evidence)?;
        let admin = directory(&row.git_dir, &mut evidence)?;
        if !seen.insert(admin.clone()) {
            return Err(decline());
        }
        if crate::worktrees::origin_id(&crate::worktrees::repository_id(caller), &admin)
            != row.worktree_id
        {
            return Err(decline());
        }
        found_caller |= root == caller_root && admin == caller_admin;
        if admin == common {
            if directory(&root.join(".git"), &mut evidence)? != common {
                return Err(decline());
            }
            absent(&admin.join("commondir"), &mut evidence)?;
        } else {
            let admins = directory(&common.join("worktrees"), &mut evidence)?;
            if admin.parent() != Some(admins.as_path())
                || directory(
                    &pointer(&root.join(".git"), "gitdir: ", &mut evidence)?,
                    &mut evidence,
                )? != admin
            {
                return Err(decline());
            }
            if directory(
                &pointer(&admin.join("commondir"), "", &mut evidence)?,
                &mut evidence,
            )? != common
            {
                return Err(decline());
            }
            let backlink = pointer(&admin.join("gitdir"), "", &mut evidence)?;
            if backlink.file_name() != Some(std::ffi::OsStr::new(".git"))
                || directory(backlink.parent().ok_or_else(decline)?, &mut evidence)? != root
            {
                return Err(decline());
            }
        }
        witness(&admin.join("HEAD"), &mut evidence)?;
        absent(&admin.join("config.worktree"), &mut evidence)?;
    }
    if !found_caller {
        return Err(decline());
    }
    directory(&common.join("objects"), &mut evidence)?;
    for relative in [
        "shallow",
        "info/grafts",
        "objects/info/alternates",
        "objects/info/http-alternates",
        "config.worktree",
    ] {
        absent(&common.join(relative), &mut evidence)?;
    }
    let packs = common.join("objects/pack");
    directory(&packs, &mut evidence)?;
    let mut count = 0;
    for entry in std::fs::read_dir(&packs)? {
        count += 1;
        if count > 4096 {
            return Err(decline());
        }
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "promisor")
        {
            return Err(decline());
        }
        if checked_metadata(&entry.path())?.is_none_or(|m| !m.is_file()) {
            return Err(decline());
        }
    }
    (evidence.system, evidence.global) = configuration_paths::capture(&caller.root)?;
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
    for (path, _) in &candidates {
        witness(path, &mut evidence)?;
    }
    // All candidate files have been witnessed. Only independent Git reads
    // overlap; consume configuration and ref validation in their original order.
    let (_, remote_head) = probe_overlap::pair(
        || {
            let (config, refs) = probe_overlap::pair(
                || {
                    let config = probe(
                        &caller.root,
                        &[
                            "config",
                            "--no-includes",
                            "--null",
                            "--list",
                            "--show-origin",
                            "--show-scope",
                        ],
                    )?;
                    validate_config(&config, &candidates, &caller.root)?;
                    Ok(config)
                },
                || {
                    probe(
                        &caller.root,
                        &[
                            "for-each-ref",
                            "--count=2049",
                            "--format=%(refname)%00%(objectname)%00%(symref)",
                        ],
                    )
                },
                #[cfg(test)]
                false,
            )?;
            evidence.config = config;
            evidence.refs = refs;
            let refs = std::str::from_utf8(&evidence.refs).map_err(|_| decline())?;
            if refs.lines().count() > MAX_REFS {
                return Err(decline());
            }
            let mut ref_oids = BTreeMap::new();
            for line in refs.lines() {
                let columns: Vec<_> = line.split('\0').collect();
                if columns.len() != 3
                    || !super::valid_object_id(columns[1])
                    || columns[0].len() > 1024
                    || columns[0].chars().any(char::is_control)
                    || !(columns[0].starts_with("refs/heads/")
                        || columns[0].starts_with("refs/remotes/")
                        || columns[0].starts_with("refs/tags/"))
                    || !(columns[2].is_empty()
                        || columns[0] == "refs/remotes/origin/HEAD"
                            && columns[2].starts_with("refs/remotes/origin/"))
                {
                    return Err(decline());
                }
                if ref_oids.insert(columns[0], columns[1]).is_some() {
                    return Err(decline());
                }
            }
            for row in worktrees {
                let admin = checked_canonical_directory(&row.git_dir)?;
                let Some(Witness::File { bytes, .. }) = evidence.files.get(&admin.join("HEAD"))
                else {
                    return Err(decline());
                };
                let head = std::str::from_utf8(bytes)
                    .map_err(|_| decline())?
                    .trim_end_matches(['\r', '\n']);
                let resolved = match &row.branch {
                    Some(branch) => {
                        let expected = format!("refs/heads/{branch}");
                        if head.strip_prefix("ref: ") != Some(expected.as_str()) {
                            return Err(decline());
                        }
                        ref_oids
                            .get(expected.as_str())
                            .copied()
                            .ok_or_else(decline)?
                    }
                    None if super::valid_object_id(head) => head,
                    None => return Err(decline()),
                };
                if resolved != row.head {
                    return Err(decline());
                }
            }
            Ok(())
        },
        || {
            super::git_output(
                &caller.root,
                ["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
            )
        },
        #[cfg(test)]
        false,
    )?;
    if !remote_head.stderr.is_empty() || remote_head.stdout.len() > 2048 {
        return Err(decline());
    }
    match remote_head.status.code() {
        Some(0) => {
            let value = std::str::from_utf8(&remote_head.stdout)
                .map_err(|_| decline())?
                .trim_end_matches('\n');
            if !value.starts_with("refs/remotes/origin/") || value.chars().any(char::is_control) {
                return Err(decline());
            }
            evidence.remote_head = remote_head.stdout;
        }
        Some(1) if remote_head.stdout.is_empty() => {}
        _ => return Err(decline()),
    }
    close(&evidence)?;
    Ok(evidence)
}

impl SharedFactsCandidate {
    fn direct_ref_oid(&self, reference: &str) -> Option<&str> {
        if !(reference.starts_with("refs/heads/") || reference.starts_with("refs/remotes/")) {
            return None;
        }
        std::str::from_utf8(&self.evidence.refs)
            .ok()?
            .lines()
            .find_map(|line| {
                let mut fields = line.split('\0');
                let name = fields.next()?;
                let oid = fields.next()?;
                let symbolic = fields.next()?;
                (name == reference && symbolic.is_empty()).then_some(oid)
            })
    }
}

/// Successful facts only, local to one resolver invocation. Status stays per root.
pub(super) fn compute(
    caller: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
    candidates: &[&WorktreeDescriptor],
    root: Option<&super::DevelopmentTarget>,
    development: Option<&super::DevelopmentTarget>,
    integration: &[super::IntegrationBranch],
    #[cfg(test)] mut fault: Option<super::SharedFactsTestFault>,
) -> Result<(BTreeMap<String, super::GitRelationship>, usize), DevMapError> {
    let empty = || (BTreeMap::new(), 0);
    // Avoid expensive guard probes for synthetic cloned-row benchmarks and for
    // roots that cannot share any immutable head. This is eligibility only.
    let mut heads = BTreeMap::<&str, BTreeSet<(&Path, &Path)>>::new();
    for row in candidates {
        if super::target_for_worktree(row, root, development).is_some() {
            heads
                .entry(&row.head)
                .or_default()
                .insert((&row.root, &row.git_dir));
        }
    }
    if !heads.values().any(|rows| {
        rows.iter()
            .map(|(root, _)| root)
            .collect::<BTreeSet<_>>()
            .len()
            > 1
            && rows
                .iter()
                .map(|(_, admin)| admin)
                .collect::<BTreeSet<_>>()
                .len()
                > 1
    }) {
        return Ok(empty());
    }
    let Some(proof) = acquire(caller, worktrees)? else {
        return Ok(empty());
    };
    #[cfg(test)]
    if matches!(fault, Some(super::SharedFactsTestFault::TagAfterCapture)) {
        // Both representative and tag rereads see this new tag. Only the full
        // closing evidence comparison can reject the older opening observation.
        super::required_output(&proof.caller.root, ["tag", "changed-after-shared-capture"])?;
        fault = None;
    }
    let mut groups =
        BTreeMap::<(String, String), Vec<(&WorktreeDescriptor, &super::DevelopmentTarget)>>::new();
    for row in candidates {
        let Some(target) = super::target_for_worktree(row, root, development) else {
            continue;
        };
        let Some(oid) = proof.direct_ref_oid(&target.ref_name) else {
            continue;
        };
        // The immutable operand must agree with the target already selected for
        // the public integration evidence, not a later independently moved ref.
        if !integration
            .iter()
            .any(|branch| branch.ref_name == target.ref_name && branch.head == oid)
        {
            continue;
        }
        groups
            .entry((row.head.clone(), oid.to_owned()))
            .or_default()
            .push((row, target));
    }
    let mut results = BTreeMap::new();
    let mut witnesses = BTreeMap::<String, Vec<u8>>::new();
    let mut completed = 0;
    for ((_, oid), rows) in groups {
        if rows
            .iter()
            .map(|(row, _)| &row.root)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
            || rows
                .iter()
                .map(|(row, _)| &row.git_dir)
                .collect::<BTreeSet<_>>()
                .len()
                < 2
        {
            continue;
        }
        let mut representative = rows[0].0.clone();
        representative.root = proof.caller.root.clone();
        representative.git_dir = proof.caller.git_dir.clone();
        let mut target = rows[0].1.clone();
        target.ref_name = oid;
        let mut tags = Vec::new();
        let mut observe = || {
            super::relationship_for_observed(
                &representative,
                Some(&target),
                false,
                0,
                Some(&mut tags),
            )
        };
        #[cfg(not(test))]
        let observed = observe();
        #[cfg(test)]
        let observed = {
            let representative_fault =
                if matches!(fault, Some(super::SharedFactsTestFault::TagBeforeRecheck)) {
                    None
                } else {
                    fault.take()
                };
            match representative_fault {
                Some(super::SharedFactsTestFault::OrdinaryRepresentativeFailure) => {
                    Err(DevMapError::GitCommand {
                        command: "injected representative attempt".into(),
                        stderr: "ordinary failure routing test".into(),
                    })
                }
                Some(super::SharedFactsTestFault::RepresentativeDeadline) => {
                    // Acquire the real proof first, then exercise real Git
                    // admission with an expired, strictly shorter local budget.
                    let budget =
                        crate::git_process::GitBudget::new(std::time::Duration::from_millis(10));
                    crate::git_process::with_budget(&budget, || {
                        std::thread::sleep(
                            budget
                                .deadline()
                                .saturating_duration_since(std::time::Instant::now()),
                        );
                        observe()
                    })
                }
                _ => observe(),
            }
        };
        let Some((facts, warning)) = optional(observed)? else {
            continue;
        };
        if warning.is_some()
            || facts.merged.is_none()
            || facts.ahead.is_none()
            || facts.behind.is_none()
            || facts.fork_point.is_none()
        {
            continue;
        }
        let base = facts
            .fork_point
            .as_ref()
            .expect("checked complete facts")
            .commit
            .clone();
        if let Some(previous) = witnesses.insert(base, tags.clone())
            && previous != tags
        {
            return Ok(empty());
        }
        for (row, target) in rows {
            let mut assembled = facts.clone();
            assembled.base_target = Some(target.name.clone());
            assembled.merge_target = Some(target.name.clone());
            assembled
                .fork_point
                .as_mut()
                .expect("checked complete facts")
                .target_branch = target.name.clone();
            results.insert(row.worktree_id.clone(), assembled);
        }
        completed += 1;
    }
    if results.is_empty() {
        return Ok(empty());
    }
    #[cfg(test)]
    if matches!(fault, Some(super::SharedFactsTestFault::TagBeforeRecheck)) {
        // Real mutation is confined to the explicitly passed test invocation's
        // owned fixture. Production builds contain neither this hook nor flag.
        super::required_output(&proof.caller.root, ["tag", "changed-before-shared-recheck"])?;
    }
    // Compare the exact untruncated tag bytes actually consumed above, not a
    // reconstructed sorted/truncated display list. A changed guard falls back once.
    for (base, before) in witnesses {
        let Some(after) = optional(super::required_output(
            &proof.caller.root,
            ["tag", "--points-at", base.as_str()],
        ))?
        else {
            return Ok(empty());
        };
        if after.stdout != before {
            return Ok(empty());
        }
    }
    if !proof.recheck()? {
        return Ok(empty());
    }
    Ok((results, completed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::SourceGitInspector;
    use crate::worktrees::WorktreeScanner;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const CHILD_CASE: &str = "DEVMAP_SHARED_FACTS_TEST_CASE";
    const CHILD_ROOT: &str = "DEVMAP_SHARED_FACTS_TEST_ROOT";

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    fn isolated(case: &str) {
        if std::env::var(CHILD_CASE).as_deref() == Ok(case) {
            exercise(case);
            return;
        }
        let owned = tempfile::tempdir().unwrap();
        // Keep tempfile's native absolute spelling for Git on Windows; the
        // canonical \\?\ spelling is reserved for ownership comparisons.
        let root = owned.path().to_path_buf();
        assert!(root.is_absolute());
        let home = root.join("home");
        let xdg = root.join("xdg");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&xdg).unwrap();
        let stdout_path = root.join("stdout");
        let stderr_path = root.join("stderr");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            &format!("git_relationship::shared_facts::tests::{case}"),
            "--nocapture",
            "--test-threads=1",
        ]);
        // Child-only isolation. Actual system configuration remains read-only and
        // participates in the positive proof; no guessed Git installation paths.
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        command
            .env(CHILD_CASE, case)
            .env(CHILD_ROOT, &root)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &xdg)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout_path).unwrap())
            .stderr(fs::File::create(&stderr_path).unwrap());
        if case == "semantic_environment_declines" {
            command.env("GIT_NAMESPACE", "sharing-test");
        }
        if case == "cat_pager_is_eligible" {
            command.env("GIT_PAGER", "cat");
        }
        if case == "empty_pager_is_eligible" {
            command.env("GIT_PAGER", "");
        }
        let mut child = OwnedChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(90);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "owned test child timed out: {case}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            status.success(),
            "{case}: {status}\n{}\n{}",
            fs::read_to_string(stdout_path).unwrap(),
            fs::read_to_string(stderr_path).unwrap()
        );
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let output = super::super::git_output(root, args).unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    }

    fn exercise(case: &str) {
        if case == "cat_pager_is_eligible" {
            assert_eq!(std::env::var("GIT_PAGER").as_deref(), Ok("cat"));
        }
        if case == "empty_pager_is_eligible" {
            assert_eq!(std::env::var("GIT_PAGER").as_deref(), Ok(""));
        }
        let owned = PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
        let canonical_owned = owned.canonicalize().unwrap();
        let main = owned.join("main");
        let linked = owned.join("linked");
        fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        git(
            &main,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "base",
            ],
        );
        git(&main, &["branch", "dev"]);
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "topic",
                linked.to_str().unwrap(),
            ],
        );
        let caller = SourceGitInspector::open(&main)
            .unwrap()
            .workspace()
            .unwrap();
        let worktrees = WorktreeScanner::scan(&caller).unwrap();
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].head, worktrees[1].head);
        if case == "capture_directory_sandwich_matches_original_evidence" {
            let original =
                capture_using_directory(&caller, &worktrees, directory, |_| Ok(())).unwrap();
            let current = capture(&caller, &worktrees).unwrap();
            assert!(
                original == current,
                "complete evidence must match the original capture on unchanged inputs"
            );
            assert!(current.directories.len() > worktrees.len());
            assert!(current.files.len() > worktrees.len());
            return;
        }
        if case == "query_target_listing_matches_real_git" {
            let local_path = caller.git_common_dir.join("config");
            let original = fs::read(&local_path).unwrap();
            let global_path = owned.join("home/.gitconfig");
            for (global, local, eligible) in [
                ("", "", true),
                ("", "[devmap]\ndevelopmentTarget =\n", true),
                ("", "[devmap]\ndevelopmentTarget = main\n", true),
                (
                    "",
                    "[devmap]\ndevelopmentTarget = first\ndevelopmentTarget = last\n",
                    true,
                ),
                ("[devmap]\ndevelopmentTarget = global\n", "", true),
                (
                    "[devmap]\ndevelopmentTarget = global\n",
                    "[DevMap]\nDevelopmentTarget = local\n",
                    true,
                ),
                (
                    "",
                    "[devmap]\ndevelopmentTarget = \"  refs/heads/主题  \"\n",
                    true,
                ),
                (
                    "",
                    "[devmap]\ndevelopmentTarget = \"quote\\\"slash\\\\\"\n",
                    true,
                ),
                ("", "[devmap]\ndevelopmentTarget\n", false),
                ("", "[devmap]\ndevelopmentTarget = \"line\\nnext\"\n", false),
                ("", "[include]\npath = missing\n", false),
            ] {
                fs::write(&global_path, global).unwrap();
                let mut bytes = original.clone();
                bytes.extend_from_slice(local.as_bytes());
                fs::write(&local_path, bytes).unwrap();
                let expected =
                    super::super::GitRelationshipResolver::development_configuration(&caller)
                        .unwrap();
                let starts = crate::git_process::test_spawn_count();
                let actual = QueryConfiguration::acquire(&caller).unwrap();
                if eligible {
                    let proof = actual.expect("plain configuration must be eligible");
                    assert_eq!(
                        proof.value(),
                        expected.as_deref(),
                        "global={global:?}, local={local:?}"
                    );
                    assert_eq!(
                        crate::git_process::test_spawn_count() - starts,
                        3,
                        "reuse witnessed listing instead of a fourth Git invocation"
                    );
                    assert!(proof.recheck().unwrap());
                } else {
                    assert!(actual.is_none(), "unsupported value must use original path");
                }
            }
            return;
        }
        match case {
            "include_declines" => {
                git(
                    &main,
                    &["config", "include.path", "missing-included-config"],
                );
            }
            "conditional_include_declines" => {
                git(
                    &main,
                    &[
                        "config",
                        "includeIf.onbranch:topic.path",
                        "missing-included-config",
                    ],
                );
            }
            "worktree_configuration_declines" => {
                git(&main, &["config", "extensions.worktreeConfig", "true"]);
                fs::write(
                    caller.git_common_dir.join("config.worktree"),
                    b"[core]\n\tbare = false\n",
                )
                .unwrap();
            }
            "unknown_configuration_declines" => {
                git(&main, &["config", "devmapUnreviewed.semantic", "value"]);
            }
            "partial_layout_declines" => {
                fs::write(
                    caller.git_common_dir.join("shallow"),
                    format!("{}\n", caller.head),
                )
                .unwrap();
            }
            "semantic_environment_declines" => {}
            _ => {
                let proof = acquire(&caller, &worktrees)
                    .unwrap()
                    .expect("plain real same-common roots require a closed proof");
                assert!(
                    proof.recheck().unwrap(),
                    "unchanged inputs must retain proof"
                );
                match case {
                    "plain_linked_roots_are_eligible"
                    | "cat_pager_is_eligible"
                    | "empty_pager_is_eligible" => return,
                    "config_edit_invalidates" => {
                        git(&main, &["config", "user.name", "changed-fixture"]);
                    }
                    "target_move_invalidates" => {
                        git(
                            &main,
                            &[
                                "-c",
                                "user.name=fixture",
                                "-c",
                                "user.email=fixture@example.invalid",
                                "commit",
                                "--allow-empty",
                                "-qm",
                                "next",
                            ],
                        );
                        git(&main, &["branch", "-f", "dev", "HEAD"]);
                    }
                    "tag_creation_invalidates" => {
                        git(&main, &["tag", "new-tag"]);
                    }
                    "missing_config_creation_invalidates" => {
                        let raw = git(&main, &["var", "GIT_CONFIG_GLOBAL"]);
                        let candidates: Vec<_> = raw.lines().map(PathBuf::from).collect();
                        assert!(!candidates.is_empty(), "Git must report config candidates");
                        let candidate = candidates.into_iter().find(|p| {
                            p.is_absolute()
                                && !p.exists()
                                && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))
                                && p.ancestors().skip(1).find(|parent| parent.exists())
                                    .and_then(|parent| parent.canonicalize().ok())
                                    .is_some_and(|parent| parent.starts_with(&canonical_owned))
                        }).expect("Git-discovered missing candidate must belong to owned isolated home");
                        fs::create_dir_all(candidate.parent().unwrap()).unwrap();
                        assert!(
                            candidate
                                .parent()
                                .unwrap()
                                .canonicalize()
                                .unwrap()
                                .starts_with(&canonical_owned)
                        );
                        fs::write(candidate, b"[user]\n\tname = created-after-proof\n").unwrap();
                    }
                    other => panic!("unknown test case {other}"),
                }
                assert!(
                    !proof.recheck().unwrap(),
                    "changed guarded inputs must discard proof"
                );
                return;
            }
        }
        assert!(
            acquire(&caller, &worktrees).unwrap().is_none(),
            "complex context must select ordinary fallback"
        );
    }

    macro_rules! cases {
        ($($case:ident),+ $(,)?) => { $(#[test] fn $case() { isolated(stringify!($case)); })+ };
    }
    cases!(
        capture_directory_sandwich_matches_original_evidence,
        query_target_listing_matches_real_git,
        plain_linked_roots_are_eligible,
        cat_pager_is_eligible,
        empty_pager_is_eligible,
        include_declines,
        conditional_include_declines,
        worktree_configuration_declines,
        semantic_environment_declines,
        unknown_configuration_declines,
        partial_layout_declines,
        config_edit_invalidates,
        target_move_invalidates,
        tag_creation_invalidates,
        missing_config_creation_invalidates,
    );
}
