use std::sync::Arc;

// This is mainly to abstract the agent itself, so we can
// use it more easily in tests.
use super::config::Config;

use log::{debug, error, info};

#[derive(Debug, Default)]
pub struct KittengridAgent {
    config: Config,
    api: Option<crate::kittengrid_api::KittengridApi>,
    services: Arc<crate::service::Services>,
    local_addr: Option<std::net::SocketAddr>,
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KittengridAgentError {
    #[error("Agent not registered")]
    NotRegisteredError,
    #[error("Agent is not listening.")]
    NotListeningError,
    #[error("Api Error: ({0})")]
    KittengridApiError(#[from] crate::kittengrid_api::KittengridApiError),
    #[error("Wireguard Error: ({0})")]
    WireguardError(#[from] Box<dyn std::error::Error>),
    #[error("Service Spawn Error: ({0})")]
    ServiceSpawnError(#[from] std::io::Error),
}

impl KittengridAgent {
    pub fn services(&self) -> Arc<crate::service::Services> {
        self.services.clone()
    }

    pub fn new(config: Config) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Reads config and initializes log
    pub async fn init(&self) {
        info!("Starting kittengrid with config: {:?}.", self.config);
        debug!("Config read: {:?}", self.config);

        debug!("Adding services to agent.");
        for service in self.config.services.iter() {
            let service: super::service::Service = service.clone().into();
            (*self.services).insert(service).await;
        }
    }

    /// Registers the agent with the kittengrid API and obtains a token to be
    /// used in subsequent requests.
    pub async fn register(&mut self) -> Result<(), crate::kittengrid_api::KittengridApiError> {
        let api = crate::kittengrid_api::from_registration(&self.config).await;
        if let Ok(api) = api {
            self.api = Some(api);
            Ok(())
        } else {
            Err(api.err().unwrap())
        }
    }

    /// Configures local network with wireguard tunnels.
    pub async fn configure_network(&mut self) -> Result<(), KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }

        let kg_api = self.api.as_ref().unwrap();

        if self.local_addr.is_none() {
            return Err(KittengridAgentError::NotListeningError);
        }

        // Fetch network configuration
        let peers = match kg_api.peers_create(self.local_addr.unwrap().port()).await {
            Ok(peers) => peers,
            Err(e) => {
                return Err(KittengridAgentError::KittengridApiError(e));
            }
        };

        for (device_counter, peer) in peers.iter().enumerate() {
            let endpoint = match kg_api.peers_get_endpoint(peer.network()).await {
                Ok(endpoint) => endpoint,
                Err(e) => {
                    return Err(KittengridAgentError::KittengridApiError(e));
                }
            };

            // Set up wireguard tunnel for the peer
            let device = match super::wireguard::WireGuard::new(device_counter).await {
                Ok(device) => device,
                Err(e) => {
                    return Err(KittengridAgentError::WireguardError(e));
                }
            };

            match device.set_config(peer, &endpoint).await {
                Ok(_) => {
                    info!(
                        "Successfully configured wireguard device {}.",
                        device.name()
                    );
                }
                Err(e) => {
                    return Err(KittengridAgentError::WireguardError(e));
                }
            }
        }

        Ok(())
    }

    /// For now, we just log errors.
    pub async fn set_status(&self, status: crate::kittengrid_api::PullRequestStatus) {
        if self.api.is_none() {
            return;
        }
        // Register with API
        if let Err(_e) = self
            .api
            .as_ref()
            .unwrap()
            .agents_update_pull_request(status.clone())
            .await
        {
            error!("Failed to set status to: {}.", status);
        };
    }

    /// Starts services in the agent.
    pub async fn spawn_services(
        &self,
        show_services_output: bool,
    ) -> Result<(), KittengridAgentError> {
        for (id, service) in self.services.descriptions().await {
            let name = service.name();
            info!("Spawning service: {} ({}).", id, name);
            let service = self.services.fetch(id).await;
            let service = service.unwrap();
            let mut service = service.lock().await;
            if show_services_output {
                service.show_output();
            }

            if let Err(e) = service.start(Arc::new(self.api.clone())).await {
                error!("Failed to spawn service: {}.", name);
                return Err(KittengridAgentError::ServiceSpawnError(e));
            }
        }
        Ok(())
    }

    /// Publishes services to the kittengrid API.
    pub async fn publish_services(&self) -> Result<(), KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }
        let services = self.services();
        for (id, service) in services.descriptions().await {
            // Register with API
            if let Err(e) = self
                .api
                .as_ref()
                .unwrap()
                .agents_create_service(id, service.name())
                .await
            {
                error!("Failed to publish service: {}.", service.name());
                return Err(KittengridAgentError::KittengridApiError(e));
            };
        }
        Ok(())
    }

    /// Registers services in the system to traffic can be routed to them.
    pub async fn register_services(&self) -> Result<(), KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }
        let services = self.services();
        for (id, service) in services.descriptions().await {
            // Register with API
            let healthcheck_path = service.health_check().map(|hc| hc.path.clone());

            let public_url = self
                .api
                .as_ref()
                .unwrap()
                .peers_create_service(
                    id,
                    &service.name(),
                    service.port(),
                    healthcheck_path,
                    None,
                    false,
                )
                .await?;

            info!(
                "Service {} registered with public URL: {}",
                service.name(),
                public_url
            );

            // Store the public URL in the service description
            let service = self.services.fetch(id).await.unwrap();
            let mut service = service.lock().await;
            service.set_public_url(public_url);
        }
        Ok(())
    }

    pub async fn register_service(
        &self,
        id: uuid::Uuid,
        name: &str,
        port: u16,
        healthcheck_path: Option<String>,
        path: Option<String>,
        websocket: bool,
    ) -> Result<String, KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }

        match self
            .api
            .as_ref()
            .unwrap()
            .peers_create_service(id, name, port, healthcheck_path, path, websocket)
            .await
        {
            Ok(response) => {
                info!("Successfully registered service: {}.", name);
                Ok(response)
            }
            Err(e) => {
                error!("Failed to register service: {}.", name);
                Err(KittengridAgentError::KittengridApiError(e))
            }
        }
    }

    /// Binds the agent to the network returining a listener.
    pub async fn bind(&mut self) -> tokio::net::TcpListener {
        let listener = tokio::net::TcpListener::bind(format!(
            "{}:{}",
            self.config.bind_address, self.config.bind_port
        ))
        .await
        .unwrap();
        let addr = listener.local_addr().unwrap();
        self.local_addr = Some(addr);

        info!("Listening on: {}", addr);
        listener
    }

    pub async fn wait(&self, listener: tokio::net::TcpListener) {
        crate::launch(
            listener,
            Arc::clone(&self.services),
            Arc::new(self.api.clone()),
        )
        .await;
    }
}
