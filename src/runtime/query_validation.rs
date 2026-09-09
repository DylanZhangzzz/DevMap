//! Read-only query source proof; never authorizes mutations or inventory writes.
use super::Identity;
use crate::{
    application::{ApplicationSnapshot, ClientQuery, RepositoryApplication},
    error::DevMapError,
    git::{SourceGitInspector, SourceWorkspace},
    git_relationship::QueryConfiguration,
};
use std::{collections::BTreeMap, path::PathBuf};
use time::OffsetDateTime;
fn source_changed() -> DevMapError {
    DevMapError::Store("authenticated source changed".into())
}

/// Connection-local, non-wire observation established after authenticated Hello.
#[derive(Clone)]
pub(super) enum QueryOrigin {
    Verified {
        workspace: Box<SourceWorkspace>,
        origin: crate::store::migration::VerifiedCurrentOrigin,
        common_identity: crate::fs_security::FileIdentity,
    },
    Refused,
}
impl QueryOrigin {
    pub(super) fn for_connection(identity: &Identity) -> Self {
        // A failed Hello observation is permanent for this connection. It must
        // never become the absence of a check after repair or path replacement.
        Self::capture(identity).unwrap_or(Self::Refused)
    }
    pub(super) fn capture(identity: &Identity) -> Result<Self, DevMapError> {
        let workspace = SourceWorkspace {
            root: identity.source.clone(),
            git_dir: identity.git_dir.clone(),
            git_common_dir: identity.common.clone(),
            head: String::new(),
            branch: None,
        };
        let common_identity =
            crate::fs_security::checked_directory_identity(&workspace.git_common_dir)?;
        let origin = crate::store::migration::application_anchor(&workspace)?;
        let seal = Self::Verified {
            workspace: Box::new(workspace),
            origin,
            common_identity,
        };
        seal.validate()?;
        Ok(seal)
    }
    fn validate(&self) -> Result<(), DevMapError> {
        let Self::Verified {
            workspace,
            origin,
            common_identity,
        } = self
        else {
            return Err(source_changed());
        };
        if crate::fs_security::checked_directory_identity(&workspace.git_common_dir)
            .map_err(|_| source_changed())?
            != *common_identity
            || crate::store::migration::application_anchor(workspace)
                .map_err(|_| source_changed())?
                != *origin
        {
            return Err(DevMapError::Store("authenticated source changed".into()));
        }
        Ok(())
    }
}
pub(super) fn with_query_origin<T>(
    bytes: &[u8],
    origin: Option<&QueryOrigin>,
    operation: impl FnOnce() -> Result<T, DevMapError>,
) -> Result<T, DevMapError> {
    let Some(origin) = origin else {
        return operation();
    };
    let is_query = matches!(
        serde_json::from_slice::<super::protocol::ApplicationRequest>(bytes)?,
        super::protocol::ApplicationRequest::Query { .. }
    );
    if is_query {
        origin.validate()?;
    }
    let result = operation()?;
    if is_query {
        origin.validate()?;
    }
    Ok(result)
}

#[derive(Default)]
pub(super) struct QueryValidation {
    sources: BTreeMap<(PathBuf, PathBuf, PathBuf), VerifiedQuerySource>,
}
/// Only successful authenticated source validation creates this private capability.
pub(crate) struct VerifiedQuerySource {
    workspace: SourceWorkspace,
    origin: crate::store::migration::VerifiedCurrentOrigin,
    common_identity: crate::fs_security::FileIdentity,
    configuration: QueryConfiguration,
}
impl VerifiedQuerySource {
    pub(crate) fn workspace(&self) -> Result<&SourceWorkspace, DevMapError> {
        if crate::fs_security::checked_directory_identity(&self.workspace.git_common_dir)
            .map_err(|_| source_changed())?
            != self.common_identity
            || crate::store::migration::application_anchor(&self.workspace)
                .map_err(|_| source_changed())?
                != self.origin
        {
            return Err(DevMapError::Store("authenticated source changed".into()));
        }
        Ok(&self.workspace)
    }
    pub(crate) fn configured(&self) -> Option<&str> {
        self.configuration.value()
    }
    fn valid(&self) -> Result<bool, DevMapError> {
        self.workspace()?;
        self.configuration.recheck()
    }
}
impl QueryValidation {
    pub(super) fn project(
        &mut self,
        app: &mut Option<RepositoryApplication>,
        identity: &Identity,
        query: &ClientQuery,
        now: OffsetDateTime,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        crate::git_process::with_operation(|| self.project_inner(app, identity, query, now))
    }
    fn project_inner(
        &mut self,
        app: &mut Option<RepositoryApplication>,
        identity: &Identity,
        query: &ClientQuery,
        now: OffsetDateTime,
    ) -> Result<ApplicationSnapshot, DevMapError> {
        let key = (
            identity.source.clone(),
            identity.git_dir.clone(),
            identity.common.clone(),
        );
        if let Some(proof) = self.sources.get(&key) {
            if proof.valid()? {
                if app.is_none() {
                    *app = Some(RepositoryApplication::open(proof.workspace()?)?);
                }
                let result = app
                    .as_mut()
                    .unwrap()
                    .project_verified_query(proof, query, now)?;
                if proof.valid()? {
                    return Ok(result);
                }
            }
            self.sources.remove(&key);
        }
        let workspace = SourceGitInspector::open(&identity.source)?.workspace_allow_unborn()?;
        if std::fs::canonicalize(&workspace.root)? != identity.source
            || std::fs::canonicalize(&workspace.git_dir)? != identity.git_dir
            || std::fs::canonicalize(&workspace.git_common_dir)? != identity.common
        {
            return Err(DevMapError::Store("authenticated source changed".into()));
        }
        if app.is_none() {
            *app = Some(RepositoryApplication::open(&workspace)?);
        }
        if let Some(configuration) = QueryConfiguration::acquire(&workspace)? {
            let proof = VerifiedQuerySource {
                origin: crate::store::migration::application_anchor(&workspace)?,
                common_identity: crate::fs_security::checked_directory_identity(
                    &workspace.git_common_dir,
                )?,
                workspace: workspace.clone(),
                configuration,
            };
            if proof.valid()? {
                let result = app
                    .as_mut()
                    .unwrap()
                    .project_verified_query(&proof, query, now)?;
                if proof.valid()? {
                    if self.sources.len() >= 16 {
                        self.sources.pop_first();
                    }
                    self.sources.insert(key, proof);
                    return Ok(result);
                }
            }
        }
        // Unsupported/changed inputs retain the original fresh validation path.
        app.as_mut().unwrap().project(&workspace, query, now)
    }
}
#[cfg(test)]
mod tests;
