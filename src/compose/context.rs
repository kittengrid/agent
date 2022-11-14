use crate::compose;
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
        let repo: String;
        {
            let ctx = self.read().unwrap();
            repo = ctx.run.repo.clone();
        }

        // Prepare callbacks.
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            Cred::ssh_key(
                username_from_url.unwrap(),
                None,
                Path::new(&format!("{}/.ssh/id_rsa", env::var("HOME").unwrap())),
                None,
            )
        });

        // Prepare fetch options.
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);

        // Prepare builder.
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);

        // Clone the project.
        builder
            .clone(repo.as_str(), Path::new("/tmp/testing-123"))
            .unwrap();
        self.set_status(Status::Fetched);
    }
}
