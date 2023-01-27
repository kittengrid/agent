use crate::data_dir::{get_data_dir, DataDir, DataDirError};
use git2::build::{CheckoutBuilder, CloneLocal, RepoBuilder};
use git2::{Cred, Object, Oid, RemoteCallbacks};
use once_cell::sync::Lazy;
use rocket::serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

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
pub enum GitManagerCloneError {
    #[error("Could not delete target_dir to recreate repository ({})", .0)]
    CouldNotDeleteRepoDir(std::io::Error),
    #[error("Git Error {}", .0)]
    GitError(git2::Error),
}
impl From<git2::Error> for GitManagerCloneError {
    fn from(err: git2::Error) -> Self {
        GitManagerCloneError::GitError(err)
    }
}

/// Data structure to manage docker-compose execution
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
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent".into());
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
    /// Clones locally the repo inside `workdir` directory given a branch.
    /// The idea is having a bare repo as the source, and make cheap (hard link) copies
    /// inside workdir, so data space is minimized and usage is fast.
    ///
    /// # Arguments:
    ///
    /// * `branch` - The branch we want to checkout.
    /// * `repo`   - The repo to be locally cloned.
    /// * `uuid`   - A uuid to be used as workdir directory.
    ///
    pub fn clone_local_branch(
        &self,
        repo: &impl RemoteRepo,
        branch: &str,
        uuid: Uuid,
    ) -> Result<(), GitManagerCloneError> {
        let source_url = format!(
            "file://{}",
            repo.target_dir(self.data_dir).to_str().unwrap()
        );
        let target_dir = self.data_dir.work_path().unwrap().join(uuid.to_string());

        match self
            .builder()
            .bare(false)
            .clone_local(CloneLocal::Local)
            .branch(branch)
            .clone(&source_url, &target_dir)
        {
            Ok(_) => Ok(()),
            Err(err) => Err(GitManagerCloneError::GitError(err)),
        }
    }

    ///
    /// Clones locally the repo inside `workdir` directory given a sha.
    /// The idea is having a bare repo as the source, and make cheap (hard link) copies
    /// inside workdir, so data space is minimized and usage is fast.
    ///
    /// # Arguments:
    ///
    /// * `commit` - The sha of the commit we want to checkout after the clone (default branch will be used for the clone).
    /// * `repo`   - The repo to be locally cloned.
    /// * `uuid`   - A uuid to be used as workdir directory.
    ///
    pub fn clone_local_commit(
        &self,
        repo: &impl RemoteRepo,
        commit: &str,
        uuid: Uuid,
    ) -> Result<(), GitManagerCloneError> {
        let source_url = format!(
            "file://{}",
            repo.target_dir(self.data_dir).to_str().unwrap()
        );
        let target_dir = self.data_dir.work_path().unwrap().join(uuid.to_string());

        match self
            .builder()
            .bare(false)
            .clone_local(CloneLocal::Local)
            .clone(&source_url, &target_dir)
        {
            Ok(repo) => {
                let obj: Object = repo
                    .find_commit(Oid::from_str(commit).unwrap())
                    .unwrap()
                    .into_object();
                repo.checkout_tree(&obj, None).unwrap();
                repo.set_head_detached(obj.id()).unwrap();
                Ok(())
            }
            Err(err) => Err(GitManagerCloneError::GitError(err)),
        }
    }

    ///
    /// If this is the first time this is called, it Clones the repository and saves it into a directory inside data_dir as
    /// a bare repo, if the repository already exists, it downloads objects and refs (git fetch).
    ///
    /// # Arguments:
    ///
    /// * `repo` - A struct that implements RemoteRepo containing the repo to clone.
    ///
    pub fn fetch_remote(&self, repo: &impl RemoteRepo) -> Result<(), GitManagerCloneError> {
        let target_dir = repo.target_dir(self.data_dir);
        let result = self
            .builder()
            .with_checkout(CheckoutBuilder::new())
            .bare(true)
            .clone(&repo.url(), &target_dir);
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
                                Ok(()) => return self.fetch_remote(repo),
                                Err(err) => {
                                    return Err(GitManagerCloneError::CouldNotDeleteRepoDir(err))
                                }
                            }
                        }
                        Ok(repo) => {
                            repo.find_remote("origin")?.fetch_refspecs()?;
                            return Ok(());
                        }
                    }
                }
                Err(GitManagerCloneError::GitError(err))
            }
        }
    }

    fn builder(&self) -> RepoBuilder {
        // Prepare clone options.
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(auth_callbacks());

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);

        builder
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

// Repo from GitHub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRepo {
    user: String,
    repo: String,
}

impl GitHubRepo {
    pub fn new(user: &str, repo: &str) -> GitHubRepo {
        GitHubRepo {
            user: user.to_string(),
            repo: repo.to_string(),
        }
    }
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

// Repo from Url (free), mostly used for testing purposes
pub struct UrlRepo {
    url: String,
}
impl UrlRepo {
    pub fn new(url: String) -> UrlRepo {
        UrlRepo { url }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum GitReference {
    Commit(String),
    Branch(String),
}

impl RemoteRepo for UrlRepo {
    fn target_dir(&self, data_dir: &DataDir) -> PathBuf {
        let mut sha256 = Sha256::new();
        sha256.update(&self.url);

        // We use sha256 hash representation of the url to be used as directory
        // for the repo
        data_dir
            .repos_path()
            .unwrap()
            .join("url")
            .join(format!("{:x}", sha256.finalize()))
    }

    fn url(&self) -> String {
        self.url.clone()
    }
}

static GIT_MANAGER: Lazy<GitManager> = Lazy::new(|| {
    let data_dir = get_data_dir();
    GitManager::new(&data_dir)
});

pub fn get_git_manager() -> GitManager<'static> {
    *GIT_MANAGER
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::data_dir::DataDir;
    use crate::utils::initialize_logger;
    use std::os::unix::fs::MetadataExt;

    use tempfile::{tempdir, TempDir};
    use uuid::uuid;

    #[test]
    fn new() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        assert!(manager.is_ok());
    }

    #[test]
    fn clone_with_valid_url() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let manager = manager.unwrap();
        let result = manager.fetch_remote(&repo);
        assert!(result.is_ok());
    }

    #[test]
    fn clone_with_valid_url_twice() {
        initialize_logger();
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir).unwrap();
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let manager = manager;
        manager.fetch_remote(&repo).unwrap();
        assert!(manager.fetch_remote(&repo).is_ok());
    }

    #[test]
    fn clone_with_garbage_in_destination() {
        initialize_logger();

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

        assert!(manager.fetch_remote(&repo).is_ok());
    }

    #[test]
    fn clone_with_invalid_url() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("I_DON_THINK_WE_WILL_EVER_HAVE_THIS_REPO"),
        };

        let manager = manager.unwrap();
        assert!(matches!(
            manager.fetch_remote(&repo).err().unwrap(),
            GitManagerCloneError
        ));
    }

    #[test]
    fn clone_url_repo() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let repo = UrlRepo::new(test_repo("simple-repo"));

        let manager = manager.unwrap();
        let result = manager.fetch_remote(&repo);
        assert!(Path::new(
            &data_dir
                .repos_path()
                .unwrap()
                .join("url")
                .join("06ed9b5652b73ec6635aee61bc39691569a694966344ad0fdcb5d679b90deee5")
                .join("HEAD")
        )
        .exists());

        assert!(result.is_ok());
    }

    #[test]
    fn clone_local_branch_url_repo() {
        let (_tempdir, data_dir) = temp_data_dir();
        let first_uuid = "f37915a0-7195-11ed-a1eb-0242ac120002";
        let second_uuid = "f37915a0-7195-11ed-a1eb-0242ac120003";
        let manager = manager_instance(&data_dir).unwrap();
        let repo = UrlRepo::new(test_repo("simple-repo"));
        manager.fetch_remote(&repo).unwrap();

        let result = manager.clone_local_branch(
            &repo,
            "main",
            uuid!("f37915a0-7195-11ed-a1eb-0242ac120002"),
        );

        assert!(result.is_ok());
        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(first_uuid)
                .join(".keepme")
        )
        .exists());

        let pack_file = "pack-6a6f56e8f0fd5aa2f35099098e72ada10adc948d.pack";

        let first_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(first_uuid)
                .join(".git")
                .join("objects")
                .join("pack")
                .join(pack_file),
        ))
        .unwrap();

        manager
            .clone_local_branch(&repo, "main", uuid!("f37915a0-7195-11ed-a1eb-0242ac120003"))
            .unwrap();

        let second_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(second_uuid)
                .join(".git")
                .join("objects")
                .join("pack")
                .join(pack_file),
        ))
        .unwrap();

        assert_eq!(first_metadata.ino(), second_metadata.ino());
    }

    #[test]
    fn clone_local_commit_url_repo() {
        let (_tempdir, data_dir) = temp_data_dir();
        let first_uuid = "f37915a0-7195-11ed-a1eb-0242ac120002";
        let second_uuid = "f37915a0-7195-11ed-a1eb-0242ac120003";
        let manager = manager_instance(&data_dir).unwrap();
        let repo = UrlRepo::new(test_repo("simple-repo"));
        manager.fetch_remote(&repo).unwrap();
        manager.fetch_remote(&repo).unwrap();
        let result = manager.clone_local_commit(
            &repo,
            "222a7a3b719453af7574f08591650f6b8ab91bd5",
            uuid!("f37915a0-7195-11ed-a1eb-0242ac120002"),
        );

        assert!(result.is_ok());
        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(first_uuid)
                .join(".keepme")
        )
        .exists());

        let pack_file = "pack-6a6f56e8f0fd5aa2f35099098e72ada10adc948d.pack";

        let first_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(first_uuid)
                .join(".git")
                .join("objects")
                .join("pack")
                .join(pack_file),
        ))
        .unwrap();

        manager
            .clone_local_commit(
                &repo,
                "222a7a3b719453af7574f08591650f6b8ab91bd5",
                uuid!("f37915a0-7195-11ed-a1eb-0242ac120003"),
            )
            .unwrap();

        let second_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(second_uuid)
                .join(".git")
                .join("objects")
                .join("pack")
                .join(pack_file),
        ))
        .unwrap();

        assert_eq!(first_metadata.ino(), second_metadata.ino());
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

    fn test_repo(repo: &str) -> String {
        format!(
            "file://{}/resources/test/{}",
            env!("CARGO_MANIFEST_DIR"),
            repo
        )
    }
}
