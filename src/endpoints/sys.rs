// GET /sys/shutdown
//
// Description: Shuts down the server
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ShutdownParams {
    message: String,
}

#[axum::debug_handler]
pub async fn shutdown(params: Json<ShutdownParams>) -> &'static str {
    log::info!("Shutting down: {}", params.message);
    tokio::spawn(async move {
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
    use std::sync::Arc;

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
        assert!(server_test.services().stop(Arc::new(None)).await.is_ok());
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
        assert!(server_test.services().stop(Arc::new(None)).await.is_ok());
    }
}
