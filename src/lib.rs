use std::collections::HashMap;
use uuid::Uuid;

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
use std::net::SocketAddr;

extern crate log;
use std::sync::{Arc, RwLock};
pub type AgentState<'a> = Arc<RwLock<HashMap<Uuid, Arc<compose::Context<'a>>>>>;
static AGENT_STATE: Lazy<AgentState> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::<Uuid, Arc<compose::Context>>::new())));

/// Returns the agent state
pub fn agent_state() -> &'static AgentState<'static> {
    &AGENT_STATE
}

pub async fn launch() {
    utils::initialize_logger();

    // build our application with a router
    let app = Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route("/compose", post(endpoints::compose::create))
        .route("/compose/:id", get(endpoints::compose::show));

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
    // rocket::build()
    //     .manage(state)
    //     .mount("/", routes![endpoints::compose::new])
    //     .mount("/", routes![endpoints::compose::status])
    //     .mount("/", routes![endpoints::compose::stop])
    //     .mount("/", routes![endpoints::compose::start])
    //     .mount("/", routes![endpoints::compose::show])
    //     .mount("/", routes![endpoints::sys::hello])
}

#[cfg(test)]
mod test_utils;
