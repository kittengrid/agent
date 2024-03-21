use std::collections::HashMap;
use uuid::Uuid;

mod api_error;
pub mod config;
pub mod data_dir;
mod endpoints;
mod utils;
use axum::{routing::get, Router};
use log::debug;

use once_cell::sync::Lazy;

extern crate log;
use std::sync::{Arc, RwLock};

pub type AgentState<'a> = Arc<RwLock<HashMap<Uuid, i32>>>;
static AGENT_STATE: Lazy<AgentState> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::<Uuid, i32>::new())));

/// Returns the agent state
pub fn agent_state() -> &'static AgentState<'static> {
    &AGENT_STATE
}

pub fn router() -> Router {
    Router::new().route("/sys/hello", get(endpoints::sys::hello))
}

pub async fn launch(listener: tokio::net::TcpListener) {
    utils::initialize_logger();

    axum::serve(listener, router()).await.unwrap();
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

#[cfg(test)]
mod test_utils;
