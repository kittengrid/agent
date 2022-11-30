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

#[derive(Error, Debug)]
pub enum GitManagerFetchError {
    #[error("Could not delete target_dir to recreate repository ({})", .0)]
    CouldNotDeleteRepoDir(std::io::Error),
    #[error("Git Error {}", .0)]
    GitError(git2::Error),
}
impl From<git2::Error> for GitManagerFetchError {
    fn from(err: git2::Error) -> Self {
        GitManagerFetchError::GitError(err)
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
    pub fn fetch(&self, repo: &impl RemoteRepo) -> Result<(), GitManagerFetchError> {
        // Prepare fetch options.
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(auth_callbacks());

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        let target_dir = repo.target_dir(self.data_dir);
        let result = builder.bare(true).clone(&repo.url(), &target_dir);
        match result {
            Ok(_) => Ok(()),
            Err(err) => {
                if err.code() == git2::ErrorCode::Exists && err.class() == git2::ErrorClass::Invalid
                {
                    warn!("Repo directory exists, trying to fetch.");
                    // Let's try to fetch refspects
                    match git2::Repository::open(&target_dir) {
                        Err(_err) => {
                            warn!("Repo directory exists but is not a valid repo, will delete it and try to clone again.");
                            match fs::remove_dir_all(&target_dir) {
                                Ok(()) => return self.fetch(repo),
                                Err(err) => {
                                    return Err(GitManagerFetchError::CouldNotDeleteRepoDir(err))
                                }
                            }
                        }
                        Ok(repo) => {
                            repo.find_remote("origin")?.fetch_refspecs()?;
                            return Ok(());
                        }
                    }
                }
                Err(GitManagerFetchError::GitError(err))
            }
        }
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

pub struct GitHubRepo {
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
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let manager = manager.unwrap();
        let result = manager.fetch(&repo);
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_with_valid_url_twice() {
        env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .init();

        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir).unwrap();
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let manager = manager;
        manager.fetch(&repo).unwrap();
        assert!(manager.fetch(&repo).is_ok());
    }

    #[test]
    fn fetch_with_garbage_in_destination() {
        env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .init();

        let (_tempdir, data_dir) = temp_data_dir();
        let target_dir = data_dir
            .repos_path()
            .unwrap()
            .join("github")
            .join("kittengrid")
            .join("deb-s3");

        fs::create_dir_all(&target_dir).unwrap();
        std::fs::File::create(target_dir.join("garbage")).expect("create failed");
        let manager = manager_instance(&data_dir).unwrap();
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        assert!(manager.fetch(&repo).is_ok());
    }

    #[test]
    fn fetch_with_invalid_url() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("I_DON_THINK_WE_WILL_EVER_HAVE_THIS_REPO"),
        };

        let manager = manager.unwrap();
        assert!(matches!(
            manager.fetch(&repo).err().unwrap(),
            GitManagerFetchError
        ));
    }

    // Helpers
    fn temp_data_dir() -> (TempDir, DataDir) {
        let directory = tempdir().unwrap();
        let mut data_dir = DataDir::new(directory.path().to_path_buf());
        data_dir.init().unwrap();

        (directory, data_dir)
    }

    fn manager_instance(data_dir: &DataDir) -> Result<GitManager, GitManagerInitError> {
        GitManager::new(data_dir)
    }
}
