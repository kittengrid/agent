use lib::kittengrid_agent::KittengridAgent;
use log::{error, info};
use std::process::exit;

#[tokio::main(flavor = "multi_thread", worker_threads = 20)]
async fn main() {
    let config = lib::config::get_config();
    let mut agent = KittengridAgent::new(config.clone());

    // Log initialization
    lib::utils::initialize_logger();

    // Service setup
    agent.init().await;

    // Bind to the network
    let listener = agent.bind().await;

    // Register with API so we can fetch network configuration
    match agent.register().await {
        Ok(_) => {
            info!("Successfully registered with kittengrid api.");
        }
        Err(e) => {
            error!("Failed to register with kittengrid api: {}", e);
            exit(1);
        }
    }

    info!("Publishing service info.");
    match agent.publish_services().await {
        Ok(_) => {
            info!("Successfully published services.");
        }
        Err(e) => {
            error!("Failed to publish services: {}.", e);
            exit(1);
        }
    }

    if config.start_services {
        info!("Service start disabled. Exiting.");
        agent
            .set_status(lib::kittengrid_api::PullRequestStatus::Booting)
            .await;

        // Network config
        match agent.configure_network().await {
            Ok(_) => {
                info!("Successfully configured network.");
            }
            Err(e) => {
                error!("Failed to configure network: {}.", e);
                exit(1);
            }
        }

        info!("Registering services.");
        match agent.register_services().await {
            Ok(_) => {
                info!("Successfully registered services.");
            }
            Err(e) => {
                error!("Failed to register services: {}.", e);
                exit(1);
            }
        }
        agent
            .set_status(lib::kittengrid_api::PullRequestStatus::Running)
            .await;

        info!("All interfaces up. Spawning services.");
        match agent.spawn_services(config.show_services_output).await {
            Ok(_) => {
                info!("Successfully spawned services.");
            }
            Err(e) => {
                error!("Failed to spawn services: {}.", e);
                exit(1);
            }
        }

        info!("Starting debugging terminal.");
        let id = uuid::Uuid::new_v4();
        match lib::ttyd::Executable::default()
            .start(&format!("/{}", id))
            .await
        {
            Ok(port) => {
                match agent
                    .register_service(
                        id,
                        "ttyd",
                        port,
                        Some(format!("/{}/token", id).to_string()),
                        Some(format!("/{}", id).to_string()),
                        true,
                    )
                    .await
                {
                    Ok(public_url) => {
                        info!("Terminal available at: {}", public_url);
                    }
                    Err(e) => {
                        error!("Failed to register service: {}. {}.", "ttyd", e)
                    }
                }
            }
            Err(e) => {
                error!("Failed to start TTYD: {}.", e);
                exit(1);
            }
        }

        info!("All services spawned. Waiting for incomming requests.");
        agent.wait(listener).await;
    } else {
        info!("Service start disabled. Exiting.");
        agent
            .set_status(lib::kittengrid_api::PullRequestStatus::Sleeping)
            .await;
    }

    exit(0);
}
