// POST /sys/shutdown
//
// Description: Shuts down the server
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::WithRejection;
use serde::Deserialize;
use std::sync::Arc;

use crate::AxumState;

#[derive(Deserialize)]
pub struct ShutdownParams {
    message: String,
    registration_id: Option<uuid::Uuid>,
}

#[axum::debug_handler]
pub async fn shutdown(
    State(state): State<Arc<AxumState>>,
    params: WithRejection<Json<ShutdownParams>, crate::api_error::ApiError>,
) -> Response {
    if params.0.registration_id != state.registration_id {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"code":"agent_replaced"})),
        )
            .into_response();
    }
    log::info!("Shutting down: {}", params.0.message);
    tokio::spawn(async move {
        if let Err(error) = state.services.stop().await {
            log::error!("Error stopping services during shutdown: {}", error);
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    Json(serde_json::json!({})).into_response()
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

    #[tokio::test]
    async fn stale_or_missing_shutdown_ids_cannot_stop_a_registered_agent() {
        use tower::ServiceExt;
        let current = uuid::Uuid::new_v4();
        let app = crate::router(crate::AxumState {
            services: std::sync::Arc::new(Default::default()),
            capabilities: Default::default(),
            registration_id: Some(current),
        });
        for payload in [
            json!({"message":"old shutdown"}),
            json!({"message":"old shutdown", "registration_id":uuid::Uuid::new_v4()}),
        ] {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/sys/shutdown")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
    }

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
