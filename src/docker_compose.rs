use crate::state_dir::{StateDir, StateDirError};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::str;
use thiserror::Error;

/// Data structure to manage docker-compose execution
pub struct DockerCompose {
    pub binary_path: PathBuf,
    pub cwd: Option<String>,
    pub compose_file: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Error, Debug)]
pub enum DockerComposeInitError {
    #[error("Cannot write docker-compose binary into state dir ({}).", .0)]
    BinaryNotWritable(std::io::Error),
    #[error("Directory not initialized")]
    DirectoryNotInitialized,
}

#[derive(Error, Debug)]
pub enum DockerComposeRunError {
    #[error("Cannot call docker compose without specifying a compose-file")]
    NoDockerComposeFile,
}

impl DockerCompose {
    /// Returns an DockerCompose instance.
    ///
    /// This method needs the state_dir so it can install (if not done already), the
    /// bundled docker-compose binary.
    ///
    /// # Arguments
    ///
    /// * `state_dir` - A StateDir to be used for installing the bundled docker-compose binary.
    ///
    /// # Examples
    ///
    /// ```
    /// use state_dir::StateDir;
    /// let state_dir = StateDir::new("/var/lib/kittengrid-agent");
    /// let docker_compose = DockerCompose::new(state_dir);
    /// ```
    ///
    pub fn new(state_dir: &StateDir) -> Result<Self, DockerComposeInitError> {
        let binary_path = match state_dir.bin_path() {
            Err(StateDirError::DirectoryNotInitialized) => {
                return Err(DockerComposeInitError::DirectoryNotInitialized)
            }
            Ok(bin_path) => bin_path.join("docker-compose"),
        };
        let binary_data = include_bytes!("docker-compose-linux-binary");

        // @TODO: Do not write it again if the file exists
        match std::fs::write(binary_path.clone(), binary_data) {
            Err(why) => Err(DockerComposeInitError::BinaryNotWritable(why)),
            Ok(_) => {
                let mut perms = fs::metadata(binary_path.clone()).unwrap().permissions();
                perms.set_mode(0o700); // Read/write for owner and read for others.

                fs::set_permissions(binary_path.clone(), perms).unwrap();
                Ok(DockerCompose {
                    binary_path,
                    cwd: None,
                    compose_file: None,
                    env: HashMap::new(),
                })
            }
        }
    }

    ///
    /// Sets the working directory for the docker-compose process.
    ///
    /// # Arguments
    ///
    /// * `dir` - A String with the directory to set working directory of docker-compose.
    ///
    pub fn cwd(&mut self, dir: String) -> &mut Self {
        self.cwd = Some(dir);
        self
    }

    ///
    /// Sets the compose-file path for the docker-compose process.
    ///
    /// # Arguments
    ///
    /// * `path` - A String containing the path of the compose file.
    ///
    pub fn compose_file(&mut self, path: String) -> &mut Self {
        self.compose_file = Some(path);
        self
    }

    ///
    /// Adds a new environment variable for the docker-compose execution.
    ///
    /// # Arguments
    ///
    /// * `key`
    /// * `value`
    ///
    pub fn env(&mut self, key: String, value: String) -> &mut Self {
        self.env.insert(key, value);
        self
    }

    ///
    /// Executes docker-compose create.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `create`, waits for it to finish and returns the result or an execution error.
    pub fn create(&self) -> Result<Output, DockerComposeRunError> {
        self.invoke("create", vec![])
    }

    /// Executes docker-compose up.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `start`, waits for it to finish and returns the result or an execution error.
    pub fn start(&self) -> Result<Output, DockerComposeRunError> {
        self.invoke("start", vec![])
    }

    /// Executes docker-compose status.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `ps`, waits for it to finish and returns the result or an execution error.
    pub fn ps(&self) -> Result<Output, DockerComposeRunError> {
        self.invoke("ps", vec![String::from("--format"), String::from("json")])
    }

    /// Invokes a docker-compose command.
    ///
    /// # Arguments
    ///
    /// * `subcommand` - The subcommand to be invoked.
    /// * `args`       - An array containing subcommand arguments
    ///
    fn invoke(&self, subcommand: &str, args: Vec<String>) -> Result<Output, DockerComposeRunError> {
        let compose_file_path = match &self.compose_file {
            Some(path) => path,
            None => return Err(DockerComposeRunError::NoDockerComposeFile),
        };

        let mut command = Command::new(self.binary_path.clone());
        for (key, value) in &self.env {
            command.env(&*key, &*value);
        }
        command.env("COMPOSE_FILE", &*compose_file_path);
        if let Some(cwd) = &self.cwd {
            command.current_dir(&*cwd);
        }
        command.arg(subcommand).args(args.as_slice());
        println!("{:?}", command);
        Ok(command.output().expect("Failed to execute docker-compose"))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::state_dir::StateDir;
    use tempfile::tempdir;

    #[test]
    fn new() {
        let directory = tempdir().unwrap();
        let mut state_dir = StateDir::new(directory.path().to_str().unwrap());

        let mut docker_compose = DockerCompose::new(&state_dir);
        assert!(matches!(
            docker_compose.err().unwrap(),
            DockerComposeInitError::DirectoryNotInitialized
        ));

        state_dir.init().unwrap();
        docker_compose = DockerCompose::new(&state_dir);
        assert!(docker_compose.is_ok());
        assert!(
            std::path::Path::new(&directory.path().join("bin").join("docker-compose")).exists()
        );
    }

    fn docker_compose(directory: &str) -> DockerCompose {
        let mut state_dir = StateDir::new(directory);

        state_dir.init().unwrap();
        DockerCompose::new(&state_dir).unwrap()
    }

    fn test_fixture(path: &str) -> String {
        let path_buf =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("resources/test/{}", path));
        path_buf.into_os_string().into_string().unwrap()
    }

    #[test]
    fn create() {
        let tempdir = tempdir().unwrap();
        let mut state_dir = StateDir::new(tempdir.path().to_str().unwrap());
        state_dir.init().unwrap();

        let mut compose = DockerCompose::new(&state_dir).unwrap();
        compose
            .cwd(test_fixture(""))
            .compose_file(test_fixture("simple-compose.yaml"));

        let mut output = compose.create();
        println!("{}", str::from_utf8(&output.unwrap().stdout).unwrap());
        println!("-=--");
        output = compose.ps();
        println!("{}", str::from_utf8(&output.unwrap().stdout).unwrap());
    }
}
