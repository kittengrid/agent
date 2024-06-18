use std::net::SocketAddr;

use std::sync::Arc;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
mod api_error;
pub mod config;
pub mod data_dir;
mod endpoints;
pub mod kittengrid_api;
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

pub fn router(services: Arc<crate::service::Services>) -> Router {
    Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route(
            "/services/:service_name/stdout",
            get(endpoints::services::stdout),
        )
        .route(
            "/services/:service_name/stderr",
            get(endpoints::services::stderr),
        )
        .route(
            "/services/:service_name/stop",
            post(endpoints::services::stop),
        )
        .route(
            "/services/:service_name/start",
            post(endpoints::services::start),
        )
        .with_state(services)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
}

pub async fn launch(listener: tokio::net::TcpListener, services: Arc<crate::service::Services>) {
    axum::serve(
        listener,
        router(services).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[cfg(test)]
pub mod test_utils;
