use crate::data_dir::{DataDir, DataDirError};
use git2::build::RepoBuilder;
use sha2::{Digest, Sha256, Sha512};
use std::fs;
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

#[derive(Error, Debug)]
pub enum GitFetchError {
    #[error("Unable to extract directory name")]
    TargetDirectoryError,
    #[error("Repository url parse error ({})", .0)]
    UrlParseError(url::ParseError),
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

        let md = fs::metadata(repos_path.clone())?;
        let permissions = md.permissions();
        let readonly = permissions.readonly();
        if readonly {
            return Err(GitManagerInitError::DirectoryNotWritable);
        }

        Ok(Self { data_dir })
    }

    fn dir_for_repo(&self, repo: &str) -> Result<Option<PathBuf>, ParseError> {
        let url = Url::parse(repo)?;
        match url.path_segments() {
            None => Ok(None),
            Some(segments) => {
                let last_entry = match segments.clone().last() {
                    Some("") => {
                        let rev = segments.clone().rev();
                        let elements: Vec<&str> = rev.skip_while(|v| v.is_empty()).collect();
                        match elements.first() {
                            Some(&"") => None,
                            Some(str) => Some(*str),
                            _ => None,
                        }
                    }
                    Some(entry) => Some(entry),
                    _ => None,
                };
                match last_entry {
                    None => Ok(None),
                    Some(entry) => {
                        let dir_name = hash_string(&url) + "-" + &strip_dot_git(entry);
                        println!("{}", dir_name);
                        Ok(Some(self.data_dir.repos_path().unwrap().join(dir_name)))
                    }
                }
            }
        }
    }

    ///
    /// Fetches the repository and saves it into a directory inside data_dir as
    /// a bare repo, if the repository already exists, it downloads objects and refs (git fetch).
    ///
    /// # Arguments:
    ///
    /// * `repo` - The Url of the git repository
    ///
    pub fn fetch(&self, repo: &str) -> Result<(), GitFetchError> {
        let target_dir = match self.dir_for_repo(repo) {
            Ok(directory) => match directory {
                None => return Err(GitFetchError::TargetDirectoryError),
                Some(dir) => dir,
            },
            Err(err) => return Err(GitFetchError::UrlParseError(err)),
        };
        println!("{:?}", target_dir);

        // Clone the project.
        let mut builder = git2::build::RepoBuilder::new();

        //     builder
        //         .bare(true)
        //         .clone(repo, self.repos_dir.join(dir_for_repo(repo)))

        // };
        Ok(())
    }
}

fn strip_dot_git(entry: &str) -> String {
    if let Some(stripped) = entry.strip_suffix(".git") {
        return stripped.to_string();
    }
    entry.to_string()
}

fn hash_string(input: &Url) -> String {
    let mut hasher = Sha256::new();

    // write input message

    hasher.update(input.host_str().unwrap().to_owned() + input.path());

    // read hash digest and consume hasher
    let result = hasher.finalize();
    format!("{:x}", result)[0..7].to_string()
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
    fn fetch_with_invalid_url() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let manager = manager.unwrap();
        assert!(matches!(
            manager.fetch("this is not a valid url").err().unwrap(),
            GitFetchError::UrlParseError(_)
        ))
    }

    #[test]
    fn fetch_with_valid_invbalid_target_dir() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let manager = manager.unwrap();
        assert!(matches!(
            manager.fetch("git:///").err().unwrap(),
            GitFetchError::TargetDirectoryError
        ))
    }

    #[test]
    fn dir_for_repo() {
        let (_tempdir, data_dir) = temp_data_dir();
        let manager = manager_instance(&data_dir);
        let manager = manager.unwrap();
        assert!(manager
            .dir_for_repo("git://this/is/correct////")
            .unwrap()
            .unwrap()
            .ends_with("e3fb9f6-correct"));
        assert!(manager
            .dir_for_repo("git://this/is/correct")
            .unwrap()
            .unwrap()
            .ends_with("e3fb9f6-correct"));
        assert!(manager
            .dir_for_repo("git://this/is/correct.git")
            .unwrap()
            .unwrap()
            .ends_with("correct"));

        assert_eq!(
            manager
                .dir_for_repo("git://this/is/correct.git")
                .unwrap()
                .unwrap()
                .to_str()
                .unwrap(),
            "pepe"
        );
    }

    // Helpers
    fn temp_data_dir() -> (TempDir, DataDir) {
        let directory = tempdir().unwrap();
        let mut data_dir = DataDir::new(directory.path().to_path_buf());
        data_dir.init().unwrap();
        (directory, data_dir)
    }

    fn manager_instance<'a>(data_dir: &'a DataDir) -> Result<GitManager<'a>, GitManagerInitError> {
        GitManager::new(&data_dir)
    }
}
