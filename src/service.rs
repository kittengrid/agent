use super::persisted_buf_reader_broadcaster::{BufferReceiver, PersistedBufReaderBroadcaster};
use log::{debug, info};
use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config;

#[derive(Default, Clone, Debug)]
pub struct ServiceDescription {
    name: String,
    cmd: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    port: u16,
    health_check: Option<config::HealthCheck>,
}

impl From<config::ServiceConfig> for ServiceDescription {
    fn from(config: config::ServiceConfig) -> Self {
        Self {
            name: config.name.clone(),
            port: config.port,
            env: config.env.unwrap_or_default(),
            args: config.args.unwrap_or_default(),
            cmd: config.cmd.unwrap_or(config.name),
            health_check: config.health_check,
        }
    }
}
// This struct is used to hold service static configuration.
// This is here mainly because the Service can't implement clone
// and for simple usage like, registering with an api and stuff
// is handy to have a cloneable struct.
impl ServiceDescription {
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn health_check(&self) -> Option<config::HealthCheck> {
        self.health_check.clone()
    }
}

#[derive(Default, Debug)]
pub struct Service {
    description: ServiceDescription,
    child: Option<Child>,
    stdout: PersistedBufReaderBroadcaster,
    stderr: PersistedBufReaderBroadcaster,
}

impl From<config::ServiceConfig> for Service {
    fn from(config: config::ServiceConfig) -> Self {
        Self {
            description: config.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ServiceStream {
    Stdout,
    Stderr,
}

impl std::fmt::Display for ServiceStream {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ServiceStream::Stdout => write!(f, "stdout"),
            ServiceStream::Stderr => write!(f, "stderr"),
        }
    }
}

impl Service {
    pub async fn subscribe_to_stream(&self, stream: ServiceStream) -> BufferReceiver {
        match stream {
            ServiceStream::Stdout => self.stdout.subscribe().await,
            ServiceStream::Stderr => self.stderr.subscribe().await,
        }
    }

    pub async fn unsubscribe_from_stream(
        &mut self,
        receiver: BufferReceiver,
        stream: ServiceStream,
    ) {
        match stream {
            ServiceStream::Stdout => self.stdout.unsubscribe(receiver).await,
            ServiceStream::Stderr => self.stderr.unsubscribe(receiver).await,
        }
    }

    /// Returns an stdout broadcaster that will broadcast the stdout of the service
    /// to all the subscribers.
    /// Note that you can subscribe to the stdout even before the service is spawned.
    pub fn stdout(&self) -> PersistedBufReaderBroadcaster {
        self.stdout.clone()
    }

    /// Returns an stderr broadcaster that will broadcast the stderr of the service
    /// to all the subscribers.
    /// Note that you can subscribe to the stdout even before the service is spawned.
    pub fn stderr(&self) -> PersistedBufReaderBroadcaster {
        self.stderr.clone()
    }

    /// Returns the service description.
    pub fn description(&self) -> &ServiceDescription {
        &self.description
    }

    /// Syntax sugar for getting the name of the service.
    pub fn name(&self) -> String {
        self.description.name.clone()
    }

    /// Syntax sugar for getting the port of the service.
    pub fn port(&self) -> u16 {
        self.description.port
    }

    /// Syntax sugar for getting the health check of the service.
    pub fn health_check(&self) -> Option<config::HealthCheck> {
        self.description.health_check.clone()
    }

    /// Stops the service
    /// It will stop the service sending a TERM signal, note that stdout/stderr channels will be kept open.
    pub async fn stop(&mut self) -> std::io::Result<()> {
        match self.child.take() {
            Some(mut child) => {
                info!(
                    "Sending TERM Signal to the service '{}'.",
                    self.description.name
                );
                child.kill()?;
                child.wait()?;
            }
            None => {
                info!("Service {} was not running", self.description.name);
            }
        }
        Ok(())
    }

    /// Starts the service
    /// It will spawn the service and start broadcasting the stdout and stderr to the subscribers.
    pub async fn start(&mut self) -> std::io::Result<()> {
        debug!("Starting service '{}'", self.description.name);
        let mut cmd = Command::new(&self.description.cmd);

        cmd.args(&self.description.args)
            .envs(&self.description.env)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout = BufReader::new(child.stdout.take().expect("stdout is None"));
        self.stdout.watch(stdout).await;

        let stderr = BufReader::new(child.stderr.take().expect("stderr is None"));
        self.stderr.watch(stderr).await;
        self.child = Some(child);
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct Services {
    services: Mutex<HashMap<String, Arc<Mutex<Service>>>>,
}

impl Services {
    pub fn new() -> Self {
        Self {
            services: Mutex::new(HashMap::new()),
        }
    }

    /// Adds a service to the services list.
    pub async fn insert(&self, service: Service) {
        debug!("Adding service '{}' to services.", service.description.name);
        self.services.lock().await.insert(
            service.description.name.clone(),
            Arc::new(Mutex::new(service)),
        );
    }

    /// Returns a service by its name.
    pub async fn fetch(&self, service_name: &str) -> Option<Arc<Mutex<Service>>> {
        self.services.lock().await.get(service_name).cloned()
    }

    /// Returns the description of a service by its name.
    pub async fn description(&self, service_name: &str) -> Option<ServiceDescription> {
        match self.services.lock().await.get(service_name) {
            Some(service) => {
                let service = service.lock().await;
                Some(service.description().clone())
            }
            None => None,
        }
    }

    /// Stops a service by its name.
    pub async fn stop_service(&self, service_name: &str) -> std::io::Result<()> {
        let service = self.services.lock().await.get(service_name).cloned();

        if service.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Service {} not found", service_name),
            ));
        }

        let service = service.unwrap();
        let mut service = service.lock().await;
        service.stop().await
    }

    /// Starts a service by its name.
    pub async fn start_service(&self, service_name: &str) -> std::io::Result<()> {
        let service = self.services.lock().await.get(service_name).cloned();

        if service.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Service {} not found", service_name),
            ));
        }

        let service = service.unwrap();
        let mut service = service.lock().await;
        service.start().await
    }

    /// Returns a stream reader for a service if found.
    pub async fn subscribe_to_stream(
        &self,
        service_name: &str,
        stream: ServiceStream,
    ) -> Option<BufferReceiver> {
        debug!("Subscribing to stdout for service {}", service_name);
        let stream = match self.services.lock().await.get(service_name).cloned() {
            Some(service) => match stream {
                ServiceStream::Stdout => service.lock().await.stdout(),
                ServiceStream::Stderr => service.lock().await.stderr(),
            },
            None => return None,
        };

        Some(stream.subscribe().await)
    }

    /// Returns an array of every service description.
    pub async fn descriptions(&self) -> Vec<ServiceDescription> {
        let mut descriptions: Vec<ServiceDescription> = Vec::new();
        for service in self.services.lock().await.values() {
            descriptions.push(service.lock().await.description.clone());
        }
        descriptions
    }

    /// Stops every service.
    pub async fn stop(&self) -> std::io::Result<()> {
        debug!("Stopping all services");
        for taken_service in self.services.lock().await.values() {
            debug!("Stopping service '{}'", taken_service.lock().await.name());
            taken_service.lock().await.stop().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::initialize_tests;

    use bytes::Bytes;
    #[test]
    fn from() {
        initialize_tests();
        let mut config = config::ServiceConfig {
            name: "test".to_string(),
            port: 8080,
            cmd: Some("test".to_string()),
            args: Some(vec!["--port".to_string()]),
            env: Some(HashMap::new()),
            ..Default::default()
        };
        let service = Service::from(config.clone());
        assert_eq!(service.description.name, "test");
        assert_eq!(service.description.port, 8080);
        assert_eq!(service.description.cmd, "test");
        assert_eq!(service.description.args, vec!["--port".to_string()]);
        assert_eq!(service.description.env, HashMap::new());
        assert!(service.child.is_none());

        // Assert we use the name as default command
        config.cmd = None;
        let service = Service::from(config);
        assert_eq!(service.description.cmd, "test");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn spawn() {
        initialize_tests();
        let config = config::ServiceConfig {
            name: "/bin/bash".to_string(),
            args: Some(vec!["-c".to_string(), "echo 1; echo 2;".to_string()]),
            ..Default::default()
        };
        let mut service = Service::from(config);
        let result = service.start().await;

        assert!(result.is_ok());
        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;
        let data = receiver.recv().await;
        assert!(data.is_some());
        assert_eq!(data.unwrap(), Bytes::from("1\n"));
        let data = receiver.recv().await;
        assert!(data.is_some());
        assert_eq!(data.unwrap(), Bytes::from("2\n"));

        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;
        let data = receiver.recv().await;
        assert_eq!(data.unwrap(), Bytes::from("1\n2\n"));

        service.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn spawn_log() {
        initialize_tests();
        let mut service = crate::test_utils::log_generator_service();
        let result = service.start().await;

        assert!(result.is_ok());
        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;
        let data = receiver.recv().await;
        assert!(data.is_some());
        println!("{:?}", data);

        service
            .unsubscribe_from_stream(receiver, ServiceStream::Stdout)
            .await;
        service.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn lifecycle() {
        initialize_tests();
        let mut service = crate::test_utils::log_generator_service();
        let result = service.start().await;
        assert!(result.is_ok());
        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;
        let data = receiver.recv().await;
        assert!(data.is_some());
        let data = receiver.recv().await;
        assert!(data.is_some());
        service.stop().await.unwrap();
        let result = service.start().await;
        assert!(result.is_ok());
        let data = receiver.recv().await;
        assert!(data.is_some());

        service
            .unsubscribe_from_stream(receiver, ServiceStream::Stdout)
            .await;
        service.stop().await.unwrap();
    }
}
