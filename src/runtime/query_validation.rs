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
    #[cfg(test)]
    test_inspections: Vec<usize>,
    #[cfg(test)]
    test_cold_hook: Option<Box<dyn FnMut(ColdStage) + Send>>,
    #[cfg(test)]
    test_after_cached_projection: Option<Box<dyn FnMut() + Send>>,
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColdStage {
    CandidateCaptured,
    AuthoritativeInspected,
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
    pub(crate) fn configuration(&self) -> &QueryConfiguration {
        &self.configuration
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
    fn inspect(&mut self, identity: &Identity) -> Result<SourceWorkspace, DevMapError> {
        #[cfg(test)]
        let before = crate::git_process::test_spawn_count();
        let result = SourceGitInspector::open(&identity.source)?.workspace_allow_unborn();
        #[cfg(test)]
        self.test_inspections
            .push(crate::git_process::test_spawn_count() - before);
        result
    }
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
            let mut source_stale = false;
            if let Some(application) = app.as_mut() {
                let attempt = application.try_project_query_boundary(proof, query, now, || {
                    #[cfg(test)]
                    if let Some(hook) = self.test_after_cached_projection.as_mut() {
                        hook();
                    }
                })?;
                match attempt {
                    crate::application::QueryBoundaryAttempt::Ready(result) => return Ok(*result),
                    crate::application::QueryBoundaryAttempt::SourceStale => source_stale = true,
                    crate::application::QueryBoundaryAttempt::Ineligible => {}
                }
            }
            if !source_stale && proof.valid()? {
                if app.is_none() {
                    *app = Some(RepositoryApplication::open(proof.workspace()?)?);
                }
                let result = app
                    .as_mut()
                    .unwrap()
                    .project_verified_query(proof, query, now)?;
                #[cfg(test)]
                if let Some(hook) = self.test_after_cached_projection.as_mut() {
                    hook();
                }
                if proof.valid()? {
                    return Ok(result);
                }
            }
            self.sources.remove(&key);
        }
        // Candidate paths come only from authenticated Hello. They authorize
        // neither application construction nor a projection before fresh Git.
        let candidate = SourceWorkspace {
            root: identity.source.clone(),
            git_dir: identity.git_dir.clone(),
            git_common_dir: identity.common.clone(),
            head: String::new(),
            branch: None,
        };
        let configuration = QueryConfiguration::acquire(&candidate)?;
        #[cfg(test)]
        if let Some(hook) = self.test_cold_hook.as_mut() {
            hook(ColdStage::CandidateCaptured);
        }
        let workspace = self.inspect(identity)?;
        if std::fs::canonicalize(&workspace.root)? != identity.source
            || std::fs::canonicalize(&workspace.git_dir)? != identity.git_dir
            || std::fs::canonicalize(&workspace.git_common_dir)? != identity.common
        {
            return Err(DevMapError::Store("authenticated source changed".into()));
        }
        #[cfg(test)]
        if let Some(hook) = self.test_cold_hook.as_mut() {
            hook(ColdStage::AuthoritativeInspected);
        }
        if app.is_none() {
            *app = Some(RepositoryApplication::open(&workspace)?);
        }
        if let Some(configuration) = configuration {
            // Authoritative inspection remains inside the candidate capture /
            // proof.valid sandwich. Unsupported or changed evidence falls back.
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
