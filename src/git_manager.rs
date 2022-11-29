use crate::data_dir::{DataDir, DataDirError};
use git2::build::RepoBuilder;
use git2::{Cred, Error, RemoteCallbacks};
use sha2::{Digest, Sha256, Sha512};
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use url::{ParseError, Url};

#[derive(Error, Debug)]
pub enum GitManagerInitError {
    #[error("Directory not initialized")]
    DirectoryNotInitialized,
    #[error("Directory not writable")]
    DirectoryNotWritable,
    #[error("IO Error {}", .0)]
    IOError(std::io::Error),
}

impl From<std::io::Error> for GitManagerInitError {
    fn from(err: std::io::Error) -> Self {
        GitManagerInitError::IOError(err)
    }
}

/// Data structure to manage docker-compose execution
#[derive(Clone, Debug)]
pub struct GitManager<'a> {
    data_dir: &'a DataDir,
}

impl<'a> GitManager<'a> {
    /// Returns an GitManager instance.
    ///
    /// This method needs the data_dir to work on, it mainly will use the `repos` subdirectory to download code.
    ///
    /// # Arguments
    ///
    /// * `data_dir` - A DataDir to be used for installing the bundled docker-compose binary.
    ///
    /// # Examples
    ///
    /// ```
    /// use data_dir::DataDir;
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent");
    /// let git_manager = GitManager::new(data_dir);
    /// ```
    ///
    pub fn new(data_dir: &'a DataDir) -> Result<Self, GitManagerInitError> {
        let repos_path = match data_dir.repos_path() {
            Err(DataDirError::DirectoryNotInitialized) => {
                return Err(GitManagerInitError::DirectoryNotInitialized)
            }
            Ok(repos_path) => repos_path,
        };

        let md = fs::metadata(repos_path)?;
        let permissions = md.permissions();
        let readonly = permissions.readonly();
        if readonly {
            return Err(GitManagerInitError::DirectoryNotWritable);
        }

        Ok(Self { data_dir })
    }

    ///
    /// Fetches the repository and saves it into a directory inside data_dir as
    /// a bare repo, if the repository already exists, it downloads objects and refs (git fetch).
    ///
    /// # Arguments:
    ///
    /// * `repo` - A struct that implements RemoteRepo containing the repo to clone.
    ///
    pub fn fetch(&self, repo: &impl RemoteRepo) -> Result<(), git2::Error> {
        // Prepare fetch options.
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(auth_callbacks());

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder
            .bare(true)
            .clone(&repo.url(), &repo.target_dir(self.data_dir))?;
        Ok(())
    }
}

// @TODO: Deal with auth
fn auth_callbacks() -> RemoteCallbacks<'static> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key(
            username_from_url.unwrap(),
            None,
            Path::new(&format!("{}/.ssh/id_rsa", env::var("HOME").unwrap())),
            None,
        )
    });
    callbacks
}

pub trait RemoteRepo {
    fn target_dir(&self, data_dir: &DataDir) -> PathBuf;
    fn url(&self) -> String;
}

struct GitHubRepo {
    user: String,
    repo: String,
}

impl RemoteRepo for GitHubRepo {
    fn target_dir(&self, data_dir: &DataDir) -> PathBuf {
        data_dir
            .repos_path()
            .unwrap()
            .join("github")
            .join(self.user.clone())
            .join(self.repo.clone())
    }

    fn url(&self) -> String {
        format!("git@github.com:{}/{}.git", self.user, self.repo)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::data_dir::DataDir;
    use std::{thread, time};
    use tempfile::{tempdir, TempDir};

    #[test]
    fn new() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        assert!(manager.is_ok());
    }

    #[test]
    fn fetch_with_valid_url() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let repo = GitHubRepo {
            user: String::from("magec"),
            repo: String::from("utf8mb4rails"),
        };

        let manager = manager.unwrap();
        let result = manager.fetch(&repo);
        println!("{:?}", result);
    }

    // Helpers
    fn temp_data_dir() -> (TempDir, DataDir) {
        let directory = tempdir().unwrap();
        let mut data_dir = DataDir::new(directory.path().to_path_buf());
        data_dir.init().unwrap();

        (directory, data_dir)
    }

    fn manager_instance(data_dir: &DataDir) -> Result<GitManager, GitManagerInitError> {
        GitManager::new(&data_dir)
    }
}
