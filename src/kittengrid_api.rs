use super::config::Config;
use serde::Deserialize;
use std::fmt;
use uuid::Uuid;

const API_BASE_PATH: &str = "/api/v1";

fn api_endpoint(api_url: &str, path: &str) -> String {
    format!(
        "{}{API_BASE_PATH}/{}",
        api_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[derive(Debug, Clone)]
pub struct KittengridApi {
    api_url: String,
    client: reqwest::Client,
    config: Config,
    registration_id: Uuid,
    replaced: tokio_util::sync::CancellationToken,
}

use thiserror::Error;

// We derive `thiserror::Error`
#[derive(Debug, Error)]
pub enum KittengridApiError {
    // The `#[from]` attribute generates `From<JsonRejection> for ApiError`
    // implementation. See `thiserror` docs for more information
    #[error("Request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("Authentication failed: {0}")]
    AuthenticationError(String),

    #[error("Unauthorized: {0}")]
    UnauthorizedError(String),

    #[error("ApiStatusError: {0}")]
    ApiStatusError(String),

    #[error("Change request is closed or merged; registration declined")]
    RegistrationDeclined,

    #[error("This agent has been replaced by a newer registration")]
    AgentReplaced,

    #[error("DeserializationError: {0}")]
    DeserializationError(String),
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct StartupOptions {
    pub start_services: bool,
    pub start_terminal: bool,
}

#[derive(Deserialize)]
struct RegistrationResponse {
    registration_id: Uuid,
    #[serde(flatten)]
    startup: StartupOptions,
}

fn agent_identity(config: &Config) -> Vec<(&'static str, &str)> {
    if config.environment_id.is_empty() {
        vec![
            ("vcs_provider", config.vcs_provider.as_str()),
            ("project_vcs_path", config.project_vcs_path.as_str()),
            ("pull_request_vcs_id", config.pull_request_vcs_id.as_str()),
        ]
    } else {
        vec![("environment_id", config.environment_id.as_str())]
    }
}

pub async fn from_registration(
    config: &Config,
) -> Result<(KittengridApi, StartupOptions), KittengridApiError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let bearer = crate::credential_provider::bearer(config)
        .await
        .map_err(|error| KittengridApiError::AuthenticationError(error.to_string()))?;
    let mut payload: serde_json::Map<String, serde_json::Value> = agent_identity(config)
        .into_iter()
        .map(|(key, value)| (key.to_string(), serde_json::json!(value)))
        .collect();
    if !config.workflow_run_id.is_empty() {
        payload.insert(
            "workflow_run_id".to_string(),
            serde_json::json!(config.workflow_run_id),
        );
    }
    let res = client
        .post(api_endpoint(&config.api_url, "agents/register"))
        .json(&payload)
        .bearer_auth(bearer)
        .send()
        .await;

    match res {
        Ok(res) => {
            if res.status().is_success() {
                let registration = res
                    .json::<RegistrationResponse>()
                    .await
                    .map_err(|error| KittengridApiError::DeserializationError(error.to_string()))?;
                Ok((
                    KittengridApi {
                        config: config.clone(),
                        api_url: config.api_url.clone(),
                        client,
                        registration_id: registration.registration_id,
                        replaced: tokio_util::sync::CancellationToken::new(),
                    },
                    registration.startup,
                ))
            } else if res.status() == reqwest::StatusCode::CONFLICT {
                // Only registration uses 409 to decline provisioning for a
                // closed/merged change request. Other API conflicts remain errors.
                Err(KittengridApiError::RegistrationDeclined)
            } else {
                Err(process_api_status_error_from_response(res).await)
            }
        }
        Err(e) => Err(KittengridApiError::RequestError(e)),
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Peer {
    address: std::net::Ipv4Addr,
    public_key: String,
    private_key: String,
    network: String,
}

impl Peer {
    pub fn network(&self) -> String {
        self.network.clone()
    }
    pub fn public_key(&self) -> String {
        self.public_key.clone()
    }
    pub fn private_key(&self) -> String {
        self.private_key.clone()
    }
    pub fn address(&self) -> std::net::Ipv4Addr {
        self.address
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Endpoint {
    public_url: String,
    address: std::net::Ipv4Addr,
    public_key: String,
    network: String,
}

impl Endpoint {
    pub fn network(&self) -> String {
        self.network.clone()
    }
    pub fn public_key(&self) -> String {
        self.public_key.clone()
    }
    pub fn address(&self) -> std::net::Ipv4Addr {
        self.address
    }
    pub fn public_url(&self) -> String {
        self.public_url.clone()
    }
}

#[derive(Debug, Clone)]
pub enum EnvironmentStatus {
    Created,
    Degraded,
    Booting,
    Sleeping,
    Error,
    Running,
    ShuttingDown,
}

impl fmt::Display for EnvironmentStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EnvironmentStatus::Created => write!(f, "created"),
            EnvironmentStatus::Degraded => write!(f, "degraded"),
            EnvironmentStatus::Booting => write!(f, "booting"),
            EnvironmentStatus::Sleeping => write!(f, "sleeping"),
            EnvironmentStatus::Error => write!(f, "error"),
            EnvironmentStatus::Running => write!(f, "running"),
            EnvironmentStatus::ShuttingDown => write!(f, "shutting_down"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ServiceStatus {
    Created,
    Stopped,
    Running,
    Paused,
    Exited,
    Dead,
    Restarting,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServiceStatus::Created => write!(f, "created"),
            ServiceStatus::Stopped => write!(f, "stopped"),
            ServiceStatus::Running => write!(f, "running"),
            ServiceStatus::Paused => write!(f, "paused"),
            ServiceStatus::Exited => write!(f, "exited"),
            ServiceStatus::Dead => write!(f, "dead"),
            ServiceStatus::Restarting => write!(f, "restarting"),
        }
    }
}
#[derive(Deserialize, Debug, Clone)]
pub struct PeersCreateServiceResponse {
    pub public_url: String,
}

impl KittengridApi {
    pub fn registration_id(&self) -> Uuid {
        self.registration_id
    }

    pub fn replacement_token(&self) -> tokio_util::sync::CancellationToken {
        self.replaced.clone()
    }

    async fn api_error(&self, response: reqwest::Response) -> KittengridApiError {
        let error = process_api_status_error_from_response(response).await;
        if matches!(error, KittengridApiError::AgentReplaced) {
            self.replaced.cancel();
        }
        error
    }

    pub async fn heartbeat(&self) -> Result<(), KittengridApiError> {
        let response = self
            .post("agents/heartbeat")
            .await?
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(self.api_error(response).await)
        }
    }

    pub fn set_startup_options(&mut self, start_services: bool, start_terminal: bool) {
        self.config.start_services = start_services;
        self.config.start_terminal = start_terminal;
    }

    /// Updates runtime status for either environment or VCS identity.
    pub async fn agents_update_environment(
        &self,
        status: EnvironmentStatus,
    ) -> Result<(), KittengridApiError> {
        let res = self
            .put("agents/environment")
            .await?
            .json(&serde_json::json!({
                "status": status.to_string(),
            }))
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(self.api_error(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    /// Publish services to Kittengrid internal api
    pub async fn agents_create_service(
        &self,
        id: Uuid,
        name: String,
    ) -> Result<(), KittengridApiError> {
        let status = if self.config.start_services {
            ServiceStatus::Created
        } else {
            ServiceStatus::Stopped
        };
        let mut payload = serde_json::json!({
            "name": name,
            "id": id.to_string(),
            "status": status.to_string(),
        });
        // Development services need no SHA; previews must supply one.
        if !self.config.last_commit_sha.is_empty() {
            payload["sha"] = serde_json::json!(self.config.last_commit_sha);
        }
        let res = self
            .post("agents/service")
            .await?
            .json(&payload)
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(self.api_error(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    // Requests Kittengrid Api to create peers
    // It returns a list of peers ready to be configured.
    pub async fn peers_create(&self, bind_port: u16) -> Result<Vec<Peer>, KittengridApiError> {
        let res = self
            .post("peers")
            .await?
            .json(&serde_json::json!({
                "bind_port": bind_port
            }))
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    let data = res.json::<Vec<Peer>>().await;
                    match data {
                        Ok(data) => Ok(data),
                        Err(e) => Err(KittengridApiError::DeserializationError(e.to_string())),
                    }
                } else {
                    Err(self.api_error(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    /// Creates a service in the Kittengrid Api.
    /// Returns the public_url
    pub async fn peers_create_service(
        &self,
        id: uuid::Uuid,
        name: &str,
        port: u16,
        healthcheck_path: Option<String>,
        path: Option<String>,
        websocket: bool,
    ) -> Result<String, KittengridApiError> {
        let mut data = serde_json::json!({
            "id": id,
            "port": port,
            "name": name,
            "websocket": websocket,
            "publish": self.config.start_services,
        });

        if let Some(healthcheck_path) = healthcheck_path {
            data["healthcheck_path"] = serde_json::Value::String(healthcheck_path);
        }
        if let Some(path) = path {
            data["path"] = serde_json::Value::String(path);
        }

        let res = self.post("peers/service").await?.json(&data).send().await;
        match res {
            Ok(res) => {
                if !res.status().is_success() {
                    // If the response is successful, we can process the response
                    return Err(self.api_error(res).await);
                }
                let data = res.json::<PeersCreateServiceResponse>().await;
                match data {
                    Ok(data) => Ok(data.public_url),
                    Err(e) => Err(KittengridApiError::DeserializationError(e.to_string())),
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    pub async fn peers_get_endpoint(&self, cidr: String) -> Result<Endpoint, KittengridApiError> {
        let res = self
            .get(format!("peers/endpoint?cidr={}", cidr).as_str())
            .await?
            .send()
            .await;

        match res {
            Ok(res) => {
                if res.status().is_success() {
                    let data = res.json::<Endpoint>().await;
                    match data {
                        Ok(data) => Ok(data),
                        Err(e) => Err(KittengridApiError::DeserializationError(e.to_string())),
                    }
                } else {
                    Err(self.api_error(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    // Updates the status of a given service
    pub async fn services_update_status(
        &self,
        id: uuid::Uuid,
        status: Option<ServiceStatus>,
        health_status: Option<crate::HealthStatus>,
        exit_status: Option<i32>,
    ) -> Result<(), KittengridApiError> {
        let mut payload = serde_json::Map::new();

        if let Some(status) = &status {
            payload.insert(
                "status".to_string(),
                serde_json::Value::String(status.to_string()),
            );
        }

        if let Some(health_status) = &health_status {
            payload.insert(
                "health_status".to_string(),
                serde_json::Value::String(health_status.to_string()),
            );
        }

        if let Some(exit_status) = &exit_status {
            payload.insert(
                "exit_status".to_string(),
                serde_json::Value::Number(serde_json::Number::from(*exit_status)),
            );
        }

        let res = self
            .put(&format!("services/{}", id))
            .await?
            .json(&serde_json::Value::Object(payload))
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(self.api_error(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    pub async fn post(&self, path: &str) -> Result<reqwest::RequestBuilder, KittengridApiError> {
        self.agent_request(self.client.post(api_endpoint(&self.api_url, path)))
            .await
    }

    pub async fn put(&self, path: &str) -> Result<reqwest::RequestBuilder, KittengridApiError> {
        self.agent_request(self.client.put(api_endpoint(&self.api_url, path)))
            .await
    }

    pub async fn get(&self, path: &str) -> Result<reqwest::RequestBuilder, KittengridApiError> {
        self.agent_request(self.client.get(api_endpoint(&self.api_url, path)))
            .await
    }

    async fn agent_request(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, KittengridApiError> {
        if self.replaced.is_cancelled() {
            return Err(KittengridApiError::AgentReplaced);
        }
        let bearer = crate::credential_provider::bearer(&self.config)
            .await
            .map_err(|error| KittengridApiError::AuthenticationError(error.to_string()))?;
        Ok(request
            .bearer_auth(bearer)
            .query(&agent_identity(&self.config))
            .query(&[("registration_id", self.registration_id)]))
    }
}

pub async fn process_api_status_error_from_response(res: reqwest::Response) -> KittengridApiError {
    if res.status().as_u16() == 409 {
        let body = res.text().await.unwrap_or_default();
        if serde_json::from_str::<serde_json::Value>(&body)
            .is_ok_and(|value| value["code"] == "agent_replaced")
        {
            return KittengridApiError::AgentReplaced;
        }
        return KittengridApiError::ApiStatusError(body);
    }
    if res.status().as_u16() == 401 {
        KittengridApiError::UnauthorizedError(res.text().await.unwrap_or_default())
    } else {
        KittengridApiError::ApiStatusError(res.text().await.unwrap_or_default())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn api_endpoints_normalize_slashes_and_preserve_query_parameters() {
        for origin in ["https://app.example.com", "https://app.example.com/"] {
            for path in [
                "peers/endpoint?cidr=10.121.0.0/16",
                "/peers/endpoint?cidr=10.121.0.0/16",
            ] {
                assert_eq!(
                    api_endpoint(origin, path),
                    "https://app.example.com/api/v1/peers/endpoint?cidr=10.121.0.0/16"
                );
            }
        }
    }

    async fn registration_server(
        status: axum::http::StatusCode,
        body: &'static str,
    ) -> (Config, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/api/v1/agents/register",
            axum::routing::post(move || async move { (status, body) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            api_url: format!("http://{}", listener.local_addr().unwrap()),
            ..Config::default()
        };
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (config, server)
    }

    #[tokio::test]
    async fn registration_conflict_is_declined_even_with_explicit_startup_flags() {
        let (mut config, server) = registration_server(axum::http::StatusCode::CONFLICT, "").await;
        config.start_services = true;
        config.start_terminal = true;
        let mut agent = crate::kittengrid_agent::KittengridAgent::new(config);
        assert!(matches!(
            agent.register().await,
            Err(KittengridApiError::RegistrationDeclined)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn registration_errors_remain_failures() {
        for status in [401, 403, 404, 500, 503] {
            let (config, server) = registration_server(
                axum::http::StatusCode::from_u16(status).unwrap(),
                "rejected",
            )
            .await;
            let error = from_registration(&config).await.unwrap_err();
            if status == 401 {
                assert!(matches!(error, KittengridApiError::UnauthorizedError(_)));
            } else {
                assert!(matches!(error, KittengridApiError::ApiStatusError(_)));
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn successful_registration_requires_a_registration_id() {
        let (config, server) = registration_server(
            axum::http::StatusCode::CREATED,
            r#"{"start_services":true}"#,
        )
        .await;
        assert!(matches!(
            from_registration(&config).await,
            Err(KittengridApiError::DeserializationError(_))
        ));
        server.abort();
    }

    #[tokio::test]
    async fn registration_success_preserves_startup_options() {
        let (config, server) = registration_server(
            axum::http::StatusCode::CREATED,
            r#"{"start_services":true,"start_terminal":false,"registration_id":"5c7bbfe8-a6b1-4f48-aeb8-72d25dcc2399"}"#,
        )
        .await;
        let (_, options) = from_registration(&config).await.unwrap();
        assert!(options.start_services);
        assert!(!options.start_terminal);
        server.abort();
    }

    #[tokio::test]
    async fn conflict_from_other_endpoints_remains_an_error() {
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(409)
                .body("conflict")
                .unwrap(),
        );
        assert!(matches!(
            process_api_status_error_from_response(response).await,
            KittengridApiError::ApiStatusError(_)
        ));
    }

    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    pub async fn from() {
        let (kittengrid_api, _) =
            crate::kittengrid_api::from_registration(crate::config::get_config())
                .await
                .unwrap();

        assert_eq!(
            kittengrid_api.config.api_key,
            crate::config::get_config().api_key
        );
    }

    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn from_with_invalid_api_key() {
        let mut config = crate::config::get_config().clone();
        config.api_key = "invalid".to_string();

        let kittengrid_api = crate::kittengrid_api::from_registration(&config).await;
        assert!(kittengrid_api.is_err());
        assert_eq!(
            format!("{}", kittengrid_api.unwrap_err()),
            "Unauthorized: HTTP Token: Access denied.\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn from_with_invalid_api_url() {
        let mut config = crate::config::get_config().clone();
        config.api_url = "invalid".to_string();

        let kittengrid_api = crate::kittengrid_api::from_registration(&config).await;
        assert!(kittengrid_api.is_err());

        assert!(kittengrid_api
            .unwrap_err()
            .to_string()
            .starts_with("Request failed"))
    }

    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn peers_create() {
        let (kittengrid_api, _) =
            crate::kittengrid_api::from_registration(crate::config::get_config())
                .await
                .unwrap();
        let peers = kittengrid_api.peers_create(0).await.unwrap();
        assert!(!peers.is_empty());
    }

    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn peers_get_endpoint() {
        let (kittengrid_api, _) =
            crate::kittengrid_api::from_registration(crate::config::get_config())
                .await
                .unwrap();
        let peers = kittengrid_api.peers_create(0).await.unwrap();
        let endpoint = kittengrid_api
            .peers_get_endpoint(peers[0].network.clone())
            .await
            .unwrap();
        assert!(!endpoint.public_url.is_empty());
    }
}
