//! Client-owned presentation state over the same direct or shared application.
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    application::{ClientView, RepositoryApplication},
    dock::{DockReadModel, ObservedTask},
    error::DevMapError,
    git::SourceGitInspector,
    runtime::{RuntimeCallError, RuntimeClient},
};

pub type SharedQueryProxy = Arc<Mutex<QueryProxy>>;

/// Library callers explicitly choose Direct; the executable chooses Shared.
#[derive(Clone, Copy)]
pub enum ProxyMode {
    Direct,
    Shared,
}

enum Backend {
    Direct(Box<RepositoryApplication>),
    Shared(Option<Box<RuntimeClient>>),
}

pub struct QueryProxy {
    view: ClientView,
    backend: Backend,
    last_refresh: Instant,
    refresh_failed: bool,
    observations: serde_json::Value,
    summary: crate::summary::SummaryState,
}

impl QueryProxy {
    pub fn open_direct(source: &Path) -> Result<Self, DevMapError> {
        Self::open(source, ProxyMode::Direct)
    }

    /// Uses the current executable's authenticated runtime helpers. Embedded
    /// consumers without those CLI entrypoints must explicitly use Direct.
    pub fn open_shared(source: &Path) -> Result<Self, DevMapError> {
        Self::open(source, ProxyMode::Shared)
    }

    pub fn open(source: &Path, mode: ProxyMode) -> Result<Self, DevMapError> {
        let workspace = SourceGitInspector::open(source)?.workspace_allow_unborn()?;
        let backend = match mode {
            ProxyMode::Direct => {
                Backend::Direct(Box::new(RepositoryApplication::open(&workspace)?))
            }
            ProxyMode::Shared => Backend::Shared(None),
        };
        let mut proxy = Self {
            view: ClientView::new(workspace),
            backend,
            last_refresh: Instant::now(),
            refresh_failed: false,
            observations: serde_json::Value::Null,
            summary: crate::summary::SummaryState::default(),
        };
        proxy.refresh(OffsetDateTime::now_utc())?;
        Ok(proxy)
    }

    pub fn client_view(&self) -> &ClientView {
        &self.view
    }

    pub fn snapshot(&self) -> &DockReadModel {
        self.view
            .snapshot()
            .expect("successful construction includes an initial projection")
    }

    pub fn refresh(&mut self, now: OffsetDateTime) -> Result<&DockReadModel, DevMapError> {
        self.refresh_failed = true;
        // This exact query (including prior heads and original inventory time)
        // survives a reconnect; the owner never owns the presentation counters.
        let query = self.view.query_input()?;
        let result = match &mut self.backend {
            Backend::Direct(app) => app.project(self.view.workspace(), &query, now)?,
            Backend::Shared(client) => {
                shared_call(client, &self.view.workspace().root, |client| {
                    client.query(&query)
                })?
            }
        };
        let accepted = self.view.apply_projection(result)?;
        self.observations = serde_json::json!({
            "git_observed_at":accepted.git_observed_at,
            "git_cycle":accepted.git_cycle,
            "store_generation":accepted.store_generation,
            "store_inputs_observed_at":accepted.store_inputs_observed_at,
            "task_observation":accepted.model.task_observation,
        });
        self.last_refresh = Instant::now();
        self.refresh_failed = false;
        Ok(self.snapshot())
    }

    pub fn refresh_if_due(&mut self, interval: Duration) -> Result<&DockReadModel, DevMapError> {
        if self.refresh_failed || self.last_refresh.elapsed() >= interval {
            self.refresh(OffsetDateTime::now_utc())?;
        }
        Ok(self.snapshot())
    }

    pub(crate) fn summary_result(&mut self, cursor: Option<&str>) -> serde_json::Value {
        if let Some(cursor) = cursor {
            return self.summary.page(cursor);
        }
        if self.refresh(OffsetDateTime::now_utc()).is_err() {
            return crate::summary::error("summary_refresh_failed");
        }
        self.capture_summary()
    }

    /// Called only while constructing the private MCP summary proxy, whose
    /// constructor has just completed its initial verified projection.
    pub(crate) fn open_summary(
        source: &Path,
        mode: ProxyMode,
    ) -> Result<(Self, serde_json::Value), DevMapError> {
        let mut proxy = Self::open(source, mode)?;
        let result = proxy.capture_summary();
        Ok((proxy, result))
    }

    fn capture_summary(&mut self) -> serde_json::Value {
        self.summary.capture(
            self.view
                .snapshot()
                .expect("successful refresh has a projection"),
            self.observations.clone(),
        )
    }

    /// Supplied rows replace this client's visible subset, preserving the public
    /// Dock contract. `complete` describes host coverage, not merge intent. Only
    /// explicit acceptance writes bindings; ordinary refresh keeps the subset.
    pub fn accept_inventory(
        &mut self,
        tasks: Vec<ObservedTask>,
        complete: bool,
        observed_at: OffsetDateTime,
    ) -> Result<&DockReadModel, DevMapError> {
        let mut prior = self.view.query_input()?;
        // Prepare replacement intent without mutating the surviving ClientView:
        // retain source time and history, but do not inherit omitted rows or an
        // earlier execution report. A late command retains the full prior so the
        // application's validated watermark no-op cannot accidentally clear it.
        if self
            .view
            .inventory_observed_at()
            .is_none_or(|old| observed_at >= old)
        {
            prior.tasks.clear();
        }
        let stamp = observed_at.format(&Rfc3339)?;
        let accepted = match &mut self.backend {
            Backend::Direct(app) => app.accept_inventory_query(
                self.view.workspace(),
                prior,
                tasks,
                complete,
                observed_at,
            )?,
            Backend::Shared(client) => {
                shared_call(client, &self.view.workspace().root, |client| {
                    client.accept_inventory(&prior, &tasks, complete, &stamp)
                })?
            }
        };
        self.view.apply_inventory(accepted)?;
        self.refresh(OffsetDateTime::now_utc())
    }
}

fn shared_call<T>(
    client: &mut Option<Box<RuntimeClient>>,
    source: &Path,
    mut call: impl FnMut(&mut RuntimeClient) -> Result<T, RuntimeCallError>,
) -> Result<T, RuntimeCallError> {
    bounded_retry(|reconnect| {
        if reconnect {
            *client = None;
        }
        if client.is_none() {
            *client = Some(Box::new(RuntimeClient::connect(source)?));
        }
        call(client.as_mut().expect("connection established"))
    })
}

/// Shared bounded admission retries; caller payloads stay immutable throughout.
fn bounded_retry<T>(
    attempt: impl FnMut(bool) -> Result<T, RuntimeCallError>,
) -> Result<T, RuntimeCallError> {
    crate::runtime::retry_application(attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_is_bounded_and_domain_failures_are_terminal() {
        for (busy, expected) in [(false, vec![false, true]), (true, vec![false, false])] {
            let mut attempts = vec![];
            let result: Result<(), _> = bounded_retry(|reconnect| {
                attempts.push(reconnect);
                if busy && attempts.len() == 2 {
                    return Ok(());
                }
                Err(if busy {
                    RuntimeCallError::Busy
                } else {
                    RuntimeCallError::Transport(std::io::Error::other("lost response"))
                })
            });
            assert_eq!(result.is_ok(), busy);
            assert_eq!(attempts, expected);
        }
        let mut count = 0;
        let result: Result<(), _> = bounded_retry(|_| {
            count += 1;
            Err(RuntimeCallError::Domain(
                crate::runtime::protocol::DomainError::Domain {
                    message: "original domain message".into(),
                },
            ))
        });
        assert_eq!(count, 1);
        assert_eq!(result.unwrap_err().to_string(), "original domain message");
    }
}
