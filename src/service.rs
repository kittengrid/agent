use super::persisted_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Child, Command};

use crate::config;

#[derive(Default)]
pub struct Service {
    name: String,
    child: Option<Child>,
    cmd: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    stdout: Option<PersistedBufReaderBroadcaster>,
    port: u16,
    health_check: Option<config::HealthCheck>,
}

impl From<config::ServiceConfig> for Service {
    fn from(config: config::ServiceConfig) -> Self {
        Self {
            name: config.name.clone(),
            port: config.port,
            env: config.env.unwrap_or_default(),
            args: config.args.unwrap_or_default(),
            cmd: config.cmd.unwrap_or(config.name),
            health_check: config.health_check,
            ..Default::default()
        }
    }
}

impl Service {
    pub fn stdout(&self) -> Option<PersistedBufReaderBroadcaster> {
        self.stdout.clone()
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn health_check(&self) -> Option<config::HealthCheck> {
        self.health_check.clone()
    }

    pub async fn spawn(&mut self) -> std::io::Result<()> {
        let mut cmd = Command::new(&self.cmd);
        cmd.args(&self.args);
        cmd.envs(&self.env);
        cmd.stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;

        let stdout = BufReader::new(child.stdout.take().expect("stdout is None"));
        self.child = Some(child);
        self.stdout = Some(PersistedBufReaderBroadcaster::new(stdout).await);

        Ok(())
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn from() {
        let mut config = config::ServiceConfig {
            name: "test".to_string(),
            port: 8080,
            cmd: Some("test".to_string()),
            args: Some(vec!["--port".to_string()]),
            env: Some(HashMap::new()),
            ..Default::default()
        };
        let service = Service::from(config.clone());
        assert_eq!(service.name, "test");
        assert_eq!(service.port, 8080);
        assert_eq!(service.cmd, "test");
        assert_eq!(service.args, vec!["--port".to_string()]);
        assert_eq!(service.env, HashMap::new());
        assert!(service.child.is_none());

        // Assert we use the name as default command
        config.cmd = None;
        let service = Service::from(config);
        assert_eq!(service.cmd, "test");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn spawn() {
        let config = config::ServiceConfig {
            name: "/bin/bash".to_string(),
            args: Some(vec!["-c".to_string(), "echo 1".to_string()]),
            ..Default::default()
        };
        let mut service = Service::from(config);
        let result = service.spawn().await;

        assert!(result.is_ok());
        let p = tokio::spawn({
            let mut receiver = service.stdout().unwrap().subscribe().await;
            async move { while let Some(_data) = receiver.recv().await {} }
        });

        let j = tokio::spawn({
            let mut receiver = service.stdout().unwrap().subscribe().await;
            async move { while let Some(_data) = receiver.recv().await {} }
        });

        p.await.unwrap();
        j.await.unwrap();
    }
}
