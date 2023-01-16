use crate::data_dir::{DataDir, DataDirError};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str;
use thiserror::Error;

/// Data structure to manage docker-compose execution
pub struct DockerCompose<'a> {
    data_dir: &'a DataDir,
    cwd: Option<String>,
    compose_file: Option<String>,
    env: HashMap<String, String>,
}

#[derive(Error, Debug)]
pub enum DockerComposeInitError {
    #[error("Cannot write docker-compose binary into state dir ({}).", .0)]
    BinaryNotWritable(Error),
    #[error("Directory not initialized")]
    DirectoryNotInitialized,
}

#[derive(Error, Debug)]
pub enum DockerComposeRunError {
    #[error("Cannot call docker compose without specifying a compose-file.")]
    NoDockerComposeFile,
    #[error("Could not execute the docker compose command ({}).", .0)]
    ExecutionError(Error),
    #[error("Exit status was not zero. Was ({})", .0.status.code().unwrap())]
    ErrorExitStatus(Output),
    #[error("Docker compose file not found or not readable ({}).", .0)]
    DockerComposeFileNotFound(String),
}

// Struct that stores the result of a docker-compose ps call
#[derive(Deserialize, Debug, Default)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerComposePsResult {
    pub command: String,
    pub exit_code: u16,
    pub health: String,
    #[serde(rename = "ID")]
    pub id: String,
    pub name: String,
    pub project: String,
    pub publishers: Option<Vec<DockerComposePsPublisher>>,
    pub service: String,
    pub state: String,
}

// Needed by previous struct definition (publishers)
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct DockerComposePsPublisher {
    pub protocol: String,
    pub published_port: u32,
    pub target_port: u32,
    pub url: String,
}

impl<'a> DockerCompose<'a> {
    /// Returns an DockerCompose instance.
    ///
    /// This method needs the state_dir so it can install (if not done already), the
    /// bundled docker-compose binary.
    ///
    /// # Arguments
    ///
    /// * `state_dir` - A DataDir to be used for installing the bundled docker-compose binary.
    ///
    /// # Examples
    ///
    /// ```
    /// use data_dir::DataDir;
    /// let data_dir = DataDir::new("/var/lib/kittengrid-agent");
    /// let docker_compose = DockerCompose::new(data_dir);
    /// ```
    ///
    pub fn new(data_dir: &'a DataDir) -> Result<Self, DockerComposeInitError> {
        let binary_path = match data_dir.bin_path() {
            Err(DataDirError::DirectoryNotInitialized) => {
                return Err(DockerComposeInitError::DirectoryNotInitialized)
            }
            Ok(bin_path) => bin_path.join("docker-compose"),
        };
        let binary_data = include_bytes!("docker-compose-linux-binary");

        // @TODO: Do not write it again if the file exists
        match std::fs::write(&binary_path, binary_data) {
            Err(why) => Err(DockerComposeInitError::BinaryNotWritable(why)),
            Ok(_) => {
                let mut perms = fs::metadata(&binary_path).unwrap().permissions();
                perms.set_mode(0o700); // Read/write for owner and read for others.

                fs::set_permissions(&binary_path, perms).unwrap();
                Ok(DockerCompose {
                    data_dir,
                    cwd: None,
                    compose_file: None,
                    env: HashMap::new(),
                })
            }
        }
    }

    fn binary_path(&self) -> PathBuf {
        self.data_dir
            .bin_path()
            .expect("Directory should be initialized if instance is created")
            .join("docker-compose")
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
    /// Sets up the project name, this will be used as a prefix for naming containers by docker-compose.
    ///
    /// # Arguments
    ///
    /// * `project_name` - A String with the name of the project.
    ///
    pub fn project_name(&mut self, project_name: String) -> &mut Self {
        self.env(String::from("COMPOSE_PROJECT_NAME"), project_name);
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

    ///
    /// Executes docker-compose start.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `start`, waits for it to finish and returns the result or an execution error.
    pub fn start(&self) -> Result<Output, DockerComposeRunError> {
        self.invoke("start", vec![])
    }

    ///
    /// Executes docker-compose stop.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `stop`, waits for it to finish and returns the result or an execution error.
    pub fn stop(&self) -> Result<Output, DockerComposeRunError> {
        self.invoke("stop", vec![])
    }

    ///
    /// Executes docker-compose rm.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `rm`, waits for it to finish and returns the result or an execution error.
    pub fn rm(&self, stop: bool) -> Result<Output, DockerComposeRunError> {
        let mut arguments = vec![String::from("-f")];
        if stop {
            arguments.push(String::from("-s"));
        }
        self.invoke("rm", arguments)
    }

    /// Executes docker-compose status.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `ps`, waits for it to finish and returns the result or an execution error.
    pub fn ps(&self) -> Result<Vec<DockerComposePsResult>, DockerComposeRunError> {
        let output = self.invoke("ps", vec![String::from("--format"), String::from("json")])?;
        let data = str::from_utf8(&output.stdout).unwrap();
        let ps: Vec<DockerComposePsResult> = serde_json::from_str(data).unwrap();
        Ok(ps)
    }

    /// Executes docker-compose down.
    ///
    /// This method executes a synchronous call to docker-compose passing the
    /// subcommand `ps`, waits for it to finish and returns the result or an execution error.
    ///
    /// # Arguments
    ///
    /// * `remove_orphans` - Remove containers for services not defined in the Compose file.
    /// * `rmi`            - Remove images used by services. "local" remove only images that don't have a custom tag ("local"|"all")
    /// * `volumes`        - Remove named volumes declared in the volumes section of the Compose file and anonymous volumes attached to containers.
    ///
    pub fn down(
        &self,
        remove_orphans: bool,
        rmi: Option<String>,
        volumes: bool,
    ) -> Result<Output, DockerComposeRunError> {
        let mut arguments = vec![];
        if remove_orphans {
            arguments.push(String::from("--remove-orphans"));
        }
        if let Some(rmi) = rmi {
            arguments.push(String::from("--rmi"));
            arguments.push(rmi);
        }
        if volumes {
            arguments.push(String::from("-v"));
        }
        self.invoke("down", arguments)
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

        let mut command = Command::new(self.binary_path());
        for (key, value) in &self.env {
            command.env(&*key, &*value);
        }
        if !Path::new(&*compose_file_path).exists() {
            return Err(DockerComposeRunError::DockerComposeFileNotFound(
                compose_file_path.to_string(),
            ));
        }

        command.env("COMPOSE_FILE", &*compose_file_path);
        if let Some(cwd) = &self.cwd {
            command.current_dir(&*cwd);
        }

        command.arg(subcommand).args(args.as_slice());
        match command.output() {
            Ok(output) => {
                if output.status.success() {
                    Ok(output)
                } else {
                    Err(DockerComposeRunError::ErrorExitStatus(output))
                }
            }
            Err(err) => Err(DockerComposeRunError::ExecutionError(err)),
        }
    }

    // For internal use, cleans up the network associated with this
    // docker compose.
    fn clean(&mut self) {
        self.down(true, Some(String::from("all")), true);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::data_dir::DataDir;
    use tempfile::{tempdir, TempDir};

    #[test]
    fn new() {
        let temp_dir = tempdir().unwrap();
        let data_dir = DataDir::new(temp_dir.path().to_path_buf());

        let docker_compose = DockerCompose::new(&data_dir);
        assert!(matches!(
            docker_compose.err().unwrap(),
            DockerComposeInitError::DirectoryNotInitialized
        ));

        let mut data_dir = DataDir::new(temp_dir.path().to_path_buf());
        data_dir.init().unwrap();

        let docker_compose = DockerCompose::new(&data_dir);
        assert!(docker_compose.is_ok());
        assert!(std::path::Path::new(&data_dir.path().join("bin").join("docker-compose")).exists());
    }

    #[test]
    fn invoke_with_incorrect_directory_and_a_valid_docker_compose_file_path() {
        let (_tempdir, data_dir) = data_dir();
        let mut compose = docker_compose(&data_dir, "simple-compose.yaml");
        compose.cwd(String::from(" I DO NOT EXIST"));
        assert!(matches!(
            compose.create().err().unwrap(),
            DockerComposeRunError::ExecutionError(_)
        ));
    }

    #[test]
    fn invoke_with_valid_directory_and_incorrect_docker_compose_file_path() {
        let (_tempdir, data_dir) = data_dir();
        let compose = docker_compose(&data_dir, "I do not exist");
        assert!(matches!(
            compose.create().err().unwrap(),
            DockerComposeRunError::DockerComposeFileNotFound(_)
        ));
    }

    #[test]
    fn invoke_with_incorrect_directory_and_a_valid_docker_compose_file_path_with_invalid_contents()
    {
        let (_tempdir, data_dir) = data_dir();
        let compose = docker_compose(&data_dir, "simple-compose-gone-wrong.yaml");
        assert!(matches!(
            compose.create().err().unwrap(),
            DockerComposeRunError::ErrorExitStatus(_)
        ));
    }

    #[test]
    fn ps() {
        let (_tempdir, data_dir) = data_dir();
        let compose = docker_compose(&data_dir, "simple-compose.yaml");
        assert!(compose.create().is_ok());
        assert!(compose.start().is_ok());

        let output = compose.ps().unwrap();
        let ps = output.first().unwrap();
        assert_eq!(ps.command, "docker-entrypoint.sh redis-server");
        assert_eq!(ps.exit_code, 0);
        assert_eq!(ps.health, "");
        assert!(compose.stop().is_ok());
        assert!(compose.rm(true).is_ok());
    }

    #[test]
    fn rm_deletes_containers_when_force_stop_is_set_to_true() {
        let (_tempdir, data_dir) = data_dir();
        let compose = docker_compose(&data_dir, "simple-compose.yaml");

        compose.create().unwrap();
        compose.start().unwrap();
        let output = compose.ps().unwrap();
        let ps = output.first().unwrap();
        assert_eq!(ps.command, "docker-entrypoint.sh redis-server");
        compose.rm(false).unwrap();
        let output = compose.ps().unwrap();
        let ps = output.first().unwrap();
        assert_eq!(ps.command, "docker-entrypoint.sh redis-server");
        compose.rm(true).unwrap();
        let output = compose.ps().unwrap();
        assert!(output.first().is_none());
    }

    // Delete Docker stuff
    impl<'a> Drop for DockerCompose<'a> {
        fn drop(&mut self) {
            self.clean();
        }
    }

    fn data_dir() -> (TempDir, DataDir) {
        let temp_dir = tempdir().unwrap();
        let mut data_dir = DataDir::new(temp_dir.path().to_path_buf());
        data_dir.init().unwrap();

        (temp_dir, data_dir)
    }

    // Helpers
    fn docker_compose<'a>(data_dir: &'a DataDir, fixture: &str) -> DockerCompose<'a> {
        let mut compose = DockerCompose::new(data_dir).unwrap();
        let project_name = String::from(
            data_dir
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap(),
        );
        compose
            .cwd(test_fixture(""))
            .project_name(project_name)
            .compose_file(test_fixture(fixture));

        compose
    }

    fn test_fixture(path: &str) -> String {
        let path_buf =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("resources/test/{}", path));
        path_buf.into_os_string().into_string().unwrap()
    }
}
