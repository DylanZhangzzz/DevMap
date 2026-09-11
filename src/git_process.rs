//! Supervision for locally constructed read-only Git commands.
use std::{
    cell::RefCell,
    future::Future,
    io,
    process::{Command, Output, Stdio},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt};
#[cfg(test)]
mod tests;
#[cfg(unix)]
mod unix;

#[derive(Debug, thiserror::Error)]
pub enum GitProcessError {
    #[error("Git operation deadline exceeded")]
    Deadline,
    #[error("Git stdout exceeds its resource limit")]
    StdoutLimit,
    #[error("Git stderr exceeds its resource limit")]
    StderrLimit,
    #[error("Git process I/O: {0}")]
    Io(#[from] io::Error),
    #[error("Git process cleanup could not be confirmed; process owner quarantined")]
    CleanupFailed,
}
#[derive(Clone)]
pub(crate) struct GitBudget {
    deadline: Instant,
}
#[cfg(test)]
static TEST_SPAWN_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) fn test_spawn_count() -> usize {
    TEST_SPAWN_COUNT.load(Ordering::Relaxed)
}
#[cfg(test)]
type Profile = std::sync::Arc<Mutex<Vec<(&'static str, Duration)>>>;
#[cfg(test)]
thread_local! {
    static PROFILE: RefCell<Option<Profile>> = const { RefCell::new(None) };
}
#[cfg(test)]
struct ProfileSpan(Option<Profile>, &'static str, Instant);
#[cfg(test)]
impl ProfileSpan {
    fn new(profile: &Option<Profile>, name: &'static str) -> Self {
        Self(profile.clone(), name, Instant::now())
    }
}
#[cfg(test)]
impl Drop for ProfileSpan {
    fn drop(&mut self) {
        if let Some(profile) = &self.0 {
            profile.lock().unwrap().push((self.1, self.2.elapsed()));
        }
    }
}
#[cfg(test)]
fn with_profile<T>(operation: impl FnOnce() -> T) -> (T, Profile) {
    struct Restore(Option<Profile>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROFILE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let profile = Profile::default();
    let old = PROFILE.with(|slot| slot.replace(Some(profile.clone())));
    let _restore = Restore(old);
    (operation(), profile)
}
impl GitBudget {
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn new(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
        }
    }
}
thread_local! {
    static BUDGET: RefCell<Option<GitBudget>> = const { RefCell::new(None) };
}
pub(crate) fn current_budget() -> GitBudget {
    BUDGET
        .with(|b| b.borrow().clone())
        .unwrap_or_else(|| GitBudget::new(Duration::from_secs(25)))
}
pub(crate) fn with_budget<T>(budget: &GitBudget, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<GitBudget>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BUDGET.with(|b| *b.borrow_mut() = self.0.take());
        }
    }
    let previous = BUDGET.with(|b| {
        let mut b = b.borrow_mut();
        let deadline = b
            .as_ref()
            .map_or(budget.deadline, |old| old.deadline.min(budget.deadline));
        b.replace(GitBudget { deadline })
    });
    let _restore = Restore(previous);
    operation()
}
pub(crate) fn with_operation<T>(operation: impl FnOnce() -> T) -> T {
    with_budget(&current_budget(), operation)
}
static HEALTHY: AtomicBool = AtomicBool::new(true);
#[cfg(test)]
static FORCE_CLEANUP_FAILURE: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
pub(crate) fn test_force_cleanup_failure(force: bool) {
    FORCE_CLEANUP_FAILURE.store(force, Ordering::SeqCst);
}
pub(crate) fn healthy() -> bool {
    HEALTHY.load(Ordering::SeqCst)
}
static ADMISSION: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        *ADMISSION.0.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        ADMISSION.1.notify_one();
    }
}
fn admit(deadline: Instant) -> Result<Permit, GitProcessError> {
    let mut count = ADMISSION.0.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if !healthy() {
            return Err(GitProcessError::CleanupFailed);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(GitProcessError::Deadline);
        };
        if *count < 4 {
            *count += 1;
            return Ok(Permit);
        }
        let (next, _) = ADMISSION
            .1
            .wait_timeout(count, remaining.min(Duration::from_millis(50)))
            .unwrap_or_else(|e| e.into_inner());
        count = next;
    }
}
#[derive(Clone)]
struct ProcessLimits {
    command_timeout: Duration,
    cleanup_timeout: Duration,
    stdout_bytes: usize,
    stderr_bytes: usize,
}
impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(5),
            cleanup_timeout: Duration::from_secs(1),
            stdout_bytes: 8 * 1024 * 1024,
            stderr_bytes: 64 * 1024,
        }
    }
}
pub(crate) fn output(command: &mut Command) -> Result<Output, GitProcessError> {
    output_with_limits(command, ProcessLimits::default())
}
fn output_with_limits(
    command: &mut Command,
    limits: ProcessLimits,
) -> Result<Output, GitProcessError> {
    #[cfg(test)]
    let profile = PROFILE.with(|slot| slot.borrow().clone());
    #[cfg(test)]
    let _total_span = ProfileSpan::new(&profile, "output_total");
    #[cfg(test)]
    let admission_span = ProfileSpan::new(&profile, "admission");
    let operation_deadline = current_budget().deadline;
    let _permit = admit(operation_deadline)?;
    #[cfg(test)]
    drop(admission_span);
    let deadline = operation_deadline.min(Instant::now() + limits.command_timeout);
    // All production callers build only program/arguments/environment/current_dir.
    // output() semantics replace stdin/stdout/stderr; no shell or caller wire is involved.
    let mut child_command = tokio::process::Command::new(command.get_program());
    child_command.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                child_command.env(key, value);
            }
            None => {
                child_command.env_remove(key);
            }
        }
    }
    if let Some(path) = command.get_current_dir() {
        child_command.current_dir(path);
    }
    child_command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    crate::runtime::prepare_git_child(&mut child_command);
    #[cfg(unix)]
    unix::prepare(&mut child_command);
    let run = move || {
        // Explicitly destroy the reactor before thread-local runtime context is
        // torn down. A reactor retained in TLS can deadlock on Windows shutdown.
        #[cfg(test)]
        let build_span = ProfileSpan::new(&profile, "reactor_build");
        let reactor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        #[cfg(test)]
        drop(build_span);
        let result = reactor.block_on(supervise(
            child_command,
            limits,
            deadline,
            #[cfg(test)]
            profile.clone(),
        ));
        #[cfg(test)]
        let drop_span = ProfileSpan::new(&profile, "reactor_drop");
        drop(reactor);
        #[cfg(test)]
        drop(drop_span);
        result
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        // The scope joins on every path. No detached pipe reader or runtime survives.
        std::thread::scope(|scope| scope.spawn(run).join().expect("Git supervisor panicked"))
    } else {
        run()
    }
}
async fn read_bounded(
    mut stream: impl AsyncRead + Unpin,
    limit: usize,
    stdout: bool,
) -> Result<Vec<u8>, GitProcessError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = stream
            .read(&mut chunk[..8192.min(limit.saturating_sub(bytes.len()).saturating_add(1))])
            .await?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > limit.saturating_sub(bytes.len()) {
            return Err(if stdout {
                GitProcessError::StdoutLimit
            } else {
                GitProcessError::StderrLimit
            });
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}
#[cfg(windows)]
type Tree = crate::runtime::GitChildTree;
#[cfg(unix)]
type Tree = unix::Tree;
async fn clean(child: &mut tokio::process::Child, tree: &mut Option<Tree>, budget: Duration) {
    // Failed cleanup is an exceptional quarantine: retain handles and the caller
    // executor/owner lock until cleanup can actually be confirmed. Never detach.
    loop {
        let attempt = tokio::time::timeout(budget, async {
            #[cfg(test)]
            if FORCE_CLEANUP_FAILURE.load(Ordering::SeqCst) {
                return Err(io::Error::other(
                    "controlled Git cleanup confirmation failure",
                ));
            }
            if let Some(tree) = tree.as_mut() {
                tree.terminate_and_wait().await?;
            }
            let _ = child.start_kill();
            child.wait().await?;
            Ok::<_, io::Error>(())
        })
        .await;
        if matches!(attempt, Ok(Ok(()))) {
            return;
        }
        HEALTHY.store(false, Ordering::SeqCst);
        ADMISSION.1.notify_all();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn supervise(
    mut command: tokio::process::Command,
    limits: ProcessLimits,
    deadline: Instant,
    #[cfg(test)] profile: Option<Profile>,
) -> Result<Output, GitProcessError> {
    if Instant::now() >= deadline {
        return Err(GitProcessError::Deadline);
    }
    #[cfg(test)]
    let spawn_span = ProfileSpan::new(&profile, "spawn_suspended");
    let mut child = command.spawn()?;
    #[cfg(test)]
    drop(spawn_span);
    #[cfg(test)]
    TEST_SPAWN_COUNT.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    let attach_span = ProfileSpan::new(&profile, "tree_attach_resume");
    let mut tree = match Tree::attach(&child) {
        Ok(tree) => Some(tree),
        Err(error) => {
            clean(&mut child, &mut None, limits.cleanup_timeout).await;
            return Err(GitProcessError::Io(error));
        }
    };
    #[cfg(test)]
    drop(attach_span);
    let mut stdout = Box::pin(read_bounded(
        child.stdout.take().expect("piped stdout"),
        limits.stdout_bytes,
        true,
    ));
    let mut stderr = Box::pin(read_bounded(
        child.stderr.take().expect("piped stderr"),
        limits.stderr_bytes,
        false,
    ));
    let mut out = None;
    let mut err = None;
    #[cfg(test)]
    let wait_span = ProfileSpan::new(&profile, "root_wait_with_concurrent_pipe_reads");
    let result = tokio::time::timeout_at(deadline.into(), async {
        {
            #[cfg(windows)]
            let mut exited = Box::pin(async { child.wait().await.map(|_| ()) });
            #[cfg(unix)]
            let mut exited = Box::pin(unix::observe_exit(child.id().expect("owned child")));
            std::future::poll_fn(|cx| {
                if out.is_none()
                    && let std::task::Poll::Ready(value) = stdout.as_mut().poll(cx)
                {
                    match value {
                        Ok(value) => out = Some(value),
                        Err(error) => return std::task::Poll::Ready(Err(error)),
                    }
                }
                if err.is_none()
                    && let std::task::Poll::Ready(value) = stderr.as_mut().poll(cx)
                {
                    match value {
                        Ok(value) => err = Some(value),
                        Err(error) => return std::task::Poll::Ready(Err(error)),
                    }
                }
                exited
                    .as_mut()
                    .poll(cx)
                    .map(|r| r.map_err(GitProcessError::Io))
            })
            .await?;
        }
        Ok::<_, GitProcessError>(())
    })
    .await;
    #[cfg(test)]
    drop(wait_span);
    #[cfg(test)]
    let clean_span = ProfileSpan::new(&profile, "cleanup_confirmed");
    clean(&mut child, &mut tree, limits.cleanup_timeout).await;
    #[cfg(test)]
    drop(clean_span);
    if !healthy() {
        return Err(GitProcessError::CleanupFailed);
    }
    result.map_err(|_| GitProcessError::Deadline)??;
    // Root exit can precede EOF because descendants inherited pipes. They have
    // now been cancelled; drain buffered bytes under the original deadline.
    #[cfg(test)]
    let drain_span = ProfileSpan::new(&profile, "pipe_drain_after_cleanup");
    tokio::time::timeout_at(deadline.into(), async {
        std::future::poll_fn(|cx| {
            if out.is_none()
                && let std::task::Poll::Ready(value) = stdout.as_mut().poll(cx)
            {
                match value {
                    Ok(value) => out = Some(value),
                    Err(error) => return std::task::Poll::Ready(Err(error)),
                }
            }
            if err.is_none()
                && let std::task::Poll::Ready(value) = stderr.as_mut().poll(cx)
            {
                match value {
                    Ok(value) => err = Some(value),
                    Err(error) => return std::task::Poll::Ready(Err(error)),
                }
            }
            if out.is_some() && err.is_some() {
                std::task::Poll::Ready(Ok(()))
            } else {
                std::task::Poll::Pending
            }
        })
        .await
    })
    .await
    .map_err(|_| GitProcessError::Deadline)??;
    #[cfg(test)]
    drop(drain_span);
    #[cfg(test)]
    let _final_wait_span = ProfileSpan::new(&profile, "final_cached_child_wait");
    let status = child.wait().await?;
    Ok(Output {
        status,
        stdout: out.unwrap(),
        stderr: err.unwrap(),
    })
}
