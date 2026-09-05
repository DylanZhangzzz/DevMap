use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;

use crate::error::DevMapError;
use crate::git::SourceWorkspace;
use crate::worktrees::WorktreeDescriptor;

const MAX_COMMITS: usize = 2_048;
const MAX_REFS: usize = 256;
const ENRICHMENT_CHUNK: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyCommit {
    pub oid: String,
    pub parents: Vec<String>,
    pub authored_at: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyRef {
    pub ref_name: String,
    pub display_name: String,
    pub oid: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyEdge {
    pub id: String,
    pub from_oid: String,
    pub to_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyBoundary {
    pub id: String,
    pub oid: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyGraph {
    pub commits: Vec<TopologyCommit>,
    pub refs: Vec<TopologyRef>,
    pub edges: Vec<TopologyEdge>,
    pub boundaries: Vec<TopologyBoundary>,
    pub complete: bool,
}

pub struct GitTopologyCollector;

impl GitTopologyCollector {
    pub fn scan(
        workspace: &SourceWorkspace,
        worktrees: &[WorktreeDescriptor],
    ) -> Result<TopologyGraph, DevMapError> {
        let RefRows {
            displayed: refs,
            omitted: omitted_refs,
            unsupported_oids: unsupported_ref_oids,
        } = read_refs(&workspace.root)?;
        let mut boundaries = BTreeMap::new();
        for reference in &omitted_refs {
            insert_boundary(&mut boundaries, &reference.oid, "history_limit");
        }
        for oid in &unsupported_ref_oids {
            insert_boundary(&mut boundaries, oid, "missing");
        }

        let tips = refs
            .iter()
            .map(|reference| reference.oid.clone())
            .chain(worktrees.iter().map(|worktree| worktree.head.clone()))
            .filter(|oid| !oid.is_empty() && !oid.bytes().all(|byte| byte == b'0'))
            .collect::<BTreeSet<_>>();
        if tips.is_empty() {
            return Ok(TopologyGraph {
                commits: Vec::new(),
                refs,
                edges: Vec::new(),
                complete: boundaries.is_empty(),
                boundaries: boundaries.into_values().collect(),
            });
        }

        let shallow_oids = read_shallow_oids(&workspace.root)?;
        let rows = read_commit_rows(&workspace.root, &tips)?;
        let history_truncated = rows.len() > MAX_COMMITS;
        let retained_rows = &rows[..rows.len().min(MAX_COMMITS)];
        let retained_oids = retained_rows
            .iter()
            .map(|row| row.oid.clone())
            .collect::<BTreeSet<_>>();

        if history_truncated {
            for tip in tips.iter().filter(|tip| !retained_oids.contains(*tip)) {
                insert_boundary(&mut boundaries, tip, "history_limit");
            }
        }
        for shallow_oid in shallow_oids
            .iter()
            .filter(|oid| retained_oids.contains(*oid))
        {
            insert_boundary(&mut boundaries, shallow_oid, "shallow");
        }

        let mut edges = Vec::new();
        for row in retained_rows {
            for parent in &row.parents {
                edges.push(TopologyEdge {
                    id: format!("edge:{parent}:{}", row.oid),
                    from_oid: parent.clone(),
                    to_oid: row.oid.clone(),
                });
                if !retained_oids.contains(parent) {
                    let reason = if history_truncated {
                        "history_limit"
                    } else {
                        "missing"
                    };
                    insert_boundary(&mut boundaries, parent, reason);
                }
            }
        }

        let enrichment = enrich_commits(&workspace.root, &retained_oids)?;
        let commits = retained_rows
            .iter()
            .map(|row| {
                let details = enrichment.get(&row.oid);
                TopologyCommit {
                    oid: row.oid.clone(),
                    parents: row.parents.clone(),
                    authored_at: details.map(|detail| detail.authored_at.clone()),
                    subject: details.map(|detail| detail.subject.clone()),
                }
            })
            .collect::<Vec<_>>();

        for oid in retained_oids
            .iter()
            .filter(|oid| !enrichment.contains_key(*oid))
        {
            insert_boundary(&mut boundaries, oid, "missing");
        }

        let incomplete = history_truncated
            || !omitted_refs.is_empty()
            || !shallow_oids.is_empty()
            || boundaries
                .values()
                .any(|boundary| boundary.reason == "missing");
        if !incomplete {
            mark_unrelated_components(&commits, &edges, &mut boundaries);
        }

        Ok(TopologyGraph {
            commits,
            refs,
            edges,
            boundaries: boundaries.into_values().collect(),
            complete: !incomplete,
        })
    }
}

#[derive(Debug)]
struct CommitRow {
    oid: String,
    parents: Vec<String>,
}

#[derive(Debug)]
struct CommitDetails {
    authored_at: String,
    subject: String,
}

struct RefRows {
    displayed: Vec<TopologyRef>,
    omitted: Vec<TopologyRef>,
    unsupported_oids: Vec<String>,
}

fn read_refs(root: &Path) -> Result<RefRows, DevMapError> {
    let output = checked_git(
        root,
        [
            "for-each-ref",
            "--count=257",
            "--format=%(refname)%00%(objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    let text = git_stdout(&output, "git for-each-ref")?;
    let mut refs = Vec::new();
    let mut unsupported_oids = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\0').collect::<Vec<_>>();
        if fields.len() != 2 {
            return Err(malformed_git("git for-each-ref"));
        }
        let Some((kind, display_name)) = ref_identity(fields[0]) else {
            continue;
        };
        if fields[1].is_empty() {
            continue;
        }
        let Some(oid) = resolve_ref_commit(root, fields[0])? else {
            unsupported_oids.push(fields[1].to_owned());
            continue;
        };
        refs.push(TopologyRef {
            ref_name: fields[0].to_owned(),
            display_name: display_name.to_owned(),
            oid,
            kind: kind.to_owned(),
        });
    }
    refs.sort_by(|left, right| left.ref_name.cmp(&right.ref_name));
    let omitted = if refs.len() > MAX_REFS {
        refs.split_off(MAX_REFS)
    } else {
        Vec::new()
    };
    Ok(RefRows {
        displayed: refs,
        omitted,
        unsupported_oids,
    })
}

fn resolve_ref_commit(root: &Path, ref_name: &str) -> Result<Option<String>, DevMapError> {
    let revision = format!("{ref_name}^{{commit}}");
    let args = [
        OsString::from("rev-parse"),
        OsString::from("--verify"),
        OsString::from("--quiet"),
        OsString::from("--end-of-options"),
        OsString::from(revision),
    ];
    let output = git_output(root, &args)?;
    if !output.status.success() {
        return Ok(None);
    }
    let oid = git_stdout(&output, "git rev-parse --verify ref^{commit}")?
        .trim()
        .to_owned();
    if oid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(oid))
    }
}

fn ref_identity(ref_name: &str) -> Option<(&'static str, &str)> {
    if let Some(name) = ref_name.strip_prefix("refs/heads/") {
        Some(("branch", name))
    } else if let Some(name) = ref_name.strip_prefix("refs/remotes/") {
        Some(("remote", name))
    } else {
        ref_name
            .strip_prefix("refs/tags/")
            .map(|name| ("tag", name))
    }
}

fn read_commit_rows(root: &Path, tips: &BTreeSet<String>) -> Result<Vec<CommitRow>, DevMapError> {
    let mut args = vec![
        OsString::from("rev-list"),
        OsString::from("--topo-order"),
        OsString::from("--parents"),
        OsString::from(format!("--max-count={}", MAX_COMMITS + 1)),
    ];
    args.extend(tips.iter().map(OsString::from));
    let output = checked_git(root, &args)?;
    let text = git_stdout(&output, "git rev-list --topo-order --parents")?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            let oid = fields
                .next()
                .ok_or_else(|| malformed_git("git rev-list --topo-order --parents"))?;
            Ok(CommitRow {
                oid: oid.to_owned(),
                parents: fields.map(str::to_owned).collect(),
            })
        })
        .collect()
}

fn enrich_commits(
    root: &Path,
    retained_oids: &BTreeSet<String>,
) -> Result<BTreeMap<String, CommitDetails>, DevMapError> {
    let ordered = retained_oids.iter().collect::<Vec<_>>();
    let mut details = BTreeMap::new();
    for chunk in ordered.chunks(ENRICHMENT_CHUNK) {
        let mut args = vec![
            OsString::from("log"),
            OsString::from("-z"),
            OsString::from("--no-walk=unsorted"),
            OsString::from("--format=%H%x00%aI%x00%s"),
        ];
        args.extend(chunk.iter().map(|oid| OsString::from(oid.as_str())));
        let output = checked_git(root, &args)?;
        let text = git_stdout(&output, "git log --no-walk=unsorted")?;
        let mut fields = text.split('\0').collect::<Vec<_>>();
        if fields.last() == Some(&"") {
            fields.pop();
        }
        if fields.len() % 3 != 0 {
            return Err(malformed_git("git log --no-walk=unsorted"));
        }
        for record in fields.chunks_exact(3) {
            details.insert(
                record[0].to_owned(),
                CommitDetails {
                    authored_at: record[1].to_owned(),
                    subject: record[2].to_owned(),
                },
            );
        }
    }
    Ok(details)
}

fn read_shallow_oids(root: &Path) -> Result<BTreeSet<String>, DevMapError> {
    let output = checked_git(root, ["rev-parse", "--is-shallow-repository"])?;
    let state = git_stdout(&output, "git rev-parse --is-shallow-repository")?;
    match state.trim() {
        "false" => Ok(BTreeSet::new()),
        "true" => {
            let output = checked_git(root, ["rev-parse", "--git-path", "shallow"])?;
            let path = git_stdout(&output, "git rev-parse --git-path shallow")?;
            let path = resolve_git_path(root, path.trim());
            let bytes = std::fs::read(path)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| DevMapError::NonUtf8GitOutput("Git shallow boundary file".into()))?;
            Ok(text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect())
        }
        _ => Err(malformed_git("git rev-parse --is-shallow-repository")),
    }
}

fn resolve_git_path(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn insert_boundary(
    boundaries: &mut BTreeMap<(String, String), TopologyBoundary>,
    oid: &str,
    reason: &str,
) {
    let key = (reason.to_owned(), oid.to_owned());
    boundaries.entry(key).or_insert_with(|| TopologyBoundary {
        id: format!("boundary:{reason}:{oid}"),
        oid: oid.to_owned(),
        reason: reason.to_owned(),
    });
}

fn mark_unrelated_components(
    commits: &[TopologyCommit],
    edges: &[TopologyEdge],
    boundaries: &mut BTreeMap<(String, String), TopologyBoundary>,
) {
    let commit_oids = commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    let mut adjacency: BTreeMap<&str, Vec<&str>> =
        commit_oids.iter().map(|oid| (*oid, Vec::new())).collect();
    for edge in edges {
        if commit_oids.contains(edge.from_oid.as_str())
            && commit_oids.contains(edge.to_oid.as_str())
        {
            adjacency
                .get_mut(edge.from_oid.as_str())
                .expect("retained parent has adjacency")
                .push(edge.to_oid.as_str());
            adjacency
                .get_mut(edge.to_oid.as_str())
                .expect("retained child has adjacency")
                .push(edge.from_oid.as_str());
        }
    }

    let mut visited = BTreeSet::new();
    let mut components = Vec::new();
    for start in &commit_oids {
        if visited.contains(start) {
            continue;
        }
        let mut queue = VecDeque::from([*start]);
        let mut component = BTreeSet::new();
        while let Some(oid) = queue.pop_front() {
            if !visited.insert(oid) {
                continue;
            }
            component.insert(oid);
            for neighbor in adjacency.get(oid).into_iter().flatten() {
                queue.push_back(neighbor);
            }
        }
        components.push(component);
    }
    if components.len() < 2 {
        return;
    }

    for component in components {
        for commit in commits.iter().filter(|commit| {
            component.contains(commit.oid.as_str())
                && commit
                    .parents
                    .iter()
                    .all(|parent| !component.contains(parent.as_str()))
        }) {
            insert_boundary(boundaries, &commit.oid, "unrelated");
        }
    }
}

fn checked_git<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = git_output(root, &args)?;
    if !output.status.success() {
        return Err(DevMapError::GitCommand {
            command: display_git_command(&args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output)
}

fn git_output<I, S>(root: &Path, args: I) -> Result<Output, DevMapError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()?)
}

fn git_stdout<'a>(output: &'a Output, command: &str) -> Result<&'a str, DevMapError> {
    std::str::from_utf8(&output.stdout)
        .map_err(|_| DevMapError::NonUtf8GitOutput(command.to_owned()))
}

fn malformed_git(command: &str) -> DevMapError {
    DevMapError::GitCommand {
        command: command.to_owned(),
        stderr: "Git returned malformed topology output".to_owned(),
    }
}

fn display_git_command(args: &[OsString]) -> String {
    format!(
        "git {}",
        args.iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    )
}
