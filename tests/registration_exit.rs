use axum::{http::StatusCode, routing::post, Router};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

async fn run_agent(status: StatusCode) -> (std::process::Output, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let request_count = calls.clone();
    let app = Router::new().fallback(post(move || {
        let calls = request_count.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            (status, "")
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("agent.yml");
    std::fs::write(&config, "services: []\n").unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"))
            .current_dir(directory.path())
            .env_clear()
            .args(["--config", config.to_str().unwrap(), "--api-url", &api_url])
            .args(["--bind-address", "127.0.0.1", "--bind-port", "0"])
            .args(["--start-services", "true", "--start-terminal", "true"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("agent should exit instead of retrying registration")
    .unwrap();
    server.abort();
    (output, calls.load(Ordering::SeqCst))
}

#[tokio::test]
async fn registration_conflict_exits_successfully_without_further_api_calls() {
    let (output, calls) = run_agent(StatusCode::CONFLICT).await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(calls, 1);
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(logs.contains("PR/MR is closed or merged"), "{logs}");
    assert!(!logs.contains("Publishing service info"), "{logs}");
}

#[tokio::test]
async fn other_registration_errors_exit_with_failure() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::INTERNAL_SERVER_ERROR] {
        let (output, calls) = run_agent(status).await;
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(calls, 1);
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(logs.contains("Failed to register"), "{logs}");
    }
}
