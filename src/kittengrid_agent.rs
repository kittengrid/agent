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
    capabilities: crate::Capabilities,
    local_addr: Option<std::net::SocketAddr>,
    development: bool,
    shutdown: tokio_util::sync::CancellationToken,
    interfaces: Vec<crate::wireguard::WireGuard>,
    proxies: tokio::task::JoinSet<()>,
}

use thiserror::Error;

async fn shutdown_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

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

    pub fn set_terminal_url(&mut self, url: String) {
        self.capabilities.terminal = crate::TerminalCapability {
            available: true,
            url: Some(url),
        };
    }

    pub fn new(config: Config) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    pub fn new_development(config: Config) -> Self {
        Self {
            config,
            development: true,
            ..Default::default()
        }
    }

    pub async fn stop_managed_services(&self) {
        if !self.development || self.config.start_services {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), self.services.stop())
                .await;
        }
    }

    /// Reads config and initializes log
    pub async fn init(&self) {
        info!("Starting kittengrid with config: {:?}.", self.config);
        debug!("Config read: {:?}", self.config);

        debug!("Adding services to agent.");
        for service in self.config.services.iter() {
            let mut service: super::service::Service = service.clone().into();
            if self.development && !self.config.start_services {
                service.mark_external();
            }
            (*self.services).insert(service).await;
        }
    }

    /// Registers using the configured credentials. Subsequent requests carry
    /// the server-assigned registration ID as well as bearer authentication.
    pub async fn register(
        &mut self,
    ) -> Result<crate::kittengrid_api::StartupOptions, crate::kittengrid_api::KittengridApiError>
    {
        let (mut api, requested_startup) = if self.development {
            crate::kittengrid_api::from_development_registration(&self.config).await?
        } else {
            crate::kittengrid_api::from_registration(&self.config).await?
        };
        let startup_options = crate::kittengrid_api::StartupOptions {
            start_services: self.config.start_services
                || (!self.development && requested_startup.start_services),
            start_terminal: !self.development
                && (self.config.start_terminal || requested_startup.start_terminal),
        };
        api.set_startup_options(
            startup_options.start_services,
            startup_options.start_terminal,
        );
        api.set_expose_only(self.development && !self.config.start_services);
        self.api = Some(api.clone());
        self.services.set_kittengrid_api(Some(api)).await;
        Ok(startup_options)
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
            let mut device = match if self.development {
                super::wireguard::WireGuard::new_development(
                    device_counter,
                    kg_api.registration_id(),
                )
                .await
            } else {
                super::wireguard::WireGuard::new(device_counter).await
            } {
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
            if self.development {
                if !self.config.start_services {
                    for service in &self.config.services {
                        if let Some(task) = crate::exposure::listen(
                            peer.address(),
                            service.port,
                            service.host.as_deref().unwrap_or("localhost"),
                        )
                        .await?
                        {
                            self.proxies.spawn(task);
                        }
                    }
                }
                self.interfaces.push(device);
            }
        }

        Ok(())
    }

    /// For now, we just log errors.
    pub async fn set_status(&self, status: crate::kittengrid_api::EnvironmentStatus) {
        if self.api.is_none() {
            return;
        }
        // Register with API
        if let Err(_e) = self
            .api
            .as_ref()
            .unwrap()
            .agents_update_environment(status.clone())
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

            if let Err(e) = service.start().await {
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
        self.try_bind().await.unwrap()
    }

    pub async fn try_bind(&mut self) -> std::io::Result<tokio::net::TcpListener> {
        let listener = tokio::net::TcpListener::bind(format!(
            "{}:{}",
            self.config.bind_address, self.config.bind_port
        ))
        .await?;
        let addr = listener.local_addr()?;
        self.local_addr = Some(addr);

        info!("Listening on: {}", addr);
        Ok(listener)
    }

    pub async fn wait(&self, listener: tokio::net::TcpListener) {
        let server = crate::launch(
            listener,
            Arc::clone(&self.services),
            self.capabilities.clone(),
            self.api.as_ref().map(|api| api.registration_id()),
            !self.development || self.config.start_services,
            self.development.then(|| self.shutdown.clone()),
        );
        if let Some(api) = &self.api {
            let replacement = api.replacement_token();
            let monitor = async {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = replacement.cancelled() => break,
                        _ = interval.tick() => {},
                    }
                    let result = tokio::select! {
                        _ = replacement.cancelled() => break,
                        result = api.heartbeat() => result,
                    };
                    match result {
                        Err(crate::kittengrid_api::KittengridApiError::AgentReplaced) => break,
                        Err(error) => log::warn!("Agent heartbeat failed: {}", error),
                        Ok(()) => {}
                    }
                }
            };
            tokio::select! {
                _ = server => {},
                _ = monitor => { info!("Agent replaced; exiting."); },
                _ = self.shutdown.cancelled(), if self.development => {},
                _ = async { if self.development { shutdown_signal().await; } else { std::future::pending::<()>().await; } } => {},
            }
            self.stop_managed_services().await;
            if self.development && !replacement.is_cancelled() {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.set_status(crate::kittengrid_api::EnvironmentStatus::Sleeping),
                )
                .await;
            }
        } else {
            server.await;
        }
    }
}
