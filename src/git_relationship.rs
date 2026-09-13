use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};

use crate::error::DevMapError;
use crate::git::SourceWorkspace;
use crate::worktrees::WorktreeDescriptor;

mod shared_facts;
pub(crate) use shared_facts::QueryConfiguration;

const MAX_FORK_TAGS: usize = 32;
const MAX_FORK_TAG_BYTES: usize = 256;
const MAX_FORK_SUBJECT_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSource {
    Config,
    LocalDev,
    LocalDevelop,
    RemoteDefault,
    LocalMain,
    LocalMaster,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentTarget {
    pub name: String,
    pub ref_name: String,
    pub source: TargetSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationBranch {
    pub name: String,
    pub ref_name: String,
    pub head: String,
    pub parent: Option<String>,
    pub source: TargetSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkPoint {
    pub target_branch: String,
    pub commit: String,
    pub tags: Vec<String>,
    pub subject: Option<String>,
    pub authored_at: Option<String>,
    pub distance_to_target: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRelationship {
    pub base_target: Option<String>,
    pub merge_target: Option<String>,
    pub merged: Option<bool>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub dirty: bool,
    pub changed_file_count: u32,
    pub status_observed: bool,
    pub fork_point: Option<ForkPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRelationshipWarning {
    pub code: &'static str,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRelationshipReport {
    pub target: Option<DevelopmentTarget>,
    pub integration_branches: Vec<IntegrationBranch>,
    pub by_worktree_id: BTreeMap<String, GitRelationship>,
    pub warnings: Vec<GitRelationshipWarning>,
}

pub struct GitRelationshipResolver;

#[cfg(test)]
#[derive(Clone, Copy)]
enum SharedFactsTestMode {
    Original,
    Shared,
}

#[cfg(test)]
enum SharedFactsTestFault {
    OrdinaryRepresentativeFailure,
    TagBeforeRecheck,
    RepresentativeDeadline,
}

#[cfg(test)]
fn resolve_with_shared_facts_fault_test(
    workspace: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
    fault: SharedFactsTestFault,
) -> Result<(GitRelationshipReport, SharedFactsTestStats), DevMapError> {
    GitRelationshipResolver::resolve_acquiring(
        workspace,
        worktrees,
        || GitRelationshipResolver::development_configuration(workspace),
        true,
        Some(fault),
    )
}

#[derive(Default)]
#[cfg_attr(not(test), allow(dead_code))]
struct SharedFactsTestStats {
    shared_groups: usize,
    shared_rows: usize,
    original_rows: usize,
}

#[cfg(test)]
fn resolve_with_shared_facts_test(
    workspace: &SourceWorkspace,
    worktrees: &[WorktreeDescriptor],
    mode: SharedFactsTestMode,
) -> Result<(GitRelationshipReport, SharedFactsTestStats), DevMapError> {
    GitRelationshipResolver::resolve_acquiring(
        workspace,
        worktrees,
        || GitRelationshipResolver::development_configuration(workspace),
        matches!(mode, SharedFactsTestMode::Shared),
        None,
    )
}

impl GitRelationshipResolver {
    pub fn resolve(
        workspace: &SourceWorkspace,
        worktrees: &[WorktreeDescriptor],
    ) -> Result<GitRelationshipReport, DevMapError> {
        Self::resolve_acquiring(
            workspace,
            worktrees,
            || Self::development_configuration(workspace),
            true,
            #[cfg(test)]
            None,
        )
        .map(|(report, _)| report)
    }

    /// Read the effective configuration in the requesting worktree, including
    /// extensions.worktreeConfig. Keep its original spelling and invalid values.
    pub(crate) fn development_configuration(
        workspace: &SourceWorkspace,
    ) -> Result<Option<String>, DevMapError> {
        optional_text(
            &workspace.root,
            ["config", "--get", "devmap.developmentTarget"],
        )
    }

    pub(crate) fn resolve_with_configuration(
        workspace: &SourceWorkspace,
        worktrees: &[WorktreeDescriptor],
        configured: Option<&str>,
    ) -> Result<GitRelationshipReport, DevMapError> {
        Self::resolve_acquiring(
            workspace,
            worktrees,
            || Ok(configured.map(str::to_owned)),
            true,
            #[cfg(test)]
            None,
        )
        .map(|(report, _)| report)
    }

    fn resolve_acquiring<F>(
        workspace: &SourceWorkspace,
        worktrees: &[WorktreeDescriptor],
        configuration: F,
        share: bool,
        #[cfg(test)] fault: Option<SharedFactsTestFault>,
    ) -> Result<(GitRelationshipReport, SharedFactsTestStats), DevMapError>
    where
        F: FnOnce() -> Result<Option<String>, DevMapError> + Send,
    {
        crate::git_process::with_operation(|| {
            let mut unique =
                BTreeMap::<(std::path::PathBuf, String), Vec<&WorktreeDescriptor>>::new();
            for worktree in worktrees {
                unique
                    .entry((worktree.root.clone(), worktree.head.clone()))
                    .or_default()
                    .push(worktree);
            }
            let unique = unique.into_values().collect::<Vec<_>>();
            let worker_count = std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .saturating_mul(2)
                .min(unique.len().max(1));
            let chunk_size = unique.len().div_ceil(worker_count).max(1);
            let budget = crate::git_process::current_budget();
            #[cfg(test)]
            let mut profile = crate::dock::collection_profile::Clock::relationships();
            // These reads have independent inputs. Keep the original command
            // helpers and capture status only for this operation, never a cache.
            let (configured, root_target, mut development_probes, status_chunks) =
                std::thread::scope(|scope| {
                    let configured =
                        scope.spawn(|| crate::git_process::with_budget(&budget, configuration));
                    let root = scope.spawn(|| {
                        crate::git_process::with_budget(&budget, || select_root_target(workspace))
                    });
                    let development = ["refs/heads/dev", "refs/heads/develop"].map(|reference| {
                        let budget = &budget;
                        (
                            reference,
                            scope.spawn(move || {
                                crate::git_process::with_budget(budget, || {
                                    ref_exists(&workspace.root, reference)
                                })
                            }),
                        )
                    });
                    let statuses = unique
                        .chunks(chunk_size)
                        .map(|chunk| {
                            let budget = budget.clone();
                            scope.spawn(move || {
                                crate::git_process::with_budget(&budget, || {
                                    chunk
                                        .iter()
                                        .map(|matches| {
                                            (matches.clone(), dirty_state(&matches[0].root))
                                        })
                                        .collect::<Vec<_>>()
                                })
                            })
                        })
                        .collect::<Vec<_>>();
                    // Do not return early: every owned worker must finish, including
                    // status probes whose result a prior target error makes unused.
                    (
                        configured
                            .join()
                            .expect("Git configuration worker panicked"),
                        root.join().expect("Git root target worker panicked"),
                        development
                            .into_iter()
                            .map(|(reference, worker)| {
                                (
                                    reference,
                                    worker
                                        .join()
                                        .expect("Git development probe worker panicked"),
                                )
                            })
                            .collect::<BTreeMap<_, _>>(),
                        statuses
                            .into_iter()
                            .map(|worker| worker.join().expect("Git status worker panicked"))
                            .collect::<Vec<_>>(),
                    )
                });
            // Consume in the original gate order. A speculative status failure
            #[cfg(test)]
            profile.mark("parallel_initial_probes");
            // cannot replace the configuration/root/development target error.
            let configured = configured?;
            let root_target = root_target?;
            let mut warnings = Vec::new();
            let development_target = select_development_target(
                workspace,
                configured.as_deref(),
                root_target.as_ref(),
                &mut warnings,
                &mut development_probes,
            )?;
            let target = development_target.clone().or_else(|| root_target.clone());
            let integration_branches =
                integration_branches(workspace, root_target.as_ref(), development_target.as_ref())?;
            // Unselected ordinary ref errors are irrelevant to the original
            // gates. Supervision failures must never become successful absence.
            for result in development_probes.into_values() {
                if let Err(error @ DevMapError::GitProcess(_)) = result {
                    return Err(error);
                }
            }
            let candidates = status_chunks
                .iter()
                .flatten()
                .filter(|(_, status)| status.is_ok())
                .flat_map(|(matches, _)| matches.iter().copied())
                .collect::<Vec<_>>();
            #[cfg(test)]
            profile.mark("target_selection");
            let (shared, shared_groups) = if share {
                shared_facts::compute(
                    workspace,
                    worktrees,
                    &candidates,
                    root_target.as_ref(),
                    development_target.as_ref(),
                    &integration_branches,
                    #[cfg(test)]
                    fault,
                )?
            } else {
                (BTreeMap::new(), 0)
            };
            let shared_rows = std::sync::atomic::AtomicUsize::new(0);
            #[cfg(test)]
            profile.mark("shared_facts");
            let original_rows = std::sync::atomic::AtomicUsize::new(0);
            let mut by_worktree_id = BTreeMap::new();
            let resolved = std::thread::scope(|scope| {
                status_chunks
                    .into_iter()
                    .map(|chunk| {
                        let budget = budget.clone();
                        let shared = &shared;
                        let shared_rows = &shared_rows;
                        let original_rows = &original_rows;
                        let target_root = &root_target;
                        let target_development = &development_target;
                        scope.spawn(move || {
                            crate::git_process::with_budget(&budget, || {
                                chunk
                                    .into_iter()
                                    .map(|(matches, status)| {
                                        (
                                            matches.clone(),
                                            match status {
                                                Ok((dirty, changed_file_count)) => {
                                                    if let Some(facts) =
                                                        shared.get(&matches[0].worktree_id)
                                                    {
                                                        shared_rows.fetch_add(
                                                            matches.len(),
                                                            std::sync::atomic::Ordering::Relaxed,
                                                        );
                                                        let mut facts = facts.clone();
                                                        facts.dirty = dirty;
                                                        facts.changed_file_count =
                                                            changed_file_count;
                                                        Ok((facts, None))
                                                    } else {
                                                        original_rows.fetch_add(
                                                            matches.len(),
                                                            std::sync::atomic::Ordering::Relaxed,
                                                        );
                                                        relationship_for(
                                                            matches[0],
                                                            target_for_worktree(
                                                                matches[0],
                                                                target_root.as_ref(),
                                                                target_development.as_ref(),
                                                            ),
                                                            dirty,
                                                            changed_file_count,
                                                        )
                                                        .map_err(|error| {
                                                            (
                                                                error,
                                                                Some((dirty, changed_file_count)),
                                                            )
                                                        })
                                                    }
                                                }
                                                Err(error) => Err((error, None)),
                                            },
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .flat_map(|worker| worker.join().expect("Git relationship worker panicked"))
                    .collect::<Vec<_>>()
            });

            #[cfg(test)]
            profile.mark("row_resolution");
            let stats = SharedFactsTestStats {
                shared_groups,
                shared_rows: shared_rows.into_inner(),
                original_rows: original_rows.into_inner(),
            };
            for (matches, result) in resolved {
                let result = match result {
                    Err((error @ DevMapError::GitProcess(_), _)) => return Err(error),
                    other => other,
                };
                for worktree in matches {
                    let relationship = match &result {
                        Ok((relationship, warning_code)) => {
                            if let Some(code) = warning_code {
                                warnings.push(GitRelationshipWarning {
                                    code,
                                    worktree_id: Some(worktree.worktree_id.clone()),
                                });
                            }
                            relationship.clone()
                        }
                        Err((_, observed_status)) => {
                            warnings.push(GitRelationshipWarning {
                                code: "git_relationship_unavailable",
                                worktree_id: Some(worktree.worktree_id.clone()),
                            });
                            let target = target_for_worktree(
                                worktree,
                                root_target.as_ref(),
                                development_target.as_ref(),
                            );
                            observed_status.map_or_else(
                                || unknown_relationship(target),
                                |(dirty, changed_file_count)| {
                                    unknown_relationship_with_observed_status(
                                        target,
                                        dirty,
                                        changed_file_count,
                                    )
                                },
                            )
                        }
                    };
                    by_worktree_id.insert(worktree.worktree_id.clone(), relationship);
                }
            }

            Ok((
                GitRelationshipReport {
                    target,
                    integration_branches,
                    by_worktree_id,
                    warnings,
                },
                stats,
            ))
        })
    }
}

fn select_root_target(
    workspace: &SourceWorkspace,
) -> Result<Option<DevelopmentTarget>, DevMapError> {
    if let Some(ref_name) = optional_text(
        &workspace.root,
        ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )? && ref_exists(&workspace.root, &ref_name)?
    {
        let name = ref_name
            .strip_prefix("refs/remotes/origin/")
            .unwrap_or(&ref_name)
            .to_owned();
        return Ok(Some(DevelopmentTarget {
            name,
            ref_name,
            source: TargetSource::RemoteDefault,
        }));
    }

    for (name, source) in [
        ("main", TargetSource::LocalMain),
        ("master", TargetSource::LocalMaster),
    ] {
        let ref_name = format!("refs/heads/{name}");
        if ref_exists(&workspace.root, &ref_name)? {
            return Ok(Some(DevelopmentTarget {
                name: name.into(),
                ref_name,
                source,
            }));
        }
    }

    Ok(None)
}

fn select_development_target(
    workspace: &SourceWorkspace,
    configured: Option<&str>,
    root: Option<&DevelopmentTarget>,
    warnings: &mut Vec<GitRelationshipWarning>,
    prefetched: &mut BTreeMap<&'static str, Result<bool, DevMapError>>,
) -> Result<Option<DevelopmentTarget>, DevMapError> {
    if let Some(configured) = configured {
        let ref_name = configured_ref(configured);
        let exists = match ref_name.as_deref() {
            Some(candidate) if root.is_none_or(|root| root.ref_name != candidate) => {
                development_ref_exists(&workspace.root, candidate, prefetched)?
            }
            _ => false,
        };
        if exists {
            return Ok(Some(DevelopmentTarget {
                name: configured.to_owned(),
                ref_name: ref_name.expect("validated configured ref"),
                source: TargetSource::Config,
            }));
        }
        warnings.push(GitRelationshipWarning {
            code: "configured_development_target_unavailable",
            worktree_id: None,
        });
    }

    for (name, source) in [
        ("dev", TargetSource::LocalDev),
        ("develop", TargetSource::LocalDevelop),
    ] {
        let ref_name = format!("refs/heads/{name}");
        if root.is_none_or(|root| root.ref_name != ref_name)
            && development_ref_exists(&workspace.root, &ref_name, prefetched)?
        {
            return Ok(Some(DevelopmentTarget {
                name: name.into(),
                ref_name,
                source,
            }));
        }
    }

    Ok(None)
}

fn development_ref_exists(
    root: &Path,
    reference: &str,
    prefetched: &mut BTreeMap<&'static str, Result<bool, DevMapError>>,
) -> Result<bool, DevMapError> {
    match prefetched.remove_entry(reference) {
        Some((key, result)) => {
            let exists = result?;
            // A configured fixed candidate may be revisited by the default
            // precedence loop; reuse its exact observation, including absence.
            prefetched.insert(key, Ok(exists));
            Ok(exists)
        }
        None => ref_exists(root, reference),
    }
}

fn integration_branches(
    workspace: &SourceWorkspace,
    root: Option<&DevelopmentTarget>,
    development: Option<&DevelopmentTarget>,
) -> Result<Vec<IntegrationBranch>, DevMapError> {
    let mut branches = Vec::new();
    if let Some(root) = root {
        branches.push(integration_branch(workspace, root, None)?);
    }
    if let Some(development) = development {
        branches.push(integration_branch(
            workspace,
            development,
            root.map(|target| target.name.clone()),
        )?);
    }
    Ok(branches)
}

fn integration_branch(
    workspace: &SourceWorkspace,
    target: &DevelopmentTarget,
    parent: Option<String>,
) -> Result<IntegrationBranch, DevMapError> {
    let revision = format!("{}^{{commit}}", target.ref_name);
    Ok(IntegrationBranch {
        name: target.name.clone(),
        ref_name: target.ref_name.clone(),
        head: required_text(&workspace.root, ["rev-parse", "--verify", &revision])?,
        parent,
        source: target.source.clone(),
    })
}

fn target_for_worktree<'a>(
    worktree: &WorktreeDescriptor,
    root: Option<&'a DevelopmentTarget>,
    development: Option<&'a DevelopmentTarget>,
) -> Option<&'a DevelopmentTarget> {
    if worktree
        .branch
        .as_deref()
        .is_some_and(|branch| root.is_some_and(|target| target.name == branch))
    {
        return None;
    }
    if worktree
        .branch
        .as_deref()
        .is_some_and(|branch| development.is_some_and(|target| target.name == branch))
    {
        return root;
    }
    development.or(root)
}

fn configured_ref(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('-')
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return None;
    }
    Some(if value.starts_with("refs/") {
        value.to_owned()
    } else {
        format!("refs/heads/{value}")
    })
}

fn relationship_for(
    worktree: &WorktreeDescriptor,
    target: Option<&DevelopmentTarget>,
    dirty: bool,
    changed_file_count: u32,
) -> Result<(GitRelationship, Option<&'static str>), DevMapError> {
    relationship_for_observed(worktree, target, dirty, changed_file_count, None)
}
fn relationship_for_observed(
    worktree: &WorktreeDescriptor,
    target: Option<&DevelopmentTarget>,
    dirty: bool,
    changed_file_count: u32,
    tag_witness: Option<&mut Vec<u8>>,
) -> Result<(GitRelationship, Option<&'static str>), DevMapError> {
    let Some(target) = target else {
        return Ok((
            GitRelationship {
                base_target: None,
                merge_target: None,
                merged: None,
                ahead: None,
                behind: None,
                dirty,
                changed_file_count,
                status_observed: true,
                fork_point: None,
            },
            None,
        ));
    };

    let Some(merge_base) = optional_text(
        &worktree.root,
        [
            OsString::from("merge-base"),
            OsString::from(&target.ref_name),
            OsString::from(&worktree.head),
        ],
    )?
    else {
        return Ok((
            unknown_relationship_with_dirty(target, dirty, changed_file_count),
            Some("git_merge_base_unavailable"),
        ));
    };
    if !valid_object_id(&merge_base) {
        return Ok((
            unknown_relationship_with_dirty(target, dirty, changed_file_count),
            Some("git_merge_base_unavailable"),
        ));
    }
    let tag_raw = required_output(
        &worktree.root,
        [
            OsString::from("tag"),
            OsString::from("--points-at"),
            OsString::from(&merge_base),
        ],
    )?;
    if let Some(witness) = tag_witness {
        *witness = tag_raw.stdout.clone();
    }
    let tag_output = output_text(&tag_raw, "git tag --points-at")?;
    let mut tags = tag_output
        .lines()
        .filter(|tag| !tag.is_empty())
        .map(|tag| bounded_utf8(tag, MAX_FORK_TAG_BYTES))
        .take(MAX_FORK_TAGS)
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    let metadata = required_text(
        &worktree.root,
        [
            OsString::from("show"),
            OsString::from("-s"),
            OsString::from("--format=%s%x00%aI"),
            OsString::from(&merge_base),
        ],
    )?;
    let (subject, authored_at) = metadata
        .split_once('\0')
        .ok_or_else(|| malformed_git("git show -s"))?;
    let counts = required_text(
        &worktree.root,
        [
            OsString::from("rev-list"),
            OsString::from("--left-right"),
            OsString::from("--count"),
            OsString::from(format!("{}...{}", target.ref_name, worktree.head)),
        ],
    )?;
    let mut fields = counts.split_whitespace();
    let behind = parse_count(fields.next())?;
    let ahead = parse_count(fields.next())?;
    if fields.next().is_some() {
        return Err(malformed_git("git rev-list --left-right --count"));
    }
    // Only an ancestor head makes the two ranges identical. In particular,
    // divergent criss-cross histories may have more than one merge base.
    // Recheck ahead as the target may have moved since merge-base was read.
    let distance_to_target = if merge_base == worktree.head && ahead == 0 {
        behind
    } else {
        parse_count(Some(&required_text(
            &worktree.root,
            [
                OsString::from("rev-list"),
                OsString::from("--count"),
                OsString::from(format!("{}..{}", merge_base, target.ref_name)),
            ],
        )?))?
    };

    Ok((
        GitRelationship {
            base_target: Some(target.name.clone()),
            merge_target: Some(target.name.clone()),
            merged: Some(ahead == 0),
            ahead: Some(ahead),
            behind: Some(behind),
            dirty,
            changed_file_count,
            status_observed: true,
            fork_point: Some(ForkPoint {
                target_branch: target.name.clone(),
                commit: merge_base,
                tags,
                subject: (!subject.is_empty())
                    .then(|| bounded_utf8(subject, MAX_FORK_SUBJECT_BYTES)),
                authored_at: (!authored_at.is_empty()).then(|| authored_at.to_owned()),
                distance_to_target: Some(distance_to_target),
            }),
        },
        None,
    ))
}

fn unknown_relationship(target: Option<&DevelopmentTarget>) -> GitRelationship {
    GitRelationship {
        base_target: target.map(|value| value.name.clone()),
        merge_target: target.map(|value| value.name.clone()),
        merged: None,
        ahead: None,
        behind: None,
        dirty: false,
        changed_file_count: 0,
        status_observed: false,
        fork_point: None,
    }
}

fn unknown_relationship_with_dirty(
    target: &DevelopmentTarget,
    dirty: bool,
    changed_file_count: u32,
) -> GitRelationship {
    GitRelationship {
        base_target: Some(target.name.clone()),
        merge_target: Some(target.name.clone()),
        merged: None,
        ahead: None,
        behind: None,
        dirty,
        changed_file_count,
        status_observed: true,
        fork_point: None,
    }
}

fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn dirty_state(root: &Path) -> Result<(bool, u32), DevMapError> {
    let output = required_output(
        root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )?;
    let mut fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut count = 0_u32;
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(malformed_git("git status --porcelain=v1 -z"));
        }
        count = count.saturating_add(1);
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            fields
                .next()
                .ok_or_else(|| malformed_git("git status --porcelain=v1 -z"))?;
        }
    }
    Ok((count > 0, count))
}

fn ref_exists(root: &Path, ref_name: &str) -> Result<bool, DevMapError> {
    let output = git_output(root, ["show-ref", "--verify", "--quiet", ref_name])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(command_failure("git show-ref --verify --quiet", &output)),
    }
}

fn optional_text<I, S>(root: &Path, args: I) -> Result<Option<String>, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Ok(None);
    }
    output_text(&output, &display_command(&arguments)).map(Some)
}

fn required_text<I, S>(root: &Path, args: I) -> Result<String, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Err(command_failure(&display_command(&arguments), &output));
    }
    output_text(&output, &display_command(&arguments))
}

fn required_output<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &arguments)?;
    if !output.status.success() {
        return Err(command_failure(&display_command(&arguments), &output));
    }
    Ok(output)
}

fn git_output<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(crate::git_process::output(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1"),
    )?)
}

fn unknown_relationship_with_observed_status(
    target: Option<&DevelopmentTarget>,
    dirty: bool,
    changed_file_count: u32,
) -> GitRelationship {
    GitRelationship {
        base_target: target.map(|value| value.name.clone()),
        merge_target: target.map(|value| value.name.clone()),
        merged: None,
        ahead: None,
        behind: None,
        dirty,
        changed_file_count,
        status_observed: true,
        fork_point: None,
    }
}

fn output_text(output: &Output, command: &str) -> Result<String, DevMapError> {
    String::from_utf8(output.stdout.clone())
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| DevMapError::NonUtf8GitOutput(command.to_owned()))
}

fn parse_count(value: Option<&str>) -> Result<u32, DevMapError> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.min(u32::MAX as u64) as u32)
        .ok_or_else(|| malformed_git("git rev-list --left-right --count"))
}

fn display_command(args: &[OsString]) -> String {
    format!(
        "git {}",
        args.iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn command_failure(command: &str, output: &Output) -> DevMapError {
    DevMapError::GitCommand {
        command: command.to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn malformed_git(command: &str) -> DevMapError {
    DevMapError::GitCommand {
        command: command.to_owned(),
        stderr: "malformed Git output".into(),
    }
}
