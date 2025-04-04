use std::net::SocketAddr;

use std::fmt;
use std::sync::Arc;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
mod api_error;
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

pub mod wireguard;

extern crate alloc;

extern crate log;

pub struct AxumState {
    services: Arc<crate::service::Services>,
    kittengrid_api: Arc<Option<kittengrid_api::KittengridApi>>,
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
        .route("/public/services", get(endpoints::public::services::index))
        .route(
            "/public/services/:id/stdout",
            get(endpoints::public::services::stdout),
        )
        .route(
            "/public/services/:id/stderr",
            get(endpoints::public::services::stderr),
        )
        .route(
            "/public/services/:id/stop",
            post(endpoints::public::services::stop),
        )
        .route(
            "/public/services/:id/start",
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
    kittengrid_api: Arc<Option<kittengrid_api::KittengridApi>>,
) {
    let state = AxumState {
        services,
        kittengrid_api,
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
