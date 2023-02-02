use crate::git_manager::{get_git_manager, GitHubRepo, GitManagerCloneError, GitReference};
use git2::{Cred, RemoteCallbacks};
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio;
use std::env;
use std::path::Path;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Status {
    Idle,
    FetchingRepo,
    RepoReady,
    ErrorFetchingRepo,
}

#[derive(Debug)]
struct InnerContext {
    pub status: Status,
    pub repo: GitHubRepo,
    pub repo_reference: GitReference,
    pub paths: Vec<String>,
    pub handle: Option<rocket::tokio::task::JoinHandle<Result<(), GitManagerCloneError>>>,
    id: Uuid,
}

pub struct Context {
    inner: Arc<RwLock<InnerContext>>,
}

impl Clone for InnerContext {
    fn clone(&self) -> Self {
        Self {
            status: self.status.clone(),
            repo: self.repo.clone(),
            repo_reference: self.repo_reference.clone(),
            paths: self.paths.clone(),
            id: self.id,
            handle: None,
        }
    }
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

    pub async fn fetch_repo(&mut self) {
        let git_manager = get_git_manager();

        let inner;
        {
            inner = self.inner.read().unwrap().clone();
        }

        let future = match inner.clone().repo_reference {
            GitReference::Commit(commit) => tokio::spawn(async move {
                let inner = inner.clone();
                get_git_manager().clone_local_commit(&inner.repo, &commit, inner.id)
            }),
            GitReference::Branch(branch) => tokio::spawn(async move {
                let inner = inner.clone();
                get_git_manager().clone_local_branch(&inner.repo, &branch, inner.id)
            }),
        };
        let mut inner = self.inner.write().unwrap();
        inner.handle = Some(future);
    }
}
