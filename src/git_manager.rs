use crate::data_dir::{get_data_dir, DataDir, DataDirError};
use git2::build::{CheckoutBuilder, CloneLocal, RepoBuilder};
use git2::{Config, Cred, FetchOptions, Object, Oid, RemoteCallbacks};
use once_cell::sync::Lazy;
use rocket::serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::str;
use std::sync::Mutex;
use thiserror::Error;
use uuid::Uuid;

///
/// GitManager is an abstraction of git2 library that simplifies the usage.
/// It is coupled with DataDir.
/// The main usage is clonning git repositories in a directory and then make local copies
/// of it when new jobs start.

#[derive(Error, Debug, Serialize)]
pub enum GitManagerInitError {
    #[error("Directory not initialized")]
    DirectoryNotInitialized,
    #[error("Directory not writable")]
    DirectoryNotWritable,
    #[error("IO Error {}", .0)]
    IOError(String),
}

impl From<std::io::Error> for GitManagerInitError {
    fn from(err: std::io::Error) -> Self {
        GitManagerInitError::IOError(err.to_string())
    }
}

#[derive(Error, Debug, Serialize, Deserialize, Clone)]
pub enum GitManagerCloneError {
    #[error("Could not delete target_dir to recreate repository ({})", .0)]
    CouldNotDeleteRepoDir(String),
    #[error("Git Error {}", .0)]
    GitError(String),
}

impl From<git2::Error> for GitManagerCloneError {
    fn from(err: git2::Error) -> Self {
        GitManagerCloneError::GitError(err.to_string())
    }
}

/// Data structure to manage docker-compose execution
pub struct GitManager<'a> {
    data_dir: &'a DataDir,
    mutex: Mutex<usize>,
}

impl<'a> GitManager<'a> {
    /// Returns an GitManager instance.
    ///
    /// # Arguments
    ///
    /// * `data_dir` - DataDir to download code to.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lib::*;
    /// use data_dir::DataDir;
    /// use git_manager::GitManager;
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent".into());
    /// let git_manager = GitManager::new(&data_dir);
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

        Ok(Self {
            data_dir,
            mutex: Mutex::new(0),
        })
    }

    ///
    /// Clones locally the repo inside `workdir` directory given a GitReference.
    /// The idea is having a bare repo as the source, and make cheap (hard link) copies
    /// inside workdir, so data space is minimized and usage is fast.
    ///
    /// # Arguments:
    ///
    /// * `reference` - The branch we want to checkout.
    /// * `repo`      - The repo to be locally cloned.
    /// * `uuid`      - A uuid to be used as workdir directory.
    ///
    pub fn clone_local_by_reference(
        &self,
        repo: &impl RemoteRepo,
        reference: &GitReference,
        uuid: Uuid,
    ) -> Result<(), GitManagerCloneError> {
        match reference {
            GitReference::Commit(commit) => self.clone_local_by_commit(repo, commit, uuid),
            GitReference::Branch(branch) => self.clone_local_by_branch(repo, branch, uuid),
        }
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
    pub fn clone_local_by_branch(
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
        self.fetch(&[branch], repo)?;
        self.builder()
            .bare(false)
            .clone_local(CloneLocal::Local)
            .branch(branch)
            .clone(&source_url, &target_dir)?;

        Ok(())
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
    pub fn clone_local_by_commit(
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
            Err(err) => Err(GitManagerCloneError::GitError(err.to_string())),
        }
    }

    pub fn fetch(
        &self,
        branches: &[&str],
        repo: &impl RemoteRepo,
    ) -> Result<(), GitManagerCloneError> {
        let target_dir = repo.target_dir(self.data_dir);
        let repo = git2::Repository::open(target_dir)?;
        let mut remote = repo.find_remote("origin")?;
        {
            let _guard = self.mutex.lock().unwrap();
            remote.download(branches, Some(&mut fetch_options()))?;
            remote.update_tips(None, true, git2::AutotagOption::Unspecified, None)?;
        }

        Ok(())
    }

    ///
    /// If this is the first time this is called, it clones the repository into a directory inside data_dir as
    /// a bare repo, if there are files in the target repository it deletes them and retries.
    ///
    /// # Arguments:
    ///
    /// * `repo` - A struct that implements RemoteRepo containing the repo to clone.
    ///
    pub fn download_remote_repository(
        &self,
        repo: &impl RemoteRepo,
    ) -> Result<(), GitManagerCloneError> {
        let target_dir = repo.target_dir(self.data_dir);
        let result;
        {
            debug!(
                "Fetching repo, adquiring lock. target_dir is: {:?}",
                target_dir
            );
            let mutex_guard = self.mutex.lock().unwrap();

            result = self
                .builder()
                .fetch_options(fetch_options())
                .with_checkout(CheckoutBuilder::new())
                .bare(true)
                .clone(&repo.url(), &target_dir);

            match result {
                Ok(_) => {
                    // This is so we track every remote ref
                    debug!(
                        "Repo cloned, reconfiguring to fetch every ref. Target_dir is: {:?}",
                        target_dir
                    );

                    let mut config = Config::new().unwrap();
                    config.add_file(
                        target_dir.join("config").as_path(),
                        git2::ConfigLevel::Local,
                        false,
                    )?;
                    config.remove("remote.origin.fetch").unwrap();
                    config
                        .set_str("remote.origin.fetch", "+refs/*:refs/*")
                        .unwrap();
                    Ok(())
                }
                Err(err) => {
                    // If we find that there is already a repo and a valid one, we leave.
                    // otherwise, we delete it and retry.
                    if err.code() == git2::ErrorCode::Exists
                        && err.class() == git2::ErrorClass::Invalid
                    {
                        match git2::Repository::open(&target_dir) {
                            Err(_err) => {
                                warn!("Repo directory exists but is not a valid repo, will delete it and try to clone again.");
                                match fs::remove_dir_all(&target_dir) {
                                    Ok(()) => {
                                        drop(mutex_guard);
                                        return self.download_remote_repository(repo);
                                    }
                                    Err(err) => {
                                        return Err(GitManagerCloneError::CouldNotDeleteRepoDir(
                                            err.to_string(),
                                        ));
                                    }
                                }
                            }
                            Ok(_repo) => {
                                return Ok(());
                            }
                        }
                    }
                    Err(GitManagerCloneError::GitError(err.to_string()))
                }
            }
        }
    }

    // This methos returns a Git2 repoBuilder with creds and settings prepared
    // for the agent
    fn builder(&self) -> RepoBuilder {
        // Prepare clone options.

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options());

        builder
    }
}

// fetch options used in git2 operations
fn fetch_options() -> FetchOptions<'static> {
    let mut fetch_options = git2::FetchOptions::new();

    fetch_options.prune(git2::FetchPrune::On);
    fetch_options.download_tags(git2::AutotagOption::All);
    let mut callbacks = RemoteCallbacks::new();
    transfer_progress(&mut callbacks);
    auth_callbacks(&mut callbacks);
    update_tips(&mut callbacks);
    fetch_options.remote_callbacks(callbacks);

    fetch_options
}

fn transfer_progress(callbacks: &mut RemoteCallbacks) {
    callbacks.transfer_progress(|stats| {
        println!("STATS {:?}", stats.indexed_deltas());
        if stats.received_objects() == stats.total_objects() {
            print!(
                "Resolving deltas {}/{}\r",
                stats.indexed_deltas(),
                stats.total_deltas()
            );
        } else if stats.total_objects() > 0 {
            print!(
                "Received {}/{} objects ({}) in {} bytes\r",
                stats.received_objects(),
                stats.total_objects(),
                stats.indexed_objects(),
                stats.received_bytes()
            );
        }
        io::stdout().flush().unwrap();
        true
    });
}

// @TODO: Deal with auth
fn auth_callbacks(callbacks: &mut RemoteCallbacks) {
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key(
            username_from_url.unwrap(),
            None,
            Path::new(&format!("{}/.ssh/id_rsa", env::var("HOME").unwrap())),
            None,
        )
    });
}

fn update_tips(callbacks: &mut RemoteCallbacks) {
    callbacks.update_tips(|refname, a, b| {
        if a.is_zero() {
            println!("[new]     {:20} {}", b, refname);
        } else {
            println!("[updated] {:10}..{:10} {}", a, b, refname);
        }
        true
    });
}

/// This is a trait that references a remote git repository.
///
/// It exists so we can implement different git providers.
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
    #[allow(dead_code)]
    pub fn new(url: String) -> UrlRepo {
        UrlRepo { url }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
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
    GitManager::new(data_dir).unwrap()
});

/// Git Manager is a helper 'class' that will manage repositories
/// inside the DataDir specified. This function returns a configured
/// instance of it, based on configutation.
pub fn get_git_manager() -> &'static GitManager<'static> {
    &GIT_MANAGER
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::{self, git_commit_all, temp_data_dir};
    use crate::utils::initialize_logger;
    use rocket::tokio;
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;
    use tempfile::{tempdir, TempDir};
    use uuid::uuid;

    #[test]
    // @TODO: DRY this with the commit version
    // When a branch receives new commits, the clone done by the agent will reflect that.
    fn commits_same_branch_by_branch() {
        initialize_logger();

        let empty_repo = test_utils::git_empty_repo();
        let mut file = File::create(empty_repo.path().join("test-branch.txt")).expect("can create");
        file.write_all(b"Hello, world!").expect("can write");

        git_commit_all(&empty_repo);
        let repo = UrlRepo::new(format!("file://{}", &empty_repo.path().to_string_lossy()));
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();

        manager.download_remote_repository(&repo).unwrap();
        let uuid = uuid!("f37915a0-7195-11ed-a1eb-0242ac120010");

        manager.clone_local_by_branch(&repo, "main", uuid).unwrap();
        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(uuid.to_string())
                .join("test-branch.txt")
        )
        .exists());

        let mut file = File::create(empty_repo.path().join("test2.txt")).expect("can create");
        file.write_all(b"Hello, world!").expect("can write");
        git_commit_all(&empty_repo);
        let uuid = uuid!("f37915a0-7195-11ed-a1eb-0242ac120011");
        manager.clone_local_by_branch(&repo, "main", uuid).unwrap();
        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(uuid.to_string())
                .join("test2.txt")
        )
        .exists());
    }

    #[test]
    // @TODO: DRY this with the branch version
    // When a branch receives new commits, the clone done by the agent will reflect that (by commit)
    fn commits_same_branch_by_commit() {
        initialize_logger();
        let empty_repo = test_utils::git_empty_repo();
        let mut file = File::create(empty_repo.path().join("test.txt")).expect("can create");
        file.write_all(b"Hello, world!").expect("can write");
        git_commit_all(&empty_repo);
        let repo = UrlRepo::new(format!("file://{}", &empty_repo.path().to_string_lossy()));
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();
        manager.download_remote_repository(&repo).unwrap();
        let uuid = uuid!("f37915a0-7195-11ed-a1eb-0242ac120010");
        manager.clone_local_by_branch(&repo, "main", uuid).unwrap();
        let work_path = &data_dir.work_path().unwrap().join(uuid.to_string());

        assert!(&work_path.join("test.txt").exists());

        let mut file = File::create(empty_repo.path().join("test2.txt")).expect("can create");
        file.write_all(b"Hello, world!").expect("can write");
        let commit = git_commit_all(&empty_repo);
        let uuid = uuid!("f37915a0-7195-11ed-a1eb-0242ac120017");
        manager.fetch(&[] as &[&str], &repo).unwrap();

        manager.clone_local_by_commit(&repo, &commit, uuid).unwrap();

        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(uuid.to_string())
                .join("test2.txt")
        )
        .exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn concurrent_clones() {
        initialize_logger();

        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let mut handles = std::vec::Vec::new();
        for i in 0..5 {
            handles.push({
                let repo = repo.clone();
                tokio::spawn(async move {
                    println!("I am {}", i);
                    let manager = get_git_manager();
                    let result = manager.download_remote_repository(&repo);
                    println!("{:?}", result);
                    assert!(result.is_ok());
                })
            });
        }
        while let Some(handle) = handles.pop() {
            handle.await.unwrap();
        }
    }

    #[test]
    fn new() {
        let manager = GitManagerHandler::new();
        assert!(manager.git_manager().is_ok());
    }

    #[test]
    fn clone_with_valid_url() {
        let manager = GitManagerHandler::new();
        let manager = manager.git_manager().unwrap();
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let result = manager.download_remote_repository(&repo);
        assert!(result.is_ok());
    }

    #[test]
    fn clone_with_valid_url_twice() {
        initialize_logger();
        let manager = GitManagerHandler::new();
        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        let manager = manager.git_manager().unwrap();
        manager.download_remote_repository(&repo).unwrap();
        assert!(manager.download_remote_repository(&repo).is_ok());
    }

    #[test]
    fn clone_with_garbage_in_destination() {
        initialize_logger();
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();

        let target_dir = data_dir
            .repos_path()
            .unwrap()
            .join("github")
            .join("kittengrid")
            .join("deb-s3");

        fs::create_dir_all(&target_dir).unwrap();
        std::fs::File::create(target_dir.join("garbage")).expect("create failed");

        let repo = GitHubRepo {
            user: String::from("kittengrid"),
            repo: String::from("deb-s3"),
        };

        assert!(manager.download_remote_repository(&repo).is_ok());
    }

    #[test]
    fn clone_with_invalid_url() {
        let manager = GitManagerHandler::new();
        let repo = GitHubRepo {
            user: String::from("kittengrid()"),
            repo: String::from("I_DON_THINK_WE_WILL_EVER_HAVE_THIS_REPO"),
        };

        let manager = manager.git_manager().unwrap();
        assert!(matches!(
            manager.download_remote_repository(&repo).err().unwrap(),
            _git_manager_clone_error
        ));
    }

    #[test]
    fn clone_url_repo() {
        let repo = UrlRepo::new(test_repo("simple-repo"));

        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();

        let result = manager.download_remote_repository(&repo);
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
    fn clone_not_default_branch_repo() {
        let repo = UrlRepo::new(test_repo("simple-repo"));
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();

        manager.download_remote_repository(&repo).unwrap();
        manager
            .clone_local_by_branch(
                &repo,
                "other-branch",
                uuid!("f37915a0-7195-11ed-a1eb-0242ac120004"),
            )
            .unwrap();

        assert!(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join("f37915a0-7195-11ed-a1eb-0242ac120004")
                .join("test")
        )
        .exists());
    }

    #[test]
    fn clone_local_branch_url_repo() {
        crate::utils::initialize_logger();

        let first_uuid = "f37915a0-7195-11ed-a1eb-0242ac120002";
        let second_uuid = "f37915a0-7195-11ed-a1eb-0242ac120003";
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();

        let repo = UrlRepo::new(test_repo("simple-repo"));
        manager.download_remote_repository(&repo).unwrap();

        let result = manager.clone_local_by_branch(
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

        let pack_file = "pack-c66b181f6683ac658107b156d3b643ef27fb4c2b.pack";

        let first_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(first_uuid)
                .join(".git/objects/pack")
                .join(pack_file),
        ))
        .unwrap();
        manager
            .clone_local_by_branch(&repo, "main", uuid!("f37915a0-7195-11ed-a1eb-0242ac120003"))
            .unwrap();

        let second_metadata = fs::metadata(Path::new(
            &data_dir
                .work_path()
                .unwrap()
                .join(second_uuid)
                .join(".git/objects/pack")
                .join(pack_file),
        ))
        .unwrap();
        assert_eq!(first_metadata.ino(), second_metadata.ino());
    }

    #[test]
    fn clone_local_commit_url_repo() {
        let first_uuid = "f37915a0-7195-11ed-a1eb-0242ac120002";
        let second_uuid = "f37915a0-7195-11ed-a1eb-0242ac120003";
        let manager = GitManagerHandler::new();
        let data_dir = &manager.data_dir;
        let manager = manager.git_manager().unwrap();
        let repo = UrlRepo::new(test_repo("simple-repo"));
        manager.download_remote_repository(&repo).unwrap();
        manager.download_remote_repository(&repo).unwrap();
        let result = manager.clone_local_by_commit(
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

        let pack_file = "pack-c66b181f6683ac658107b156d3b643ef27fb4c2b.pack";

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
            .clone_local_by_commit(
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

    struct GitManagerHandler {
        pub temp_dir: TempDir,
        pub data_dir: DataDir,
    }

    impl GitManagerHandler {
        pub fn new() -> Self {
            let temp_dir = tempdir().unwrap();
            let mut data_dir = DataDir::new(temp_dir.path().to_path_buf());
            data_dir.init();
            Self { temp_dir, data_dir }
        }

        pub fn git_manager<'a>(&'a self) -> Result<GitManager<'a>, GitManagerInitError> {
            Ok(GitManager::new(&self.data_dir)?)
        }
    }

    fn test_repo(repo: &str) -> String {
        format!(
            "file://{}/resources/test/{}",
            env!("CARGO_MANIFEST_DIR"),
            repo
        )
    }
}
