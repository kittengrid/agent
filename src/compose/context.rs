use crate::git_manager::{GitHubRepo, GitManagerCloneError, GitReference};

use serde::{Deserialize, Serialize};
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Status {
    Idle,
    FetchingRepo,
    RepoReady,
    ErrorFetchingRepo(GitManagerCloneError),
}

#[derive(Debug)]
struct InnerContext {
    pub status: Status,
    pub repo: GitHubRepo,
    pub repo_reference: GitReference,
    pub paths: Vec<String>,
    pub handle: Option<rocket::tokio::task::JoinHandle<()>>,
    id: Uuid,
}

pub struct Context {
    inner: Arc<RwLock<InnerContext>>,
}

impl Context {
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
                id: Uuid::new_v4(),
                handle: None,
            })),
        }
    }

    pub fn status(&self) -> Status {
        self.read().unwrap().status.clone()
    }

    pub fn id(&self) -> Uuid {
        self.inner.read().unwrap().id
    }

    pub fn set_status(&self, status: Status) {
        self.write().unwrap().status = status;
    }

    pub fn set_handle(&self, handle: rocket::tokio::task::JoinHandle<()>) {
        self.write().unwrap().handle = Some(handle);
    }

    fn write(&self) -> LockResult<RwLockWriteGuard<'_, InnerContext>> {
        self.inner.write()
    }

    fn read(&self) -> LockResult<RwLockReadGuard<'_, InnerContext>> {
        self.inner.read()
    }

    pub fn clone(self: &Context) -> Self {
        Context {
            inner: self.inner.clone(),
        }
    }
}
