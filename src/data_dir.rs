use crate::config::get_config;
use log::warn;
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::{fs, io};
use tempfile::tempdir;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DataDirInitError {
    #[error("Cannot write state directory")]
    DirectoryNotWritable,
    #[error("IO Error ({})", .0)]
    IOError(std::io::Error),
}

#[derive(Error, Debug, PartialEq)]
pub enum DataDirError {
    #[error("Directory not initialized")]
    DirectoryNotInitialized,
}

/// Data structure to manage the state directory
#[derive(Clone, Debug)]
pub struct DataDir {
    path: PathBuf,
    initialized: bool,
}

impl DataDir {
    /// Returns an DataDir struct with the path set.
    ///
    /// # Arguments
    ///
    /// * `path` - A string slice that holds the path of the directory.
    ///            It should be writable by user running the agent.
    ///
    /// # Examples
    ///
    /// ```
    /// use data_dir::DataDir;
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent".into());
    /// ```
    ///
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
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
    /// use data_dir::DataDir;
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent");
    /// data_dir.init();
    /// ```
    ///
    pub fn init(&mut self) -> Result<(), DataDirInitError> {
        return match build_directory_structure(&self.path) {
            Err(DataDirInitError::DirectoryNotWritable) => {
                warn!(
                    "Cannot write to destination dir ({}), using a temporary directory instead",
                    self.path.to_str().unwrap()
                );
                let temp_dir = tempdir().unwrap();

                self.path = PathBuf::from(temp_dir.path());
                build_directory_structure(&self.path)
            }
            Err(error) => Err(error),
            Ok(()) => {
                self.initialized = true;
                Ok(())
            }
        };
    }

    /// Returns the path of the state dir.
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Returns the bin directory of the state dir
    pub fn bin_path(&self) -> Result<std::path::PathBuf, DataDirError> {
        if !self.initialized {
            return Err(DataDirError::DirectoryNotInitialized);
        }

        Ok(self.path.join("bin"))
    }

    /// Returns the work directory of the state dir
    pub fn work_path(&self) -> Result<std::path::PathBuf, DataDirError> {
        if !self.initialized {
            return Err(DataDirError::DirectoryNotInitialized);
        }

        Ok(self.path.join("work"))
    }

    /// Returns the repos directory of the state dir
    pub fn repos_path(&self) -> Result<std::path::PathBuf, DataDirError> {
        if !self.initialized {
            return Err(DataDirError::DirectoryNotInitialized);
        }

        Ok(self.path.join("repos"))
    }
}

fn build_directory_structure(path: &PathBuf) -> Result<(), DataDirInitError> {
    let paths = vec!["bin", "repos", "work"];
    let mut temp_builder = fs::DirBuilder::new();
    let builder = temp_builder.recursive(true);

    for new_path in paths {
        if let Err(err) = builder.create(path.join(new_path)) {
            return match err.kind() {
                io::ErrorKind::PermissionDenied => Err(DataDirInitError::DirectoryNotWritable),
                _ => Err(DataDirInitError::IOError(err)),
            };
        };
    }
    Ok(())
}

static DATA_DIR: Lazy<DataDir> = Lazy::new(|| {
    let config = get_config();
    let mut data_dir = DataDir::new(config.work_directory.clone().into());
    data_dir.init().unwrap(); // panics if we cannot initialize work dir
    data_dir
});

pub fn get_data_dir() -> &'static DataDir {
    &DATA_DIR
}

#[cfg(test)]
mod test {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new() {
        let data_dir = DataDir::new(PathBuf::from("/tmp"));
        assert_eq!(data_dir.path(), PathBuf::from("/tmp"));
    }

    #[test]
    fn init() {
        // Normal creation
        let mut data_dir = DataDir::new(tempdir().unwrap().path().to_path_buf());
        assert!(data_dir.init().is_ok());

        // When we pass a readonly directory it creates a new temporary one
        let readonly_dir = tempdir().unwrap();
        let mut perms = fs::metadata(readonly_dir.path()).unwrap().permissions();
        perms.set_readonly(true);

        fs::set_permissions(readonly_dir.path(), perms).unwrap();
        let mut data_dir = DataDir::new(readonly_dir.path().to_path_buf());
        data_dir.init().unwrap();
        assert_ne!(data_dir.path, readonly_dir.path().to_path_buf());

        // When we call several times (directories already exist)
        let mut data_dir = DataDir::new(tempdir().unwrap().path().to_path_buf());
        assert!(data_dir.init().is_ok());
    }

    #[test]
    fn bin() {
        let dir = tempdir().unwrap();
        let mut data_dir = DataDir::new(dir.path().to_path_buf());
        data_dir.init().unwrap();
        assert_eq!(
            data_dir.bin_path().unwrap().to_str().unwrap(),
            dir.path().join("bin").to_str().unwrap()
        );
    }
}
