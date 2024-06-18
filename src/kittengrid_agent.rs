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
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KittengridAgentError {
    #[error("Agent not registered")]
    NotRegisteredError,
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

    pub async fn register(&mut self) -> Result<(), crate::kittengrid_api::KittengridApiError> {
        let api = crate::kittengrid_api::from_registration(&self.config).await;
        if let Ok(api) = api {
            self.api = Some(api);
            Ok(())
        } else {
            Err(api.err().unwrap())
        }
    }

    pub async fn configure_network(&mut self) -> Result<(), KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }

        let kg_api = self.api.as_ref().unwrap();

        // Fetch network configuration
        let peers = match kg_api.peers_create().await {
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

            // block until all tun readers closed
            // @TODO: Save joinhandle to clean shutdown.
            tokio::spawn(async move {
                info!("Starting kittengrid tunnel for device: {}.", device.name());
                device.wait();
            });
        }

        Ok(())
    }

    pub async fn spawn_services(&self) -> Result<(), KittengridAgentError> {
        for service in self.services.descriptions().await {
            let name = service.name();
            info!("Spawning service: {}.", name);
            let service = self.services.fetch(&name).await;
            let service = service.unwrap();
            let mut service = service.lock().await;

            if let Err(e) = service.start().await {
                error!("Failed to spawn service: {}.", name);
                return Err(KittengridAgentError::ServiceSpawnError(e));
            }
        }
        Ok(())
    }

    pub async fn register_services(&self) -> Result<(), KittengridAgentError> {
        if self.api.is_none() {
            return Err(KittengridAgentError::NotRegisteredError);
        }
        let services = self.services();
        for service in services.descriptions().await {
            // Register with API
            let health_check = service.health_check().unwrap();
            let path = health_check.path.unwrap();

            if let Err(e) = self
                .api
                .as_ref()
                .unwrap()
                .peers_create_service(service.name(), service.port(), path)
                .await
            {
                error!("Failed to register service: {}.", service.name());
                return Err(KittengridAgentError::KittengridApiError(e));
            };
        }
        Ok(())
    }

    pub async fn wait(&self, listener: Option<tokio::net::TcpListener>) {
        let listener = match listener {
            Some(listener) => listener,
            None => tokio::net::TcpListener::bind(format!(
                "{}:{}",
                self.config.bind_address, self.config.bind_port
            ))
            .await
            .unwrap(),
        };

        crate::launch(listener, Arc::clone(&self.services)).await;
    }
}
