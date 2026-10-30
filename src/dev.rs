use crate::{
    config::Config,
    development::{self, Client, CreateOptions},
    kittengrid_agent::KittengridAgent,
    login::LoginOptions,
    workspaces::{self, Store},
};
use std::{io::IsTerminal, path::PathBuf};

#[derive(clap::Args, Clone)]
pub struct DevOptions {
    #[command(flatten)]
    pub environment: CreateOptions,
    #[command(flatten)]
    pub login: LoginOptions,
    /// Opt into managing configured processes. By default, only expose them.
    #[arg(long)]
    pub start_services: bool,
    /// Override the global workspace mapping file.
    #[arg(long, env = "KITTENGRID_WORKSPACES_FILE")]
    pub workspaces_file: Option<PathBuf>,
}

async fn prepare(
    config: &Config,
    options: &DevOptions,
) -> Result<development::DevelopmentEnvironment, String> {
    let client = Client::new(config).await?;
    let project = development::resolve_project(&client, config, &options.environment).await?;
    let identity = workspaces::identity(&project).await?;
    let file = options
        .workspaces_file
        .clone()
        .or_else(|| dirs::config_dir().map(|p| p.join("kittengrid-agent/workspaces.json")))
        .ok_or("Cannot locate workspace state; supply --workspaces-file")?;
    let mut store = Store::open(file).await?;
    let environment = if options.environment.name.is_none() {
        if let Some(id) = store.get(&identity)? {
            client.cached(&project.id, id).await?
        } else {
            None
        }
    } else {
        None
    };
    let environment = match environment {
        Some(environment) => environment,
        None => {
            client
                .create(&project.id, options.environment.name.as_deref())
                .await?
        }
    };
    store.save(identity, environment.id.clone())?;
    Ok(environment)
}

fn authentication_error(error: &str) -> bool {
    error.starts_with("Not authenticated")
        || error.starts_with("Authentication expired")
        || error.contains("run login again")
        || error.contains("run kittengrid-agent login again")
}

pub async fn run(config: &Config, options: &DevOptions) -> Result<(), String> {
    if config.services.is_empty() {
        return Err("Define services with names and ports in kittengrid.yaml (or --config) before running dev".into());
    }
    let mut ports = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for service in &config.services {
        if service.port == 0
            || service.name.is_empty()
            || !ports.insert(service.port)
            || !names.insert(&service.name)
        {
            return Err(
                "Dev services need unique nonempty names and unique ports between 1 and 65535"
                    .into(),
            );
        }
    }
    let environment = match prepare(config, options).await {
        Err(error) if config.api_key.is_empty() && authentication_error(&error) => {
            if !std::io::stdin().is_terminal() && !options.login.manual && !options.login.no_browser
            {
                return Err("Login required. Run kittengrid-agent login first, or explicitly use dev --manual / --no-browser".into());
            }
            crate::login::run(config, &options.login).await?;
            prepare(config, options).await?
        }
        result => result?,
    };
    let mut config = config.clone();
    config.environment_id = environment.id.clone();
    config.workflow_run_id.clear();
    config.last_commit_sha.clear();
    config.start_services = options.start_services;
    config.start_terminal = false;
    crate::utils::initialize_logger();
    println!(
        "Development environment: {} ({})",
        environment.name, environment.id
    );
    let show_output = config.show_services_output;
    let mut agent = KittengridAgent::new_development(config);
    agent.init().await;
    let result = async {
        let listener = agent
            .try_bind()
            .await
            .map_err(|error| format!("Cannot bind agent listener: {error}"))?;
        agent.register().await.map_err(|error| error.to_string())?;
        agent
            .publish_services()
            .await
            .map_err(|error| error.to_string())?;
        agent.configure_network().await.map_err(|error| {
            format!("Cannot configure userspace development networking: {error}")
        })?;
        agent
            .register_services()
            .await
            .map_err(|error| error.to_string())?;
        if options.start_services {
            agent
                .spawn_services(show_output)
                .await
                .map_err(|error| error.to_string())?;
        }
        agent
            .set_status(crate::kittengrid_api::EnvironmentStatus::Running)
            .await;
        println!(
            "{} Press Ctrl-C to stop the agent.",
            if options.start_services {
                "Managing configured services."
            } else {
                "Exposing services; their processes remain externally managed."
            }
        );
        agent.wait(listener).await;
        Ok(())
    }
    .await;
    if result.is_err() {
        agent.stop_managed_services().await;
        agent
            .set_status(crate::kittengrid_api::EnvironmentStatus::Error)
            .await;
    }
    result
}
