// GET /sys/shutdown
//
// Description: Shuts down the server
use axum::{extract::State, Json};
use axum_extra::extract::WithRejection;
use serde::Deserialize;
use std::sync::Arc;

use crate::AxumState;

#[derive(Deserialize)]
pub struct ShutdownParams {
    message: String,
}

#[axum::debug_handler]
pub async fn shutdown(
    State(state): State<Arc<AxumState>>,
    params: WithRejection<Json<ShutdownParams>, crate::api_error::ApiError>,
) -> &'static str {
    log::info!("Shutting down: {}", params.0.message);
    tokio::spawn(async move {
        if let Err(error) = state.services.stop().await {
            log::error!("Error stopping services during shutdown: {}", error);
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    "{}"
}

// GET /sys/hello
//
// Description: Returns the cutiest Http response
pub async fn hello() -> &'static str {
    "kitty"
}

#[cfg(test)]
mod test {
    use crate::test_utils::*;
    use axum::http::StatusCode;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn hello() {
        let server_test = ServerTest::new(false).await;

        let response = server_test
            .client
            .get(server_test.url_for("/sys/hello"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "kitty");
        assert!(server_test.services().stop().await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn shutdown() {
        let server_test = ServerTest::new(false).await;

        let response = server_test
            .client
            .post(server_test.url_for("/sys/shutdown"))
            .json(&json!({"message": "shutting down"}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "{}");
        assert!(server_test.services().stop().await.is_ok());
    }
}
