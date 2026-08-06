use std::net::SocketAddr;

use std::fmt;
use std::sync::Arc;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
mod api_error;
mod binary_utils;
pub mod config;
pub mod data_dir;
mod endpoints;
pub mod kittengrid_api;
pub mod process_controller;
pub mod utils;
use axum::{
    routing::{get, post},
    Router,
};
pub mod kittengrid_agent;
pub mod persisted_buf_reader_broadcaster;
pub mod service;
pub mod ttyd;

pub mod wireguard;

extern crate alloc;

extern crate log;

#[derive(Clone, Debug, serde::Serialize)]
pub struct TerminalCapability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Default for TerminalCapability {
    fn default() -> Self {
        Self {
            available: false,
            url: None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Capabilities {
    pub version: u8,
    pub terminal: TerminalCapability,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            version: 1,
            terminal: TerminalCapability::default(),
        }
    }
}

pub struct AxumState {
    services: Arc<crate::service::Services>,
    capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

pub fn router(state: AxumState) -> Router {
    Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route("/sys/shutdown", post(endpoints::sys::shutdown))
        .route(
            "/public/capabilities",
            get(endpoints::public::capabilities::index),
        )
        .route("/public/services", get(endpoints::public::services::index))
        .route(
            "/public/services/{id}/stdout",
            get(endpoints::public::services::stdout),
        )
        .route(
            "/public/services/{id}/stderr",
            get(endpoints::public::services::stderr),
        )
        .route(
            "/public/services/{id}/combined_output",
            get(endpoints::public::services::combined_output),
        )
        .route(
            "/public/services/{id}/stop",
            post(endpoints::public::services::stop),
        )
        .route(
            "/public/services/{id}/start",
            post(endpoints::public::services::start),
        )
        .with_state(Arc::new(state))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
}

pub async fn launch(
    listener: tokio::net::TcpListener,
    services: Arc<crate::service::Services>,
    capabilities: Capabilities,
) {
    let state = AxumState {
        services,
        capabilities,
    };
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[cfg(test)]
pub mod test_utils;
