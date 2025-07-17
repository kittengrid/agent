use super::persisted_buf_reader_broadcaster::{BufferReceiver, PersistedBufReaderBroadcaster};
use crate::kittengrid_api::KittengridApi;
use crate::process_controller::ProcessController;
use log::{debug, error, info};
use serde::ser::SerializeStruct;
use std::future::Future;
use std::pin::Pin;

use serde::Serialize;
use serde_json::json;
use std::{collections::HashMap, process::ExitStatus};

use std::io::BufReader;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config;

#[derive(Default, Clone, Debug, Serialize)]
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

#[derive(Debug, Serialize, Clone, Copy)]
pub enum ServiceStatus {
    Running,
    Stopped,
}

impl Default for ServiceStatus {
    fn default() -> Self {
        ServiceStatus::Stopped
    }
}

#[derive(Default, Debug)]
pub struct Service {
    id: uuid::Uuid,
    description: ServiceDescription,
    process_controller: Option<ProcessController>,
    stdout: PersistedBufReaderBroadcaster,
    stderr: PersistedBufReaderBroadcaster,
    status: ServiceStatus,
}

impl Serialize for Service {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let mut service = serializer.serialize_struct("Service", 2)?;
        service.serialize_field("description", self.description())?;
        service.serialize_field("status", &self.status)?;
        service.end()
    }
}

impl From<config::ServiceConfig> for Service {
    fn from(config: config::ServiceConfig) -> Self {
        Self {
            description: config.into(),
            id: uuid::Uuid::new_v4(),
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

    pub fn show_output(&mut self) {
        self.stdout
            .set_output_mode(crate::persisted_buf_reader_broadcaster::OutputMode::Stdout);
        self.stderr
            .set_output_mode(crate::persisted_buf_reader_broadcaster::OutputMode::Stderr);
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

    pub fn id(&self) -> uuid::Uuid {
        self.id
    }

    /// Syntax sugar for getting the health check of the service.
    pub fn health_check(&self) -> Option<config::HealthCheck> {
        self.description.health_check.clone()
    }

    /// Stops the service
    /// It will stop the service sending a TERM signal, note that stdout/stderr channels will be kept open.
    pub async fn stop(&mut self) -> std::io::Result<()> {
        match self.process_controller.take() {
            Some(mut process_controller) => {
                info!(
                    "Sending TERM Signal to the service '{}'.",
                    self.description.name
                );
                match process_controller.stop().await {
                    Ok(()) => {}
                    Err(e) => {
                        error!(
                            "Error stopping service '{}': {:?}",
                            self.description.name, e
                        );
                    }
                };
            }
            None => {
                info!("Service {} was not running", self.description.name);
            }
        }
        Ok(())
    }

    /// Starts the service
    /// It will spawn the service and start broadcasting the stdout and stderr to the subscribers.
    pub async fn start(
        &mut self,
        kittengrid_api: Arc<Option<KittengridApi>>,
    ) -> std::io::Result<()> {
        debug!("Starting service '{}'", self.description.name);
        if let Some(kittengrid_api) = &*kittengrid_api {
            if let Err(e) = kittengrid_api
                .services_update_status(
                    self.id,
                    Some(crate::kittengrid_api::ServiceStatus::Created),
                    None,
                    None,
                )
                .await
            {
                error!("Error updating service status: {:?}", e);
            }
        }

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
        self.status = ServiceStatus::Running;

        if let Some(kittengrid_api) = &*kittengrid_api {
            if let Err(e) = kittengrid_api
                .services_update_status(
                    self.id,
                    Some(crate::kittengrid_api::ServiceStatus::Running),
                    None,
                    None,
                )
                .await
            {
                error!("Error updating service status: {:?}", e);
            }
        }

        let on_stop_callback = Arc::new(Self::create_on_exit_callback(
            self.description.name.clone(),
            self.id,
            Arc::clone(&kittengrid_api),
        ));

        let health_check = self.health_check().map(|health_check| {
            crate::process_controller::HealthCheck::from_config(health_check, self.port())
        });

        let on_health_status_change_callback = Arc::new(Self::create_health_status_callback(
            self.description.name.clone(),
            self.id,
            Arc::clone(&kittengrid_api),
        ));

        let process_controller = ProcessController::new(
            child,
            on_stop_callback,
            health_check,
            Some(on_health_status_change_callback),
        )
        .await;
        self.process_controller = Some(process_controller);

        Ok(())
    }

    // Returns the callback that will be called when the service stops.
    // It simply makes a call to kittengrid api to update the service status.
    fn create_on_exit_callback(
        service_name: String,
        service_id: uuid::Uuid,
        kittengrid_api: Arc<Option<KittengridApi>>,
    ) -> impl Fn(ExitStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync {
        move |status: ExitStatus| {
            let description = service_name.clone();
            let id = service_id;
            let service_status = crate::kittengrid_api::ServiceStatus::Exited;
            let kittengrid_api = Arc::clone(&kittengrid_api);
            let exit_status = status.code();

            Box::pin(async move {
                if let Some(kittengrid_api) = &*kittengrid_api {
                    match kittengrid_api
                        .services_update_status(id, Some(service_status), None, exit_status)
                        .await
                    {
                        Ok(()) => {
                            info!("Service '{}' status updated to Stopped", description)
                        }
                        Err(e) => error!(
                            "Error updating service '{}' status to Stopped: {:?}",
                            description, e
                        ),
                    };
                };
            })
        }
    }

    // Returns the callback that will be called when the service health status
    // changes.
    // It simply makes a call to kittengrid api to update the service status.
    fn create_health_status_callback(
        service_name: String,
        service_id: uuid::Uuid,
        kittengrid_api: Arc<Option<KittengridApi>>,
    ) -> impl Fn(crate::HealthStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync
    {
        move |status: crate::HealthStatus| {
            let description = service_name.clone();
            let id = service_id;
            let kittengrid_api = Arc::clone(&kittengrid_api);

            Box::pin(async move {
                if let Some(kittengrid_api) = &*kittengrid_api {
                    match kittengrid_api
                        .services_update_status(id, None, Some(status), None)
                        .await
                    {
                        Ok(()) => {
                            info!("Service '{}' status updated to Stopped", description)
                        }
                        Err(e) => error!(
                            "Error updating service '{}' status to Stopped: {:?}",
                            description, e
                        ),
                    };
                };
            })
        }
    }
}

#[derive(Default, Debug)]
pub struct Services {
    services: Mutex<HashMap<uuid::Uuid, Arc<Mutex<Service>>>>,
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
        self.services
            .lock()
            .await
            .insert(service.id, Arc::new(Mutex::new(service)));
    }

    /// Returns a service by its name.
    pub async fn fetch(&self, id: uuid::Uuid) -> Option<Arc<Mutex<Service>>> {
        self.services.lock().await.get(&id).cloned()
    }

    /// Returns the description of a service by its name.
    pub async fn description(&self, id: uuid::Uuid) -> Option<ServiceDescription> {
        match self.services.lock().await.get(&id) {
            Some(service) => {
                let service = service.lock().await;
                Some(service.description().clone())
            }
            None => None,
        }
    }

    /// Stops a service by its id.
    pub async fn stop_service(&self, id: uuid::Uuid) -> std::io::Result<()> {
        let service = self.services.lock().await.get(&id).cloned();

        if service.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Service {} not found", id),
            ));
        }

        let service = service.unwrap();
        let mut service = service.lock().await;
        service.stop().await
    }

    pub async fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct InnerService {
            id: uuid::Uuid,
            description: ServiceDescription,
            status: ServiceStatus,
        }

        #[derive(Serialize)]
        struct ServicesSerializer {
            services: Vec<InnerService>,
        }
        let mut services: ServicesSerializer = ServicesSerializer {
            services: Vec::new(),
        };

        for service in self.services.lock().await.values() {
            let service = service.lock().await;
            let inner = InnerService {
                id: service.id(),
                description: service.description().clone(),
                status: service.status,
            };

            services.services.push(inner);
        }

        json!(services)
    }

    /// Starts a service by its id.
    pub async fn start_service(
        &self,
        id: uuid::Uuid,
        kittengrid_api: Arc<Option<KittengridApi>>,
    ) -> std::io::Result<()> {
        let service = self.services.lock().await.get(&id).cloned();

        if service.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Service {} not found", id),
            ));
        }

        let service = service.unwrap();
        let mut service = service.lock().await;
        service.start(kittengrid_api).await
    }

    /// Returns a stream reader for a service if found.
    pub async fn subscribe_to_stream(
        &self,
        id: uuid::Uuid,
        stream: ServiceStream,
    ) -> Option<BufferReceiver> {
        debug!("Subscribing to stdout for service {}", id);
        let stream = match self.services.lock().await.get(&id).cloned() {
            Some(service) => match stream {
                ServiceStream::Stdout => service.lock().await.stdout(),
                ServiceStream::Stderr => service.lock().await.stderr(),
            },
            None => return None,
        };

        Some(stream.subscribe().await)
    }

    /// Returns a stream reader for a service if found.
    pub async fn unsubscribe_from_stream(
        &self,
        id: uuid::Uuid,
        stream: ServiceStream,
        receiver: BufferReceiver,
    ) -> Result<(), std::io::Error> {
        debug!("Subscribing to stdout for service {}", id);
        let stream = match self.services.lock().await.get(&id).cloned() {
            Some(service) => match stream {
                ServiceStream::Stdout => service.lock().await.stdout(),
                ServiceStream::Stderr => service.lock().await.stderr(),
            },
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Service not found",
                ))
            }
        };

        stream.unsubscribe(receiver).await;
        Ok(())
    }

    /// Returns an array of every service description.
    pub async fn descriptions(&self) -> HashMap<uuid::Uuid, ServiceDescription> {
        let mut descriptions: HashMap<uuid::Uuid, ServiceDescription> = HashMap::new();
        for (id, service) in self.services.lock().await.iter() {
            descriptions.insert(*id, service.lock().await.description.clone());
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
        assert!(service.process_controller.is_none());

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
        let result = service.start(Arc::new(None)).await;

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
        let result = service.start(Arc::new(None)).await;

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
        let result = service.start(Arc::new(None)).await;
        assert!(result.is_ok());
        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;
        let data = receiver.recv().await;
        assert!(data.is_some());
        let data = receiver.recv().await;
        assert!(data.is_some());
        service.stop().await.unwrap();
        let result = service.start(Arc::new(None)).await;
        assert!(result.is_ok());
        let data = receiver.recv().await;
        assert!(data.is_some());

        service
            .unsubscribe_from_stream(receiver, ServiceStream::Stdout)
            .await;
        service.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_spawn_inherits_env_vars() {
        initialize_tests();
        let config = crate::config::ServiceConfig {
            name: "/usr/bin/env".to_string(),
            ..Default::default()
        };
        let mut service = Service::from(config);

        std::env::set_var("TEST_ENV", "test_value");

        let result = service.start(Arc::new(None)).await;
        assert!(result.is_ok());

        let mut receiver = service.subscribe_to_stream(ServiceStream::Stdout).await;

        // consume until we get TEST_ENV=test_value
        let mut found = false;
        while let Some(data) = receiver.recv().await {
            if data == *"TEST_ENV=test_value\n" {
                found = true;
                break;
            }
        }
        assert!(found, "TEST_ENV=test_value not found in output");

        service.stop().await.unwrap();
    }
}
