use crate::compose;
use crate::git_manager::{get_git_manager, GitHubRepo};
use git2::{Cred, RemoteCallbacks};
use rocket::serde::{Deserialize, Serialize};
use std::env;
use std::path::Path;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize, Copy, Clone)]
pub enum Status {
    Fetching,
    Reading,
    Fetched,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
struct InnerContext {
    status: Status,
    run: compose::Run,
    id: Uuid,
}

pub struct Context {
    inner: Arc<RwLock<InnerContext>>,
}

impl Context {
    pub fn new(status: Status, run: compose::Run, id: Uuid) -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerContext { status, run, id })),
        }
    }

    pub fn status(&self) -> Status {
        self.read().unwrap().status
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

    pub async fn fetch_repo(&self) {
        let repo: GitHubRepo;
        {
            let ctx = self.read().unwrap();
            repo = ctx.run.repo.clone();
        }
        let git_manager = get_git_manager();
        git_manager.fetch_remote(&repo);
        git_manager.clone_local_branch(repo, branch, uuid)
    }
}
