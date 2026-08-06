use crate::AxumState;
use axum::{extract::State, response::Json};
use std::sync::Arc;

pub async fn index(State(state): State<Arc<AxumState>>) -> Json<crate::Capabilities> {
    Json(state.capabilities.clone())
}

#[cfg(test)]
mod test {
    use crate::test_utils::ServerTest;
    use reqwest::StatusCode;

    #[tokio::test]
    async fn lists_agent_capabilities() {
        let server = ServerTest::new(false).await;

        let response = server
            .client
            .get(server.url_for("/public/capabilities"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let capabilities: serde_json::Value = response.json().await.unwrap();
        assert_eq!(capabilities["version"], 1);
        assert_eq!(capabilities["terminal"]["available"], false);
    }
}
