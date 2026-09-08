//! Lazy mutation transport, independent of optional map projection state.
use std::path::{Path, PathBuf};

use crate::{
    error::DevMapError,
    git::SourceWorkspace,
    mutation::{MutationCommand, MutationResult},
    proxy::ProxyMode,
    runtime::{PreparedMutation, RuntimeCallError, RuntimeClient, protocol::DomainError},
};

pub struct MutationProxy {
    mode: ProxyMode,
    client: Option<Box<RuntimeClient>>,
    binding: Option<SourceBinding>,
}
#[derive(Debug, PartialEq, Eq)]
struct SourceBinding {
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
}
impl MutationProxy {
    pub fn new(mode: ProxyMode) -> Self {
        Self {
            mode,
            client: None,
            binding: None,
        }
    }
    pub fn execute(
        &mut self,
        workspace: &SourceWorkspace,
        command: &MutationCommand,
    ) -> Result<MutationResult, DevMapError> {
        crate::git_process::with_operation(|| self.execute_inner(workspace, command))
    }
    fn execute_inner(
        &mut self,
        workspace: &SourceWorkspace,
        command: &MutationCommand,
    ) -> Result<MutationResult, DevMapError> {
        let source = self.bind_source(workspace)?;
        if matches!(self.mode, ProxyMode::Direct) {
            return command.execute(workspace);
        }
        // Allocate/serialize before attempt one. Neither reconnect nor Busy can
        // regenerate domain identity, times, observed HEAD, or request bytes.
        let prepared = PreparedMutation::new(command).map_err(domain_error)?;
        retry(|reconnect| {
            if reconnect {
                self.client = None;
            }
            self.attempt(&source, &prepared)
        })
        .map_err(domain_error)
    }
    fn bind_source(&mut self, workspace: &SourceWorkspace) -> Result<PathBuf, DevMapError> {
        let binding = SourceBinding {
            root: std::fs::canonicalize(&workspace.root)?,
            git_dir: std::fs::canonicalize(&workspace.git_dir)?,
            common_dir: std::fs::canonicalize(&workspace.git_common_dir)?,
        };
        if self.binding.as_ref().is_some_and(|old| old != &binding) {
            return Err(DevMapError::InvalidDomain("mutation proxy source changed"));
        }
        let source = binding.root.clone();
        self.binding = Some(binding);
        Ok(source)
    }
    fn attempt(
        &mut self,
        source: &Path,
        prepared: &PreparedMutation,
    ) -> Result<MutationResult, RuntimeCallError> {
        if self.client.is_none() {
            self.client = Some(Box::new(RuntimeClient::connect(source)?));
        }
        self.client
            .as_mut()
            .expect("connection established")
            .mutate_prepared(prepared)
    }
}
fn retry<T>(
    attempt: impl FnMut(bool) -> Result<T, RuntimeCallError>,
) -> Result<T, RuntimeCallError> {
    crate::runtime::retry_application(attempt)
}
fn domain_error(error: RuntimeCallError) -> DevMapError {
    match error {
        RuntimeCallError::Domain(DomainError::RevisionConflict {
            current_revision,
            current_plan,
            ..
        }) => DevMapError::RoutePlanConflict {
            revision: current_revision,
            current_plan,
        },
        other => DevMapError::Runtime(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_source_binding_is_canonical_and_rejects_root_gitdir_or_common_switches() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["root", "git", "common", "other"] {
            std::fs::create_dir(temp.path().join(name)).unwrap();
        }
        let workspace = SourceWorkspace {
            root: temp.path().join("root"),
            git_dir: temp.path().join("git"),
            git_common_dir: temp.path().join("common"),
            branch: None,
            head: "a".repeat(40),
        };
        let mut proxy = MutationProxy::new(ProxyMode::Shared);
        let bound = proxy.bind_source(&workspace).unwrap();
        let mut alias = workspace.clone();
        alias.root = alias.root.join(".");
        assert_eq!(proxy.bind_source(&alias).unwrap(), bound);
        for field in ["root", "git", "common"] {
            let mut changed = workspace.clone();
            match field {
                "root" => changed.root = temp.path().join("other"),
                "git" => changed.git_dir = temp.path().join("other"),
                _ => changed.git_common_dir = temp.path().join("other"),
            }
            assert!(matches!(
                proxy.bind_source(&changed),
                Err(DevMapError::InvalidDomain("mutation proxy source changed"))
            ));
            assert_eq!(proxy.bind_source(&workspace).unwrap(), bound);
            assert!(proxy.client.is_none());
        }
    }
    #[test]
    fn bounded_retry_reconnects_transport_once_and_keeps_domain_terminal() {
        for busy in [false, true] {
            let mut attempts = vec![];
            let payload = String::from("one immutable prepared payload");
            let result = retry(|reconnect| {
                attempts.push((reconnect, payload.clone()));
                if attempts.len() == 1 {
                    if busy {
                        Err(RuntimeCallError::Busy)
                    } else {
                        Err(std::io::Error::other("lost reply").into())
                    }
                } else {
                    Ok(())
                }
            });
            result.unwrap();
            assert_eq!(attempts, vec![(false, payload.clone()), (!busy, payload)]);
        }
        let mut count = 0;
        let error = retry::<()>(|_| {
            count += 1;
            Err(RuntimeCallError::Domain(DomainError::Domain {
                message: "original".into(),
            }))
        })
        .unwrap_err();
        assert_eq!(count, 1);
        assert_eq!(domain_error(error).to_string(), "original");
        let error = domain_error(RuntimeCallError::Domain(DomainError::RevisionConflict {
            message: "route plan revision conflict: current revision is 7".into(),
            current_revision: 7,
            current_plan: None,
        }));
        assert!(matches!(
            error,
            DevMapError::RoutePlanConflict {
                revision: 7,
                current_plan: None
            }
        ));
        assert_eq!(
            error.to_string(),
            "route plan revision conflict: current revision is 7"
        );
    }
}
