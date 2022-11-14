use log::warn;
use std::{fs, io};
use tempfile::tempdir;

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum StateDirError {
    #[error("Cannot write state directory")]
    DirectoryNotWritable,
    #[error("IO Error")]
    IOError,
}

/// Data structure to manage the state directory
pub struct StateDir {
    pub path: String,
    initialized: bool,
}

impl StateDir {
    /// Returns an StateDir struct with the path set.
    ///
    /// # Arguments
    ///
    /// * `path` - A string slice that holds the path of the directory.
    ///            It should be writable by user running the agent.
    ///
    /// # Examples
    ///
    /// ```
    /// use state_dir::StateDir;
    /// let state_dir = StateDir::new("/var/lib/kittengrid-agent");
    /// ```
    ///
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            initialized: false,
        }
    }

    /// Initializes directory structure, this method is idempotent and non destructive.
    ///
    /// If it  encounters an issue when trying to write to the destination directory
    /// a new temp dir is created.
    ///
    /// # Examples
    ///
    /// ```
    /// use state_dir::StateDir;
    /// let state_dir = StateDir::new("/var/lib/kittengrid-agent");
    /// state_dir.init();
    /// ```
    ///
    pub fn init(&mut self) -> Result<(), StateDirError> {
        return match build_directory_structure(self.path.as_str()) {
            Err(StateDirError::DirectoryNotWritable) => {
                warn!(
                    "Cannot write to destination dir ({}), using a temporary directory instead",
                    self.path
                );
                let temp_dir = tempdir().unwrap();

                self.path = temp_dir.path().to_str().unwrap().to_string();
                build_directory_structure(self.path.as_str())
            }
            Err(StateDirError::IOError) => Err(StateDirError::IOError),
            Ok(()) => {
                self.initialized = true;
                Ok(())
            }
        };
    }

    /// Returns the bin directory of the state dir
    pub fn bin_path(&mut self) -> std::path::PathBuf {
        if !self.initialized {
            self.init();
        }
        std::path::Path::new(&self.path).join("bin")
    }
}

fn build_directory_structure(path: &str) -> Result<(), StateDirError> {
    let paths = vec!["bin", "repos"];
    let mut temp_builder = fs::DirBuilder::new();
    let builder = temp_builder.recursive(true);

    for new_path in paths {
        if let Err(err) = builder.create(std::path::Path::new(path).join(new_path)) {
            return match err.kind() {
                io::ErrorKind::PermissionDenied => Err(StateDirError::DirectoryNotWritable),
                _ => Err(StateDirError::IOError),
            };
        };
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new() {
        let state_dir = StateDir::new("/tmp");
        assert_eq!(state_dir.path, "/tmp");
    }

    #[test]
    fn init() {
        // Normal creation
        let mut state_dir = StateDir::new(tempdir().unwrap().path().to_str().unwrap());
        assert_eq!(state_dir.init(), Ok(()));

        // When we pass a readonly directory it creates a new temporary one
        let readonly_dir = tempdir().unwrap();
        let mut perms = fs::metadata(readonly_dir.path()).unwrap().permissions();
        perms.set_readonly(true);

        fs::set_permissions(readonly_dir.path(), perms).unwrap();
        let mut state_dir = StateDir::new(readonly_dir.path().to_str().unwrap());
        state_dir.init();
        assert_ne!(state_dir.path, readonly_dir.path().to_str().unwrap());

        // When we call several times (directories already exist)
        let mut state_dir = StateDir::new(tempdir().unwrap().path().to_str().unwrap());
        assert_eq!(state_dir.init(), Ok(()));
        assert_eq!(state_dir.init(), Ok(()));
    }

    #[test]
    fn bin() {
        let dir = tempdir().unwrap();
        let mut state_dir = StateDir::new(dir.path().to_str().unwrap());
        state_dir.init();
        assert_eq!(
            state_dir.bin_path().to_str().unwrap(),
            dir.path().join("bin").to_str().unwrap()
        );
    }
}
