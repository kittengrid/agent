use axum::{http::StatusCode, routing::post, Router};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

async fn run_agent(
    explicit_config: Option<&str>,
    default_config: bool,
) -> (std::process::Output, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let request_count = calls.clone();
    let app = Router::new().fallback(post(move || {
        let calls = request_count.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            (StatusCode::UNAUTHORIZED, "")
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    if default_config {
        std::fs::write(directory.path().join("kittengrid.yml"), "services: []\n").unwrap();
    }
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
    command
        .current_dir(directory.path())
        .env_clear()
        .args([
            "--api-url",
            &api_url,
            "--bind-address",
            "127.0.0.1",
            "--bind-port",
            "0",
        ])
        .kill_on_drop(true);
    if let Some(path) = explicit_config {
        command.args(["--config", path]);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .expect("agent should exit promptly")
        .unwrap();
    server.abort();
    (output, calls.load(Ordering::SeqCst))
}

#[tokio::test]
async fn missing_explicit_config_exits_before_registration_even_with_a_default_file() {
    for default_config in [false, true] {
        let (output, calls) = run_agent(Some("kittengrid.yaml"), default_config).await;
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(calls, 0);
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(
            logs.contains("Failed to open configuration file 'kittengrid.yaml'"),
            "{logs}"
        );
        assert!(!logs.contains("Successfully spawned services"), "{logs}");
        assert!(!logs.contains("panicked"), "{logs}");
    }
}

#[tokio::test]
async fn omitting_config_still_supports_defaults_and_auto_discovery() {
    for default_config in [false, true] {
        let (output, calls) = run_agent(None, default_config).await;
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(calls, 1);
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(logs.contains("Failed to register"), "{logs}");
        assert!(
            !logs.contains("Failed to open configuration file"),
            "{logs}"
        );
    }
}
