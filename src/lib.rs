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

#[cfg(test)]
mod test_utils;
