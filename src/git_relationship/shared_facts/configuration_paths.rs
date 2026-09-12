//! Overlap independent path discovery, before witnessing any configuration file.
use super::{DevMapError, Path, probe};

pub(super) fn capture(root: &Path) -> Result<(Vec<u8>, Vec<u8>), DevMapError> {
    pair(
        |name| probe(root, &["var", name]),
        #[cfg(test)]
        false,
    )
}

fn pair(
    probe: impl Fn(&str) -> Result<Vec<u8>, DevMapError> + Sync,
    #[cfg(test)] refuse_spawn: bool,
) -> Result<(Vec<u8>, Vec<u8>), DevMapError> {
    let budget = crate::git_process::current_budget();
    crate::git_process::with_budget(&budget, || {
        std::thread::scope(|scope| {
            let run = || crate::git_process::with_budget(&budget, || probe("GIT_CONFIG_GLOBAL"));
            #[cfg(test)]
            let worker = if refuse_spawn {
                Err(std::io::Error::other("injected thread spawn refusal"))
            } else {
                std::thread::Builder::new().spawn_scoped(scope, run)
            };
            #[cfg(not(test))]
            let worker = std::thread::Builder::new().spawn_scoped(scope, run);
            let Ok(worker) = worker else {
                // Keep the original short circuit if no worker was started.
                return Ok((probe("GIT_CONFIG_SYSTEM")?, probe("GIT_CONFIG_GLOBAL")?));
            };
            let system = probe("GIT_CONFIG_SYSTEM");
            // Join even when system discovery declined. Scoped unwinding also joins.
            let global = worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            // Either supervisor failure must escape optional-proof fallback. System
            // wins ties; ordinary declines keep the original system-first order.
            match (system, global) {
                (Err(error @ DevMapError::GitProcess(_)), _) => Err(error),
                (_, Err(error @ DevMapError::GitProcess(_))) => Err(error),
                (system, global) => Ok((system?, global?)),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_process::{GitBudget, GitProcessError, current_budget, with_budget};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::Duration;

    #[test]
    fn independent_probes_overlap_and_inherit_the_absolute_deadline() {
        let budget = GitBudget::new(Duration::from_secs(3));
        let (system_tx, system_rx) = mpsc::channel();
        let (global_tx, global_rx) = mpsc::channel();
        let system_rx = Mutex::new(system_rx);
        let global_rx = Mutex::new(global_rx);
        let result = with_budget(&budget, || {
            pair(
                |name| {
                    assert_eq!(current_budget().deadline(), budget.deadline());
                    if name == "GIT_CONFIG_SYSTEM" {
                        system_tx.send(()).unwrap();
                        global_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap();
                    } else {
                        global_tx.send(()).unwrap();
                        system_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap();
                    }
                    Ok(name.as_bytes().to_vec())
                },
                false,
            )
        })
        .unwrap();
        assert_eq!(
            result,
            (b"GIT_CONFIG_SYSTEM".to_vec(), b"GIT_CONFIG_GLOBAL".to_vec())
        );
    }

    #[test]
    fn global_fatal_error_survives_system_decline_and_worker_is_joined() {
        let finished = AtomicBool::new(false);
        let result = pair(
            |name| {
                if name == "GIT_CONFIG_SYSTEM" {
                    return Err(super::super::decline());
                }
                finished.store(true, Ordering::SeqCst);
                Err(GitProcessError::CleanupFailed.into())
            },
            false,
        );
        assert!(finished.load(Ordering::SeqCst));
        assert!(matches!(
            result,
            Err(DevMapError::GitProcess(GitProcessError::CleanupFailed))
        ));
    }

    #[test]
    fn expired_budget_refuses_real_git_in_both_probes() {
        let budget = GitBudget::new(Duration::ZERO);
        let result = with_budget(&budget, || capture(Path::new(".")));
        assert!(matches!(
            result,
            Err(DevMapError::GitProcess(GitProcessError::Deadline))
        ));
    }

    #[test]
    fn system_fatal_error_wins_fatal_ties() {
        let result = pair(
            |name| {
                Err(if name == "GIT_CONFIG_SYSTEM" {
                    GitProcessError::Deadline
                } else {
                    GitProcessError::CleanupFailed
                }
                .into())
            },
            false,
        );
        assert!(matches!(
            result,
            Err(DevMapError::GitProcess(GitProcessError::Deadline))
        ));
    }

    #[test]
    fn spawn_refusal_preserves_serial_order_and_short_circuit() {
        let calls = Mutex::new(Vec::new());
        assert!(
            pair(
                |name| {
                    calls.lock().unwrap().push(name.to_owned());
                    Err(super::super::decline())
                },
                true
            )
            .is_err()
        );
        assert_eq!(*calls.lock().unwrap(), ["GIT_CONFIG_SYSTEM"]);
        calls.lock().unwrap().clear();
        pair(
            |name| {
                calls.lock().unwrap().push(name.to_owned());
                Ok(Vec::new())
            },
            true,
        )
        .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["GIT_CONFIG_SYSTEM", "GIT_CONFIG_GLOBAL"]
        );
    }

    #[test]
    fn caller_unwind_joins_worker_and_worker_panic_propagates() {
        let finished = AtomicBool::new(false);
        let result = std::panic::catch_unwind(|| {
            pair(
                |name| {
                    if name == "GIT_CONFIG_SYSTEM" {
                        panic!("caller failed");
                    }
                    finished.store(true, Ordering::SeqCst);
                    Ok(Vec::new())
                },
                false,
            )
        });
        assert!(result.is_err());
        assert!(finished.load(Ordering::SeqCst));
        let result = std::panic::catch_unwind(|| {
            pair(
                |name| {
                    if name == "GIT_CONFIG_GLOBAL" {
                        panic!("worker failed");
                    }
                    Ok(Vec::new())
                },
                false,
            )
        });
        assert!(result.is_err());
    }
}
