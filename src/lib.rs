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
