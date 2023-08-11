use std::collections::HashMap;
use uuid::Uuid;

mod api_error;
pub mod compose;
pub mod config;
pub mod data_dir;
pub mod docker_compose;
mod endpoints;
pub mod git_manager;
mod utils;
use axum::{
    routing::{get, post},
    Router,
};
use log::debug;
use std::net::TcpListener;

use once_cell::sync::Lazy;

extern crate log;
use std::sync::{Arc, RwLock};

pub type AgentState<'a> = Arc<RwLock<HashMap<Uuid, Arc<compose::Context<'a>>>>>;
static AGENT_STATE: Lazy<AgentState> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::<Uuid, Arc<compose::Context>>::new())));

/// Returns the agent state
pub fn agent_state() -> &'static AgentState<'static> {
    &AGENT_STATE
}

pub fn router() -> Router {
    Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route("/compose", post(endpoints::compose::create))
        .route("/compose/:id", get(endpoints::compose::show))
}

pub async fn launch(listener: TcpListener) {
    utils::initialize_logger();

    // build our application with a router
    let app = router();

    axum::Server::from_tcp(listener)
        .unwrap()
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// Makes a call to kittengrid API to register the agent advertise address
// so we can communicate with it
pub async fn publish_advertise_address(address: String, token: String, api_url: String) {
    debug!("Publishing advertise address: {} to: {}", address, api_url);
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/agents/register", api_url))
        .json(&serde_json::json!({ "address": address }))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    match res {
        Ok(res) => {
            if res.status().is_success() {
                debug!("Advertise address published successfully");
            } else {
                debug!("Failed to publish advertise address: {}", res.status());
            }
        }
        Err(e) => {
            debug!("Failed to publish advertise address: {}", e);
        }
    }
}

// Starts a container with the socat utility to expose the Docker Engine API through port 2376.
// This is needed to be able to stream logs from the containers.
pub fn expose_docker_engine_api() {
    debug!("Exposing docker engine API");

    if utils::is_port_in_use(2376) {
        debug!("Docker engine API is already exposed");
        return;
    }

    std::process::Command::new("docker")
        .args(&[
            "run",
            "-d",
            "--restart=always",
            "-p",
            "0.0.0.0:2376:2375",
            "-v",
            "/var/run/docker.sock:/var/run/docker.sock",
            "alpine/socat",
            "tcp-listen:2375,fork,reuseaddr",
            "unix-connect:/var/run/docker.sock",
        ])
        .output()
        .expect("Failed to expose docker engine API");
}

#[cfg(test)]
mod test_utils;
