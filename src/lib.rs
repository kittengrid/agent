use persisted_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
use std::{collections::HashMap, net::SocketAddr};

use tower_http::trace::{DefaultMakeSpan, TraceLayer};

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

pub async fn stdout_receiver_for_service(
    service_name: &str,
) -> Option<PersistedBufReaderBroadcaster> {
    let services = services().read().unwrap();

    if let Some(service) = services.get(service_name) {
        service.stdout()
    } else {
        None
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/sys/hello", get(endpoints::sys::hello))
        .route(
            "/services/:service_name/stdout",
            get(endpoints::services::stdout),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
}

pub async fn launch(listener: tokio::net::TcpListener) {
    axum::serve(
        listener,
        router().into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[cfg(test)]
pub mod test_utils;
