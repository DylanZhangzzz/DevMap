//! Retry admission without regenerating the caller's immutable request.
use super::RuntimeCallError;
use std::time::{Duration, Instant};

/// Busy means the owner has not accepted this request. A transport failure may
/// follow a commit, so permit only one reconnect for the entire operation.
/// This deadline bounds starting retries and admission backoff; it does not
/// cancel an accepted command or shorten the transport's existing 30s exchange.
pub(crate) fn retry_application<T>(
    attempt: impl FnMut(bool) -> Result<T, RuntimeCallError>,
) -> Result<T, RuntimeCallError> {
    let deadline = crate::git_process::current_budget()
        .deadline()
        .min(Instant::now() + Duration::from_secs(25));
    retry_until(deadline, attempt)
}

fn retry_until<T>(
    deadline: Instant,
    mut attempt: impl FnMut(bool) -> Result<T, RuntimeCallError>,
) -> Result<T, RuntimeCallError> {
    let mut reconnect = false;
    let mut transport_retried = false;
    let mut backoff = Duration::from_millis(50);
    for index in 0..128 {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "runtime application retry deadline",
            )
            .into());
        }
        match attempt(reconnect) {
            Err(RuntimeCallError::Busy) => {
                reconnect = false;
                let remaining = deadline.saturating_duration_since(Instant::now());
                if index == 127 || remaining.is_zero() {
                    return Err(RuntimeCallError::Busy);
                }
                std::thread::sleep(backoff.min(remaining));
                if Instant::now() >= deadline {
                    return Err(RuntimeCallError::Busy);
                }
                backoff = (backoff * 2).min(Duration::from_millis(250));
            }
            Err(error @ RuntimeCallError::Transport(_)) => {
                if transport_retried || index == 127 || Instant::now() >= deadline {
                    return Err(error);
                }
                transport_retried = true;
                reconnect = true;
            }
            result => return result,
        }
    }
    unreachable!("final failed attempt returns before loop end")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::protocol::DomainError;

    #[test]
    fn repeated_busy_can_reach_success_without_reconnecting() {
        let mut flags = vec![];
        let result = retry_until(Instant::now() + Duration::from_secs(3), |reconnect| {
            flags.push(reconnect);
            if flags.len() <= 3 {
                Err(RuntimeCallError::Busy)
            } else {
                Ok("accepted")
            }
        })
        .unwrap();
        assert_eq!(result, "accepted");
        assert_eq!(flags, vec![false; 4]);
    }

    #[test]
    fn always_busy_stops_at_absolute_deadline() {
        let start = Instant::now();
        let mut count = 0;
        let result: Result<(), _> = retry_until(start + Duration::from_millis(120), |_| {
            count += 1;
            Err(RuntimeCallError::Busy)
        });
        assert!(matches!(result, Err(RuntimeCallError::Busy)));
        assert!((1..=3).contains(&count));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn transport_reconnect_budget_is_not_renewed_by_busy() {
        let mut flags = vec![];
        let result: Result<(), _> =
            retry_until(Instant::now() + Duration::from_secs(3), |reconnect| {
                flags.push(reconnect);
                Err(match flags.len() {
                    1 | 3 => RuntimeCallError::Busy,
                    _ => std::io::Error::other("lost response").into(),
                })
            });
        assert!(matches!(result, Err(RuntimeCallError::Transport(_))));
        assert_eq!(flags, vec![false, false, true, false]);
    }

    #[test]
    fn domain_conflict_is_terminal_and_expired_budget_starts_nothing() {
        let mut count = 0;
        let result: Result<(), _> = retry_until(Instant::now() + Duration::from_secs(1), |_| {
            count += 1;
            Err(RuntimeCallError::Domain(DomainError::RevisionConflict {
                message: "original conflict".into(),
                current_revision: 9,
                current_plan: None,
            }))
        });
        assert_eq!(count, 1);
        assert!(matches!(
            result,
            Err(RuntimeCallError::Domain(DomainError::RevisionConflict {
                current_revision: 9,
                ..
            }))
        ));
        let result: Result<(), _> = retry_until(Instant::now(), |_| panic!("expired admission"));
        assert!(matches!(result, Err(RuntimeCallError::Transport(_))));
    }
}
