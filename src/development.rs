//! Project resolution and development-environment allocation used by `dev`.
use crate::{config::Config, repository::RepositoryIdentity};
use serde::{Deserialize, Serialize};

#[derive(clap::Args, Clone)]
pub struct CreateOptions {
    /// Optional environment name; normalized by the backend into a URL-safe slug.
    #[arg(long, env = "KITTENGRID_DEV_NAME")]
    pub name: Option<String>,
    /// Explicit project public ID; bypass Git discovery and repository resolution.
    #[arg(long, env = "KITTENGRID_PROJECT_ID", conflicts_with = "repository")]
    pub project_id: Option<String>,
    /// Git repository URL instead of discovering a local remote.
    #[arg(long, env = "KITTENGRID_REPOSITORY", hide_env_values = true)]
    pub repository: Option<String>,
    /// Local Git remote to inspect when discovering a repository.
    #[arg(long, env = "KITTENGRID_GIT_REMOTE", default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolvedProject {
    pub id: String,
    pub name: Option<String>,
    pub vcs_provider: String,
    pub vcs_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DevelopmentEnvironment {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub status: String,
    pub name_generated: bool,
    pub created: bool,
}

pub async fn create(
    config: &Config,
    options: &CreateOptions,
) -> Result<DevelopmentEnvironment, String> {
    let client = Client::new(config).await?;
    let project = resolve_project(&client, config, options).await?;
    client.create(&project.id, options.name.as_deref()).await
}

pub(crate) async fn resolve_project(
    client: &Client<'_>,
    config: &Config,
    options: &CreateOptions,
) -> Result<ResolvedProject, String> {
    if let Some(id) = &options.project_id {
        client.project(id).await
    } else {
        let repository = if let Some(remote) = &options.repository {
            RepositoryIdentity::from_remote(remote)?
        } else if !config.vcs_provider.is_empty() || !config.project_vcs_path.is_empty() {
            if config.vcs_provider.is_empty() || config.project_vcs_path.is_empty() {
                return Err("Configure both vcs_provider and project_vcs_path, or use --repository / --project-id".into());
            }
            RepositoryIdentity {
                provider: config.vcs_provider.clone(),
                path: config.project_vcs_path.clone(),
            }
        } else {
            let directory =
                std::env::current_dir().map_err(|_| "Cannot locate the working directory")?;
            RepositoryIdentity::discover(&directory, &options.remote).await?
        };
        client.resolve(&repository).await
    }
}

pub(crate) struct Client<'a> {
    client: reqwest::Client,
    backend: url::Url,
    config: &'a Config,
}

impl<'a> Client<'a> {
    pub(crate) async fn new(config: &'a Config) -> Result<Self, String> {
        let client = Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|_| "Cannot initialize API client")?,
            backend: config.backend().map_err(|error| error.to_string())?,
            config,
        };
        client.bearer().await?;
        Ok(client)
    }

    async fn bearer(&self) -> Result<String, String> {
        let bearer = crate::credential_provider::bearer(self.config)
            .await
            .map_err(|error| error.to_string())?;
        if bearer.is_empty() {
            return Err(
                "Not authenticated. Run kittengrid-agent login first, or supply an API key".into(),
            );
        }
        Ok(bearer)
    }

    pub(crate) async fn project(&self, id: &str) -> Result<ResolvedProject, String> {
        let url = self.resource_url(id, &[])?;
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer().await?)
            .send()
            .await
            .map_err(|_| "Project resolution request failed")?;
        check_status(&response)?;
        response
            .json()
            .await
            .map_err(|_| "Invalid project response".into())
    }

    fn resource_url(&self, project: &str, segments: &[&str]) -> Result<url::Url, String> {
        if project.is_empty() || !project.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Invalid project public ID".into());
        }
        let mut url = self.backend.clone();
        url.path_segments_mut()
            .map_err(|_| "Invalid API endpoint")?
            .extend(["api", "v1", "projects", project])
            .extend(segments);
        Ok(url)
    }

    pub(crate) async fn cached(
        &self,
        project: &str,
        id: &str,
    ) -> Result<Option<DevelopmentEnvironment>, String> {
        let url = self.resource_url(project, &["development-environments", id])?;
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer().await?)
            .send()
            .await
            .map_err(|_| "Environment lookup failed")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        check_status(&response)?;
        response
            .json()
            .await
            .map(Some)
            .map_err(|_| "Invalid environment response".into())
    }

    async fn resolve(&self, repository: &RepositoryIdentity) -> Result<ResolvedProject, String> {
        let url = self
            .backend
            .join("api/v1/projects/resolve")
            .map_err(|_| "Invalid API endpoint")?;
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer().await?)
            .query(&[
                ("vcs_provider", &repository.provider),
                ("project_vcs_path", &repository.path),
            ])
            .send()
            .await
            .map_err(|_| "Project resolution request failed")?;
        check_status(&response)?;
        response
            .json()
            .await
            .map_err(|_| "Invalid project resolution response".into())
    }

    pub(crate) async fn create(
        &self,
        project_id: &str,
        name: Option<&str>,
    ) -> Result<DevelopmentEnvironment, String> {
        if project_id.is_empty() || !project_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Invalid project public ID".into());
        }
        let mut url = self.backend.clone();
        // Path segments are encoded individually; a supplied ID cannot escape
        // into another API path or become a query string.
        url.path_segments_mut()
            .map_err(|_| "Invalid API endpoint")?
            .extend([
                "api",
                "v1",
                "projects",
                project_id,
                "development-environments",
            ]);
        let body = name
            .map(|name| serde_json::json!({"name":name}))
            .unwrap_or_else(|| serde_json::json!({}));
        let response = self
            .client
            .post(url)
            .bearer_auth(self.bearer().await?)
            .json(&body)
            .send()
            .await
            .map_err(|_| "Environment creation request failed")?;
        check_status(&response)?;
        response
            .json()
            .await
            .map_err(|_| "Invalid environment creation response".into())
    }
}

fn check_status(response: &reqwest::Response) -> Result<(), String> {
    match response.status().as_u16() {
        200 | 201 => Ok(()),
        401 => Err("Authentication expired or invalid; run kittengrid-agent login again".into()),
        403 => Err("Your current credentials do not permit this operation".into()),
        404 => Err("Project not found or inaccessible; check the repository, organization, and project ID".into()),
        409 => Err("Project is ambiguous or the environment name belongs to another owner; use --project-id or choose another --name".into()),
        400 | 422 => Err("Invalid request: check the project identity and use a name that normalizes to a 1–63 character URL-safe slug".into()),
        status => Err(format!("Development-environment API rejected the request (HTTP {status})")),
    }
}
