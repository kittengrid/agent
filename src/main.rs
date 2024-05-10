use lib::config::get_config;
use lib::wireguard::WireGuard;
use log::{error, info};

use std::process::exit;

#[tokio::main]
async fn main() {
    let config = get_config();

    lib::utils::initialize_logger();
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.bind_address, config.bind_port))
            .await
            .unwrap();

    // Register with API
    let kg_api = match lib::kittengrid_api::from_registration(config).await {
        Ok(api) => {
            info!("Successfully registered with kittengrid api");
            api
        }
        Err(e) => {
            error!("Failed to register with kittengrid api: {}", e);
            exit(1);
        }
    };

    // Fetch network configuration
    let peers = match kg_api.peers_create().await {
        Ok(peers) => peers,
        Err(e) => {
            error!("Failed to create peers: {}", e);
            exit(1);
        }
    };

    for (device_counter, peer) in peers.iter().enumerate() {
        let endpoint = match kg_api.peers_get_endpoint(peer.network()).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                error!("Failed to fetch endpoints: {}", e);
                exit(1);
            }
        };

        // Set up wireguard tunnel for the peer
        let device = match WireGuard::new(device_counter).await {
            Ok(device) => device,
            Err(e) => {
                error!("Failed to create wireguard device: {}", e);
                exit(1);
            }
        };

        match device.set_config(peer, &endpoint).await {
            Ok(_) => {
                info!("Successfully configured wireguard device");
            }
            Err(e) => {
                error!("Failed to set wireguard device configuration: {}", e);
                exit(1);
            }
        }

        // block until all tun readers closed
        tokio::spawn(async move {
            info!("Starting kittengrid tunnel for device: {}", device.name());
            device.wait();
        });
    }
    info!("All interfaces started, launching the web server.");
    lib::launch(listener).await;
}
