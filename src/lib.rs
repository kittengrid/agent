use std::collections::HashMap;
use uuid::Uuid;

mod api_error;
pub mod config;
pub mod data_dir;
mod endpoints;
pub mod kittengrid_api;
pub mod utils;
use axum::{routing::get, Router};
pub mod wireguard;

extern crate alloc;

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
    axum::serve(listener, router()).await.unwrap();
}

#[cfg(test)]
pub mod test_utils;
