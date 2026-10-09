use axum::{http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};

#[tokio::test]
async fn environment_id_can_be_supplied_by_cli_environment_or_yaml() {
    for source in ["cli", "environment", "yaml"] {
        let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new().route(
            "/api/agents/register",
            post(move |Json(body): Json<Value>| {
                let sender = sender.clone();
                async move {
                    sender.send(body).unwrap();
                    // Stop before network setup: this test checks real binary
                    // configuration parsing and its registration payload.
                    (StatusCode::NOT_FOUND, "unknown environment")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.yml");
        let yaml = if source == "yaml" {
            "environment_id: 7ia79uwdfc4j\nservices: []\n"
        } else {
            "services: []\n"
        };
        std::fs::write(&path, yaml).unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
        command
            .current_dir(directory.path())
            .env_clear()
            .args(["--config", path.to_str().unwrap(), "--api-url", &api_url])
            .kill_on_drop(true);
        match source {
            "cli" => {
                command.args(["--environment-id", "7ia79uwdfc4j"]);
            }
            "environment" => {
                command.env("KITTENGRID_ENVIRONMENT_ID", "7ia79uwdfc4j");
            }
            _ => {}
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}: {output:?}");
        let request = tokio::time::timeout(std::time::Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            json!({"environment_id": "7ia79uwdfc4j"}),
            "{source}"
        );
        server.abort();
    }
}
