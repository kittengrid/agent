use crate::StateDir;

/// Data structure to manage docker-compose execution
pub struct DockerCompose {
    pub binary_path: String,
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
    /// let docker_copose = DockerCompose::new(state_dir);
    /// ```
    ///
    pub fn new(state_dir: &mut StateDir) {
        let docker_compose = state_dir.bin_path().join("docker-compose");
        std::fs::write(
            docker_compose,
            include_bytes!("docker-compose-linux-binary"),
        );
    }

    // pub fn execute_binary() {
    //     unsafe {
    //         if let Some(program) = COMPOSE_BINARY.clone() {
    //             let test = Command::new(program)
    //                 .output()
    //                 .expect("failed to execute process");
    //             println!("{}", str::from_utf8(&test.stdout).unwrap());
    //         }
    //     }
    // }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::StateDir;
    use tempfile::tempdir;

    #[test]
    fn new() {
        let directory = tempdir().unwrap();
        let mut state_dir = StateDir::new(directory.path().to_str().unwrap());
        let docker_compose = DockerCompose::new(&mut state_dir);

        assert!(
            std::path::Path::new(&directory.path().join("bin").join("docker-compose")).exists()
        );
    }
}
