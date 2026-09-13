//! Scoped independent probes with ordered ordinary errors and fatal supervision.
use super::DevMapError;

pub(super) fn pair<A, B: Send>(
    primary: impl FnOnce() -> Result<A, DevMapError>,
    secondary: impl Fn() -> Result<B, DevMapError> + Sync,
    #[cfg(test)] refuse_spawn: bool,
) -> Result<(A, B), DevMapError> {
    let budget = crate::git_process::current_budget();
    crate::git_process::with_budget(&budget, || {
        std::thread::scope(|scope| {
            let run = || crate::git_process::with_budget(&budget, &secondary);
            #[cfg(test)]
            let worker = if refuse_spawn {
                Err(std::io::Error::other("injected spawn refusal"))
            } else {
                std::thread::Builder::new().spawn_scoped(scope, run)
            };
            #[cfg(not(test))]
            let worker = std::thread::Builder::new().spawn_scoped(scope, run);
            let Ok(worker) = worker else {
                return Ok((primary()?, secondary()?));
            };
            let first = primary();
            let second = worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            match (first, second) {
                (Err(error @ DevMapError::GitProcess(_)), _) => Err(error),
                (_, Err(error @ DevMapError::GitProcess(_))) => Err(error),
                (first, second) => Ok((first?, second?)),
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
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::time::Duration;

    #[test]
    fn nested_three_probes_overlap_and_share_one_absolute_budget() {
        let budget = GitBudget::new(Duration::from_secs(3));
        let (tx, rx) = mpsc::channel();
        let rx = Mutex::new(rx);
        let result = with_budget(&budget, || {
            pair(
                || {
                    pair(
                        || {
                            assert_eq!(current_budget().deadline(), budget.deadline());
                            for _ in 0..2 {
                                rx.lock()
                                    .unwrap()
                                    .recv_timeout(Duration::from_secs(2))
                                    .unwrap();
                            }
                            Ok(1)
                        },
                        || {
                            assert_eq!(current_budget().deadline(), budget.deadline());
                            tx.send(()).unwrap();
                            Ok(2)
                        },
                        false,
                    )
                },
                || {
                    assert_eq!(current_budget().deadline(), budget.deadline());
                    tx.send(()).unwrap();
                    Ok(3)
                },
                false,
            )
        })
        .unwrap();
        assert_eq!(result, ((1, 2), 3));
    }

    #[test]
    fn started_fatal_errors_escape_declines_in_original_fatal_tie_order() {
        for stage in 0..3 {
            let called = AtomicUsize::new(0);
            let run = |index| -> Result<(), DevMapError> {
                called.fetch_add(1, Ordering::SeqCst);
                if index == stage {
                    Err(GitProcessError::Deadline.into())
                } else {
                    Err(super::super::decline())
                }
            };
            let result = pair(|| pair(|| run(0), || run(1), false), || run(2), false);
            assert_eq!(called.load(Ordering::SeqCst), 3);
            assert!(matches!(
                result,
                Err(DevMapError::GitProcess(GitProcessError::Deadline))
            ));
        }
        let result = pair::<(), ()>(
            || Err(GitProcessError::Deadline.into()),
            || Err(GitProcessError::CleanupFailed.into()),
            false,
        );
        assert!(matches!(
            result,
            Err(DevMapError::GitProcess(GitProcessError::Deadline))
        ));
        let result = pair::<(), ()>(
            || Err(DevMapError::Store("first".into())),
            || Err(DevMapError::Store("second".into())),
            false,
        );
        assert!(matches!(result, Err(DevMapError::Store(message)) if message == "first"));
    }

    #[test]
    fn spawn_refusal_keeps_primary_validation_short_circuit() {
        let called = AtomicUsize::new(0);
        let result = pair::<(), ()>(
            || Err(super::super::decline()),
            || {
                called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            true,
        );
        assert!(result.is_err());
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert_eq!(pair(|| Ok(1), || Ok(2), true).unwrap(), (1, 2));
    }

    #[test]
    fn either_panic_joins_other_work_before_unwinding_returns() {
        for primary_panics in [false, true] {
            let finished = AtomicUsize::new(0);
            let result = std::panic::catch_unwind(|| {
                pair::<(), ()>(
                    || {
                        assert!(!primary_panics, "injected primary panic");
                        finished.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                    || {
                        assert!(primary_panics, "injected worker panic");
                        finished.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                    false,
                )
            });
            assert!(result.is_err());
            assert_eq!(finished.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn expired_budget_refuses_real_git_without_renewal() {
        let budget = GitBudget::new(Duration::ZERO);
        let result = with_budget(&budget, || {
            pair(
                || super::super::probe(std::path::Path::new("."), &["--version"]),
                || super::super::probe(std::path::Path::new("."), &["--version"]),
                false,
            )
        });
        assert!(matches!(
            result,
            Err(DevMapError::GitProcess(GitProcessError::Deadline))
        ));
    }
}
