use super::config::Config;
use serde::Deserialize;
use std::fmt;

#[derive(Debug)]
pub struct KittengridApi {
    api_token: String,
    api_url: String,
    client: reqwest::Client,
    config: Config,
}

use thiserror::Error;

// We derive `thiserror::Error`
#[derive(Debug, Error)]
pub enum KittengridApiError {
    // The `#[from]` attribute generates `From<JsonRejection> for ApiError`
    // implementation. See `thiserror` docs for more information
    #[error("Request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("Unauthorized: {0}")]
    UnauthorizedError(String),

    #[error("ApiStatusError: {0}")]
    ApiStatusError(String),

    #[error("DeserializationError: {0}")]
    DeserializationError(String),
}

#[derive(Deserialize)]
struct RegisterAgentResponse {
    token: String,
}

pub async fn from_registration(config: &Config) -> Result<KittengridApi, KittengridApiError> {
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/agents/register", config.api_url))
        .json(&serde_json::json!({
            "vcs_provider": config.vcs_provider,
            "pull_request_vcs_id": config.pull_request_vcs_id,
            "project_vcs_id": config.project_vcs_id,
            "workflow_id": config.workflow_id,
        }))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .send()
        .await;

    match res {
        Ok(res) => {
            if res.status().is_success() {
                match res.json::<RegisterAgentResponse>().await {
                    Ok(data) => {
                        let api_token = data.token;
                        Ok(KittengridApi {
                            api_token,
                            config: config.clone(),
                            api_url: config.api_url.clone(),
                            client,
                        })
                    }
                    Err(e) => Err(KittengridApiError::DeserializationError(e.to_string())),
                }
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
pub enum PullRequestStatus {
    Created,
    Booting,
    Sleeping,
    Error,
    Running,
    ShuttingDown,
    Merged,
}

impl fmt::Display for PullRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PullRequestStatus::Created => write!(f, "created"),
            PullRequestStatus::Booting => write!(f, "booting"),
            PullRequestStatus::Sleeping => write!(f, "sleeping"),
            PullRequestStatus::Error => write!(f, "error"),
            PullRequestStatus::Running => write!(f, "running"),
            PullRequestStatus::ShuttingDown => write!(f, "shutting_down"),
            PullRequestStatus::Merged => write!(f, "merged"),
        }
    }
}

impl KittengridApi {
    /// Updates a pull_request status using the agents Kittengrid internal api
    pub async fn agents_update_pull_request(
        &self,
        status: PullRequestStatus,
    ) -> Result<(), KittengridApiError> {
        let res = self
            .put("api/agents/pull_request")
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
                    Err(process_api_status_error_from_response(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    /// Publish services to Kittengrid internal api
    pub async fn agents_create_service(&self, name: String) -> Result<(), KittengridApiError> {
        let res = self
            .post("api/agents/service")
            .json(&serde_json::json!({
                "name": name,
                "sha": self.config.last_commit_sha,
            }))
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(process_api_status_error_from_response(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    // Requests Kittengrid Api to create peers
    // It returns a list of peers ready to be configured.
    pub async fn peers_create(&self, bind_port: u16) -> Result<Vec<Peer>, KittengridApiError> {
        let res = self
            .post("api/peers")
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
                    Err(process_api_status_error_from_response(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    pub async fn peers_create_service(
        &self,
        id: uuid::Uuid,
        name: String,
        port: u16,
        path: Option<String>,
    ) -> Result<(), KittengridApiError> {
        let res = self
            .post("api/peers/service")
            .json(&serde_json::json!({
                "id": id,
                "port": port,
                "path": path,
                "name": name,
                "publish": self.config.start_services,
                "sha": self.config.last_commit_sha,
            }))
            .send()
            .await;
        match res {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(process_api_status_error_from_response(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    pub async fn peers_get_endpoint(&self, cidr: String) -> Result<Endpoint, KittengridApiError> {
        let res = self
            .get(format!("api/peers/endpoint?cidr={}", cidr).as_str())
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
                    Err(process_api_status_error_from_response(res).await)
                }
            }
            Err(e) => Err(KittengridApiError::RequestError(e)),
        }
    }

    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}/{}", self.api_url, path))
            .header("Authorization", format!("Bearer {}", self.api_token))
    }

    pub fn put(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .put(format!("{}/{}", self.api_url, path))
            .header("Authorization", format!("Bearer {}", self.api_token))
    }

    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}/{}", self.api_url, path))
            .header("Authorization", format!("Bearer {}", self.api_token))
    }
}

pub async fn process_api_status_error_from_response(res: reqwest::Response) -> KittengridApiError {
    if res.status().as_u16() == 401 {
        KittengridApiError::UnauthorizedError(res.text().await.unwrap())
    } else {
        KittengridApiError::ApiStatusError(res.text().await.unwrap())
    }
}

#[cfg(test)]
mod test {
    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    pub async fn from() {
        let kittengrid_api = crate::kittengrid_api::from_registration(crate::config::get_config())
            .await
            .unwrap();

        assert!(!kittengrid_api.api_token.is_empty());
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
        let kittengrid_api = crate::kittengrid_api::from_registration(crate::config::get_config())
            .await
            .unwrap();
        let peers = kittengrid_api.peers_create(0).await.unwrap();
        assert!(!peers.is_empty());
    }

    // We need to stub the API calls
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn peers_get_endpoint() {
        let kittengrid_api = crate::kittengrid_api::from_registration(crate::config::get_config())
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
