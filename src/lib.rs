use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

mod api_error;
pub mod compose;
mod config;
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
use once_cell::sync::Lazy;
use std::net::{IpAddr, SocketAddr};

extern crate log;
use std::sync::{Arc, RwLock};

use crate::config::get_config;
pub type AgentState<'a> = Arc<RwLock<HashMap<Uuid, Arc<compose::Context<'a>>>>>;
static AGENT_STATE: Lazy<AgentState> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::<Uuid, Arc<compose::Context>>::new())));

/// Returns the agent state
pub fn agent_state() -> &'static AgentState<'static> {
    &AGENT_STATE
}

pub async fn launch() {
    utils::initialize_logger();
    let config = get_config();

    // build our application with a router
    let app = Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route("/compose", post(endpoints::compose::create))
        .route("/compose/:id", get(endpoints::compose::show));

    let ip_addr = IpAddr::from_str(&config.bind_address).expect("Incorrect ip addr specified");
    let addr = SocketAddr::new(ip_addr, config.bind_port);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[cfg(test)]
mod test_utils;
