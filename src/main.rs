use lib::kittengrid_agent::KittengridAgent;
use log::{error, info};
use std::process::exit;

#[tokio::main(flavor = "multi_thread", worker_threads = 20)]
async fn main() {
    let mut agent = KittengridAgent::new(lib::config::get_config().clone());

    // Log initialization
    lib::utils::initialize_logger();

    // Service setup
    agent.init().await;

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

    info!("All interfaces up. Spawning services.");
    match agent.spawn_services().await {
        Ok(_) => {
            info!("Successfully spawned services.");
        }
        Err(e) => {
            error!("Failed to spawn services: {}.", e);
            exit(1);
        }
    }

    info!("Services started, registering.");
    match agent.register_services().await {
        Ok(_) => {
            info!("Successfully registered services.");
        }
        Err(e) => {
            error!("Failed to register services: {}.", e);
            exit(1);
        }
    }

    info!("All services spawned. Waiting for incomming requests.");
    agent.wait(None).await;
}
