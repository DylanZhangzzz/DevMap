//! Bounded read-only origin payload collection. Serial Evidence::capture remains the oracle.
use super::admission::{Admission, SCRATCH_BYTES, read_admitted};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) trait Hooks: Sync {
    fn tasks_ready(&self, _total: usize) {}
    fn spawn_ready(&self, _slot: usize) -> std::io::Result<()> {
        Ok(())
    }
    fn before_origin(&self, _ordinal: usize) -> Result<(), DevMapError> {
        Ok(())
    }
    fn after_origin(&self, _ordinal: usize) {}
    #[cfg(test)]
    fn admitted(&self, _bytes: usize, _nodes: usize, _paths: usize, _directories: usize) {}
    fn completion_ordinal(&self, ordinal: usize) -> usize {
        ordinal
    }
    fn omit_completion(&self, _ordinal: usize) -> bool {
        false
    }
    fn before_merge(&self, _ordinal: usize, _fragment: &mut Evidence) {}
    fn serial_fallback(&self) {}
}
pub(super) struct NoHooks;
impl Hooks for NoHooks {}

#[cfg(test)]
pub(super) struct CaptureProfile {
    pub(super) evidence: Evidence,
    pub(super) discovery_us: u128,
    pub(super) payload_us: u128,
    pub(super) finishing_us: u128,
}

/// Observe the same candidate, retaining its complete evidence for comparison.
/// Hook timestamps are nested in the enclosing capture, not separate queries.
#[cfg(test)]
pub(super) fn profile_candidate(w: &SourceWorkspace) -> Result<CaptureProfile, DevMapError> {
    use std::sync::atomic::AtomicU64;
    use std::time::Instant;
    struct Profile {
        start: Instant,
        ready: AtomicU64,
        last_origin: AtomicU64,
        fallback: AtomicUsize,
    }
    impl Profile {
        fn elapsed(&self) -> u64 {
            u64::try_from(self.start.elapsed().as_micros()).unwrap()
        }
    }
    impl Hooks for Profile {
        fn tasks_ready(&self, total: usize) {
            assert!(total > 0);
            self.ready.store(self.elapsed(), Ordering::SeqCst);
        }
        fn after_origin(&self, _: usize) {
            self.last_origin.fetch_max(self.elapsed(), Ordering::SeqCst);
        }
        fn serial_fallback(&self) {
            self.fallback.fetch_add(1, Ordering::SeqCst);
        }
    }
    let hooks = Profile {
        start: Instant::now(),
        ready: AtomicU64::new(0),
        last_origin: AtomicU64::new(0),
        fallback: AtomicUsize::new(0),
    };
    let evidence = candidate(w, &hooks)?;
    let finished = hooks.elapsed();
    let ready = hooks.ready.load(Ordering::SeqCst);
    let last_origin = hooks.last_origin.load(Ordering::SeqCst);
    if hooks.fallback.load(Ordering::SeqCst) != 0
        || ready == 0
        || last_origin < ready
        || finished < last_origin
    {
        return Err(fail("profiling origin capture boundaries invalid"));
    }
    Ok(CaptureProfile {
        evidence,
        discovery_us: u128::from(ready),
        payload_us: u128::from(last_origin - ready),
        finishing_us: u128::from(finished - last_origin),
    })
}

struct Task {
    admin: PathBuf,
    root: PathBuf,
}
struct Collector<'a, 'h> {
    evidence: Evidence,
    admission: &'a Admission<'h>,
    scratch: [u8; SCRATCH_BYTES],
}
impl<'a, 'h> Collector<'a, 'h> {
    fn new(admission: &'a Admission<'h>) -> Self {
        Self {
            evidence: Evidence::default(),
            admission,
            scratch: [0; SCRATCH_BYTES],
        }
    }
    fn directory(&mut self, path: &Path) -> Result<PathBuf, DevMapError> {
        let path = safe::checked_canonical_directory(path)?;
        self.admission.charge_path(&path)?;
        let identity = safe::checked_directory_identity(&path)?.stable_text();
        self.admission.admit_directory(&path)?;
        self.evidence.directories.insert(path.clone(), identity);
        Ok(path)
    }
    fn file(&mut self, path: &Path) -> Result<(), DevMapError> {
        self.admission.charge_path(path)?;
        self.admission.charge_node()?;
        let value = if let Some(meta) = safe::checked_metadata(path)? {
            if !meta.is_file() || meta.len() > 1024 * 1024 {
                return Err(fail("origin proof file unsupported"));
            }
            let mut file = safe::checked_file(path, false, false)?;
            let identity = safe::file_identity(&file)?.stable_text();
            let bytes = read_admitted(&mut file, self.admission, &mut self.scratch)?;
            if safe::file_identity(&safe::checked_file(path, false, false)?)?.stable_text()
                != identity
            {
                return Err(fail("origin proof file changed"));
            }
            Some((identity, bytes))
        } else {
            None
        };
        self.evidence.files.insert(path.to_owned(), value);
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
            self.admission.charge_node()?;
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
    fn origin(&mut self, common: &Path, task: &Task) -> Result<(), DevMapError> {
        let physical_root = self.directory(&task.root)?;
        let admin = self.directory(&task.admin)?;
        let source = SourceWorkspace {
            root: task.root.clone(),
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
            self.file(&task.root.join(".git"))?;
        }
        self.tree(&admin.join("refs"), 0)?;
        self.evidence.roots.push(FrozenOrigin {
            worktree_id: worktrees::origin_id(&worktrees::repository_id(&source), &admin),
            git_dir: admin,
            workspace_path: physical_root,
            incarnation: journal::worktree_incarnation(&source)?,
        });
        Ok(())
    }
}

fn add_task(
    tasks: &mut Vec<Task>,
    path_bytes: &mut usize,
    admin: PathBuf,
    root: PathBuf,
) -> Result<(), DevMapError> {
    // Additional retained discovery paths have their own bounded budget. They
    // are not Evidence visits and must not alter the serial charge totals.
    if tasks.len() >= 256 {
        return Err(fail("origin proof worktree limit"));
    }
    let bytes = path_bytes
        .checked_add(admin.as_os_str().as_encoded_bytes().len())
        .and_then(|n| n.checked_add(root.as_os_str().as_encoded_bytes().len()))
        .ok_or_else(|| fail("origin proof task path byte limit"))?;
    if bytes > 1024 * 1024 {
        return Err(fail("origin proof task path byte limit"));
    }
    *path_bytes = bytes;
    tasks.push(Task { admin, root });
    Ok(())
}

struct OriginGuard<'a, H: Hooks> {
    hooks: &'a H,
    ordinal: usize,
}
impl<H: Hooks> Drop for OriginGuard<'_, H> {
    fn drop(&mut self) {
        self.hooks.after_origin(self.ordinal);
    }
}

pub(super) fn candidate(w: &SourceWorkspace, hooks: &impl Hooks) -> Result<Evidence, DevMapError> {
    // A nested block ensures all retained fragments and admission state are
    // dropped before the one permitted fresh serial replay.
    let attempt = {
        #[cfg(test)]
        let observer = |b, n, p, d| hooks.admitted(b, n, p, d);
        #[cfg(test)]
        let admission = Admission::with_observer(&observer);
        #[cfg(not(test))]
        let admission = Admission::new();
        collect(w, hooks, &admission)
    }?;
    match attempt {
        Some(evidence) => Ok(evidence),
        None => {
            hooks.serial_fallback();
            Evidence::capture(w)
        }
    }
}

fn discover(
    w: &SourceWorkspace,
    admission: &Admission<'_>,
) -> Result<(PathBuf, Vec<Task>, Evidence), DevMapError> {
    let mut discovery = Collector::new(admission);
    let common = discovery.directory(&w.git_common_dir)?;
    if common.file_name().and_then(|n| n.to_str()) != Some(".git") {
        return Err(fail("origin proof layout unsupported"));
    }
    let main = common
        .parent()
        .ok_or_else(|| fail("origin proof main missing"))?;
    let mut tasks = Vec::new();
    let mut task_path_bytes = 0;
    add_task(
        &mut tasks,
        &mut task_path_bytes,
        common.clone(),
        main.to_owned(),
    )?;
    let objects = discovery.directory(&common.join("objects"))?;
    let _access = fs::read_dir(objects)?;
    let administration = common.join("worktrees");
    if safe::checked_metadata(&administration)?.is_some() {
        discovery.directory(&administration)?;
        for entry in fs::read_dir(&administration)? {
            // Check count before retaining the next path or inspecting payload.
            if tasks.len() >= 256 {
                return Err(fail("origin proof worktree limit"));
            }
            let admin = discovery.directory(&entry?.path())?;
            let backlink = PathBuf::from(pointer_text(&admin.join("gitdir"))?);
            if !backlink.is_absolute()
                || backlink.file_name().and_then(|n| n.to_str()) != Some(".git")
            {
                return Err(fail("origin proof backlink unsupported"));
            }
            let root = backlink
                .parent()
                .ok_or_else(|| fail("origin proof root missing"))?;
            add_task(&mut tasks, &mut task_path_bytes, admin, root.to_owned())?;
        }
    } else {
        discovery.file(&administration)?;
    }
    Ok((common, tasks, discovery.evidence))
}

fn collect(
    w: &SourceWorkspace,
    hooks: &impl Hooks,
    admission: &Admission<'_>,
) -> Result<Option<Evidence>, DevMapError> {
    // Discovery's scratch and directory iterators die before workers start.
    let (common, tasks, mut result) = discover(w, admission)?;
    hooks.tasks_ready(tasks.len());
    let next = AtomicUsize::new(0);
    let (refused, mut completions, worker_error) = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut refused = false;
        for slot in 0..tasks.len().min(4) {
            let tasks = &tasks;
            let next = &next;
            let common = &common;
            let worker = hooks.spawn_ready(slot).and_then(|()| {
                std::thread::Builder::new().spawn_scoped(scope, move || {
                    let mut completed = Vec::new();
                    loop {
                        let ordinal = next.fetch_add(1, Ordering::Relaxed);
                        if ordinal >= tasks.len() {
                            break;
                        }
                        let _guard = OriginGuard { hooks, ordinal };
                        let outcome: Result<Evidence, DevMapError> = (|| {
                            hooks.before_origin(ordinal)?;
                            let mut collector = Collector::new(admission);
                            collector.origin(common, &tasks[ordinal])?;
                            Ok(collector.evidence)
                        })();
                        completed.push((ordinal, outcome));
                    }
                    completed
                })
            });
            match worker {
                Ok(handle) => handles.push(handle),
                Err(_) => {
                    refused = true;
                    break;
                }
            }
        }
        // Refusal never cancels already-owned workers: they drain the finite
        // list and their genuine errors must take precedence over replay.
        let mut completions = Vec::new();
        let mut worker_error = None;
        for handle in handles {
            match handle.join() {
                Ok(mut completed) => completions.append(&mut completed),
                Err(_) => {
                    admission.cancel();
                    worker_error = Some(fail("origin proof payload worker panicked"));
                }
            }
        }
        (refused, completions, worker_error)
    });
    if let Some(error) = worker_error {
        return Err(error);
    }
    // Preserve real payload failures before applying coordinator fault seams or
    // considering spawn replay. Every worker has already been explicitly joined.
    completions.sort_by_key(|(ordinal, _)| *ordinal);
    let mut fragments = Vec::new();
    for (ordinal, outcome) in completions {
        let fragment = outcome?;
        if !hooks.omit_completion(ordinal) {
            fragments.push((hooks.completion_ordinal(ordinal), fragment));
        }
    }
    // With zero owned workers, refusal legitimately has zero completions.
    // Otherwise all started workers drain the list, requiring exact coverage.
    if !(refused && next.load(Ordering::Relaxed) == 0) {
        let mut ordinals = BTreeSet::new();
        for (ordinal, fragment) in &fragments {
            if *ordinal >= tasks.len() || !ordinals.insert(*ordinal) || fragment.roots.len() != 1 {
                return Err(fail("origin proof invalid payload completion"));
            }
        }
        if ordinals.len() != tasks.len() {
            return Err(fail("origin proof missing payload completion"));
        }
    }
    let totals = admission.finish()?;
    if refused {
        return Ok(None);
    }
    fragments.sort_by_key(|(ordinal, _)| *ordinal);
    let mut conflict = false;
    for (ordinal, mut fragment) in fragments {
        hooks.before_merge(ordinal, &mut fragment);
        for (path, identity) in fragment.directories {
            if result.files.contains_key(&path)
                || result
                    .directories
                    .get(&path)
                    .is_some_and(|old| old != &identity)
            {
                conflict = true;
            } else {
                result.directories.insert(path, identity);
            }
        }
        for (path, value) in fragment.files {
            if result.directories.contains_key(&path)
                || result.files.get(&path).is_some_and(|old| old != &value)
            {
                conflict = true;
            } else {
                result.files.insert(path, value);
            }
        }
        result.roots.append(&mut fragment.roots);
    }
    if conflict {
        return Ok(None);
    }
    if result.directories.len() != totals.directories {
        return Err(fail("origin proof directory completion mismatch"));
    }
    result.bytes = totals.bytes;
    result.nodes = totals.nodes;
    result.path_bytes = totals.path_bytes;
    result.roots.sort_by(|a, b| a.git_dir.cmp(&b.git_dir));
    Ok(Some(result))
}
