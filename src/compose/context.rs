use crate::git_manager::{GitHubRepo, GitManagerCloneError, GitReference};

use crate::docker_compose::{DockerCompose, DockerComposeInitError, DockerComposeRunError};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Status {
    Idle,
    FetchingRepo,
    RepoReady,
    ErrorFetchingRepo(GitManagerCloneError),
    ErrorInitializingDockerCompose(DockerComposeInitError),
    ComposeInitialized,
    ComposeCreated,
    ComposeCreatingError(DockerComposeRunError),
    ComposeStarted,
    ComposeStartingError(DockerComposeRunError),
    ComposeStopping,
    ComposeStopped,
    ComposeStoppingError(DockerComposeRunError),
}

#[derive(Debug)]
struct InnerContext<'a> {
    pub status: Status,
    pub repo: GitHubRepo,
    pub repo_reference: GitReference,
    pub paths: Vec<String>,
    pub handle: Option<rocket::tokio::task::JoinHandle<()>>,
    pub docker_compose: Option<DockerCompose<'a>>,
    id: Uuid,
}

pub struct Context<'a> {
    inner: Arc<RwLock<InnerContext<'a>>>,
}

impl<'a> Context<'a> {
    pub fn new(
        status: Status,
        repo: GitHubRepo,
        repo_reference: GitReference,
        paths: Vec<String>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerContext {
                status,
                repo,
                repo_reference,
                paths,
                docker_compose: None,
                id: Uuid::new_v4(),
                handle: None,
            })),
        }
    }

    pub fn repo(&self) -> GitHubRepo {
        self.read().unwrap().repo.clone()
    }

    pub fn repo_reference(&self) -> GitReference {
        self.read().unwrap().repo_reference.clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.read().unwrap().paths.clone()
    }

    pub fn status(&self) -> Status {
        self.read().unwrap().status.clone()
    }

    pub fn docker_compose(&self) -> Option<DockerCompose<'a>> {
        self.inner.read().unwrap().docker_compose.clone()
    }

    pub fn set_docker_compose(&self, docker_compose: DockerCompose<'a>) {
        self.inner.write().unwrap().docker_compose = Some(docker_compose);
    }

    pub fn id(&self) -> Uuid {
        self.inner.read().unwrap().id
    }

    pub fn set_status(&self, status: Status) {
        self.inner.write().unwrap().status = status;
    }

    pub fn set_handle(&self, handle: rocket::tokio::task::JoinHandle<()>) {
        self.inner.write().unwrap().handle = Some(handle);
    }

    fn read(&self) -> LockResult<RwLockReadGuard<'_, InnerContext<'_>>> {
        self.inner.read()
    }

    pub fn clone(&self) -> Self {
        Context {
            inner: self.inner.clone(),
        }
    }
}
