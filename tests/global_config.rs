use axum::{body::to_bytes, extract::Request, http::StatusCode, routing::post, Router};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

#[derive(Debug)]
struct Registration {
    authorization: String,
    body: Value,
}

struct Harness {
    directory: tempfile::TempDir,
    api_url: String,
    requests: mpsc::UnboundedReceiver<Registration>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new() -> Self {
        let (sender, requests) = mpsc::unbounded_channel();
        let app = Router::new().fallback(post(move |request: Request| {
            let sender = sender.clone();
            async move {
                let authorization = request.headers()["Authorization"]
                    .to_str()
                    .unwrap()
                    .to_owned();
                let body = to_bytes(request.into_body(), 8192).await.unwrap();
                sender
                    .send(Registration {
                        authorization,
                        body: serde_json::from_slice(&body).unwrap(),
                    })
                    .unwrap();
                // Stop before any networking setup or service launch.
                (StatusCode::UNAUTHORIZED, "test registration rejected")
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            directory: tempfile::tempdir().unwrap(),
            api_url,
            requests,
            server,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn write(&self, path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn settings(&self, source: &str) -> String {
        format!(
            "api_url: {}\napi_key: {source}-secret-key\nenvironment_id: {source}-environment\n",
            self.api_url
        )
    }

    async fn run(
        &mut self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> (std::process::Output, Option<Registration>) {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("xdg"))
            .env("APPDATA", self.path("appdata"))
            .args(["--bind-address", "127.0.0.1", "--bind-port", "0"])
            .args(args)
            .kill_on_drop(true);
        for (name, value) in env {
            command.env(name, value);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .expect("agent should exit promptly")
            .unwrap();
        let registration = self.requests.try_recv().ok();
        assert!(
            self.requests.try_recv().is_err(),
            "only registration should be attempted"
        );
        (output, registration)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn assert_source(output: &std::process::Output, registration: Option<Registration>, source: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let registration = registration.expect("configuration should reach registration");
    assert_eq!(
        registration.authorization,
        format!("Bearer {source}-secret-key")
    );
    assert_eq!(
        registration.body["environment_id"],
        format!("{source}-environment")
    );
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(
        !logs.contains("secret-key"),
        "configuration secrets must not be logged: {logs}"
    );
    assert!(
        logs.contains("Config {"),
        "startup configuration should remain visible: {logs}"
    );
    assert!(
        logs.contains("[REDACTED]"),
        "API key should be masked: {logs}"
    );
    assert!(!logs.contains("panicked"), "{logs}");
}

#[tokio::test]
async fn flags_override_environment_then_project_then_global_settings() {
    for winner in ["global", "project", "environment", "flag"] {
        let mut harness = Harness::new().await;
        let global = harness.path("global.yaml");
        harness.write(&global, &harness.settings("global"));
        let mut args = vec!["--global-config", global.to_str().unwrap()];
        if winner != "global" {
            let project = harness.path("kittengrid.yaml");
            harness.write(&project, &(harness.settings("project") + "services: []\n"));
        }
        let env = if ["environment", "flag"].contains(&winner) {
            vec![
                ("KITTENGRID_API_KEY", "environment-secret-key"),
                ("KITTENGRID_ENVIRONMENT_ID", "environment-environment"),
            ]
        } else {
            vec![]
        };
        if winner == "flag" {
            args.extend([
                "--api-key",
                "flag-secret-key",
                "--environment-id",
                "flag-environment",
            ]);
        }
        let (output, registration) = harness.run(&args, &env).await;
        assert_source(&output, registration, winner);
    }
}

#[tokio::test]
async fn global_config_path_supports_environment_and_flag_override() {
    for flag_override in [false, true] {
        let mut harness = Harness::new().await;
        let env_path = harness.path("env-global.yaml");
        let flag_path = harness.path("flag-global.yaml");
        harness.write(&env_path, &harness.settings("environment"));
        harness.write(&flag_path, &harness.settings("flag"));
        let args = if flag_override {
            vec!["--global-config", flag_path.to_str().unwrap()]
        } else {
            vec![]
        };
        let (output, registration) = harness
            .run(
                &args,
                &[("KITTENGRID_GLOBAL_CONFIG", env_path.to_str().unwrap())],
            )
            .await;
        assert_source(
            &output,
            registration,
            if flag_override { "flag" } else { "environment" },
        );
    }
}

#[tokio::test]
async fn project_config_path_supports_environment_and_flag_override() {
    for flag_override in [false, true] {
        let mut harness = Harness::new().await;
        let global = harness.path("global.yaml");
        let env_path = harness.path("env-project.yaml");
        let flag_path = harness.path("flag-project.yaml");
        harness.write(&global, &harness.settings("global"));
        harness.write(&env_path, &harness.settings("environment"));
        harness.write(&flag_path, &harness.settings("flag"));
        let mut args = vec!["--global-config", global.to_str().unwrap()];
        if flag_override {
            args.extend(["--config", flag_path.to_str().unwrap()]);
        }
        let (output, registration) = harness
            .run(
                &args,
                &[("KITTENGRID_CONFIG_PATH", env_path.to_str().unwrap())],
            )
            .await;
        assert_source(
            &output,
            registration,
            if flag_override { "flag" } else { "environment" },
        );
    }
}

#[tokio::test]
async fn invalid_or_missing_explicit_global_file_fails_before_registration() {
    for contents in [
        None,
        Some("api_url: [invalid yaml\n"),
        Some("services: []\n"),
    ] {
        let mut harness = Harness::new().await;
        let global = harness.path("global.yaml");
        if let Some(contents) = contents {
            harness.write(&global, contents);
        }
        harness.write(
            &harness.path("kittengrid.yaml"),
            &harness.settings("project"),
        );
        let (output, registration) = harness
            .run(&["--global-config", global.to_str().unwrap()], &[])
            .await;
        assert_eq!(output.status.code(), Some(1));
        assert!(registration.is_none());
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(logs.contains("global.yaml"), "{logs}");
        assert!(!logs.contains("panicked"), "{logs}");
        if contents == Some("services: []\n") {
            assert!(logs.contains("cannot define services"), "{logs}");
        }
    }
}

#[tokio::test]
async fn disabling_global_settings_ignores_even_an_explicit_malformed_file() {
    let mut harness = Harness::new().await;
    let global = harness.path("global.yaml");
    harness.write(&global, "api_url: [invalid yaml\n");
    harness.write(
        &harness.path("kittengrid.yaml"),
        &harness.settings("project"),
    );
    let (output, registration) = harness
        .run(
            &[
                "--no-global-config",
                "--global-config",
                global.to_str().unwrap(),
            ],
            &[],
        )
        .await;
    assert_source(&output, registration, "project");
}

#[tokio::test]
async fn malformed_project_file_fails_cleanly_after_loading_global_settings() {
    let mut harness = Harness::new().await;
    let global = harness.path("global.yaml");
    harness.write(&global, &harness.settings("global"));
    harness.write(&harness.path("kittengrid.yaml"), "services: [broken\n");
    let (output, registration) = harness
        .run(&["--global-config", global.to_str().unwrap()], &[])
        .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(registration.is_none());
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(
        logs.contains("Failed to parse configuration file 'kittengrid.yaml'"),
        "{logs}"
    );
    assert!(!logs.contains("panicked"), "{logs}");
}

#[tokio::test]
async fn default_project_discovery_prefers_yml_over_yaml() {
    let mut harness = Harness::new().await;
    let global = harness.path("global.yaml");
    harness.write(&global, &harness.settings("global"));
    harness.write(
        &harness.path("kittengrid.yml"),
        &harness.settings("preferred"),
    );
    harness.write(&harness.path("kittengrid.yaml"), &harness.settings("other"));
    let (output, registration) = harness
        .run(&["--global-config", global.to_str().unwrap()], &[])
        .await;
    assert_source(&output, registration, "preferred");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn xdg_global_settings_take_precedence_over_home_fallback() {
    let mut harness = Harness::new().await;
    harness.write(
        &harness.path("xdg/kittengrid-agent/config.yaml"),
        &harness.settings("xdg"),
    );
    harness.write(
        &harness.path("home/.config/kittengrid-agent/config.yaml"),
        &harness.settings("home"),
    );
    let (output, registration) = harness.run(&[], &[]).await;
    assert_source(&output, registration, "xdg");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn empty_or_relative_xdg_uses_home_fallback() {
    for xdg in ["", "relative/path"] {
        let mut harness = Harness::new().await;
        harness.write(
            &harness.path("home/.config/kittengrid-agent/config.yaml"),
            &harness.settings("home"),
        );
        let (output, registration) = harness.run(&[], &[("XDG_CONFIG_HOME", xdg)]).await;
        assert_source(&output, registration, "home");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn missing_discovered_global_file_is_optional_but_malformed_file_is_not() {
    for malformed in [false, true] {
        let mut harness = Harness::new().await;
        if malformed {
            harness.write(
                &harness.path("xdg/kittengrid-agent/config.yaml"),
                "api_url: [broken\n",
            );
        }
        let api_url = harness.api_url.clone();
        let (output, registration) = harness.run(&["--api-url", &api_url], &[]).await;
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(registration.is_none(), malformed);
        let logs = String::from_utf8_lossy(&output.stderr);
        if malformed {
            assert!(
                logs.contains("Failed to parse configuration file"),
                "{logs}"
            );
        }
        assert!(!logs.contains("panicked"), "{logs}");
    }
}
