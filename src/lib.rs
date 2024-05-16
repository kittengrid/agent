use std::collections::HashMap;
use uuid::Uuid;

mod api_error;
pub mod config;
pub mod data_dir;
mod endpoints;
pub mod kittengrid_api;
pub mod utils;
use axum::{routing::get, Router};
pub mod persisted_buf_reader_broadcaster;
pub mod service;
pub mod wireguard;

extern crate alloc;

use once_cell::sync::Lazy;

extern crate log;
use std::sync::{Arc, RwLock};

pub type Services<'a> = Arc<RwLock<HashMap<String, service::Service>>>;
static SERVICES: Lazy<Services> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::<String, service::Service>::new())));

/// Returns the agent state
pub fn services() -> &'static Services<'static> {
    &SERVICES
}

pub fn router() -> Router {
    Router::new().route("/sys/hello", get(endpoints::sys::hello))
}

pub async fn launch(listener: tokio::net::TcpListener) {
    axum::serve(listener, router()).await.unwrap();
}

#[cfg(test)]
pub mod test_utils;
