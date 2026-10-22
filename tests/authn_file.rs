use axum::{extract::Request, http::StatusCode, routing::post, Router};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

struct Harness {
    directory: tempfile::TempDir,
    api_url: String,
    requests: mpsc::UnboundedReceiver<String>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new() -> Self {
        let (sender, requests) = mpsc::unbounded_channel();
        let app = Router::new().fallback(post(move |request: Request| {
            let sender = sender.clone();
            async move {
                sender
                    .send(
                        request.headers()["Authorization"]
                            .to_str()
                            .unwrap()
                            .to_owned(),
                    )
                    .unwrap();
                (StatusCode::UNAUTHORIZED, "test registration rejected")
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let harness = Self {
            directory: tempfile::tempdir().unwrap(),
            api_url,
            requests,
            server,
        };
        harness.write(&harness.path("global.yaml"), "{}\n");
        harness
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn write(&self, path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn auth(&self, path: &Path, key: &str) {
        self.write(path, &serde_json::json!({"api_key": key}).to_string());
    }

    async fn run(
        &mut self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> (std::process::Output, Option<String>) {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("xdg"))
            .args([
                "--global-config",
                self.path("global.yaml").to_str().unwrap(),
            ])
            .args([
                "--api-url",
                &self.api_url,
                "--bind-address",
                "127.0.0.1",
                "--bind-port",
                "0",
            ])
            .args(args)
            .kill_on_drop(true);
        for (key, value) in env {
            command.env(key, value);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .expect("agent should exit promptly")
            .unwrap();
        let authorization = self.requests.try_recv().ok();
        assert!(
            self.requests.try_recv().is_err(),
            "only registration should be attempted"
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        (output, authorization)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn assert_key(output: &std::process::Output, authorization: Option<String>, key: &str) {
    assert_eq!(
        authorization.as_deref(),
        Some(format!("Bearer {key}").as_str())
    );
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(!logs.contains(key), "key leaked to logs: {logs}");
    assert!(logs.contains("[REDACTED]"), "{logs}");
}

#[tokio::test]
async fn configured_keys_override_saved_credentials_in_the_existing_hierarchy() {
    for winner in ["saved", "global", "project", "environment", "flag"] {
        let mut harness = Harness::new().await;
        let auth = harness.path("authn.json");
        harness.auth(&auth, "saved-secret-key");
        let mut args = vec!["--auth-file", auth.to_str().unwrap()];
        if winner != "saved" {
            harness.write(&harness.path("global.yaml"), "api_key: global-secret-key\n");
        }
        if ["project", "environment", "flag"].contains(&winner) {
            harness.write(
                &harness.path("kittengrid.yaml"),
                "api_key: project-secret-key\nservices: []\n",
            );
        }
        let env = if ["environment", "flag"].contains(&winner) {
            vec![("KITTENGRID_API_KEY", "environment-secret-key")]
        } else {
            vec![]
        };
        if winner == "flag" {
            args.extend(["--api-key", "flag-secret-key"]);
        }
        let (output, authorization) = harness.run(&args, &env).await;
        assert_key(&output, authorization, &format!("{winner}-secret-key"));
    }
}

#[tokio::test]
async fn auth_file_path_uses_flags_over_environment_over_project_over_global() {
    for winner in ["global", "project", "environment", "flag"] {
        let mut harness = Harness::new().await;
        let global = harness.path("global-auth.json");
        let project = harness.path("project-auth.json");
        let environment = harness.path("environment-auth.json");
        let flag = harness.path("flag-auth.json");
        for (path, source) in [
            (&global, "global"),
            (&project, "project"),
            (&environment, "environment"),
            (&flag, "flag"),
        ] {
            harness.auth(path, &format!("{source}-secret-key"));
        }
        harness.write(
            &harness.path("global.yaml"),
            &serde_yaml::to_string(&serde_json::json!({"auth_file":global})).unwrap(),
        );
        if winner != "global" {
            harness.write(
                &harness.path("kittengrid.yaml"),
                &serde_yaml::to_string(&serde_json::json!({"auth_file":project, "services":[]}))
                    .unwrap(),
            );
        }
        let env = if ["environment", "flag"].contains(&winner) {
            vec![("KITTENGRID_AUTH_FILE", environment.to_str().unwrap())]
        } else {
            vec![]
        };
        let args = if winner == "flag" {
            vec!["--auth-file", flag.to_str().unwrap()]
        } else {
            vec![]
        };
        let (output, authorization) = harness.run(&args, &env).await;
        assert_key(&output, authorization, &format!("{winner}-secret-key"));
    }
}

#[tokio::test]
async fn explicit_missing_or_malformed_authentication_file_fails_before_api_calls() {
    for contents in [
        None,
        Some("{private-secret-broken-json"),
        Some(r#"{"unsupported":"private-secret-value"}"#),
    ] {
        let mut harness = Harness::new().await;
        let path = harness.path("authn.json");
        if let Some(contents) = contents {
            harness.write(&path, contents);
        }
        let (output, authorization) = harness
            .run(&["--auth-file", path.to_str().unwrap()], &[])
            .await;
        assert!(authorization.is_none());
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(logs.contains("authn.json"), "{logs}");
        assert!(!logs.contains("private-secret"), "{logs}");
    }
}

#[tokio::test]
async fn unused_saved_credentials_do_not_break_explicit_api_keys() {
    let mut harness = Harness::new().await;
    let path = harness.path("authn.json");
    harness.write(&path, "broken-private-saved-key");
    let (output, authorization) = harness
        .run(
            &[
                "--auth-file",
                path.to_str().unwrap(),
                "--api-key",
                "explicit-secret-key",
            ],
            &[],
        )
        .await;
    assert_key(&output, authorization, "explicit-secret-key");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("broken-private-saved-key"));
}

#[tokio::test]
async fn saved_key_is_independent_of_the_configured_backend() {
    let mut first = Harness::new().await;
    let mut second = Harness::new().await;
    let path = first.path("authn.json");
    first.auth(&path, "private-secret-key");
    assert_ne!(first.api_url, second.api_url);
    for harness in [&mut first, &mut second] {
        let (output, authorization) = harness
            .run(&["--auth-file", path.to_str().unwrap()], &[])
            .await;
        assert_key(&output, authorization, "private-secret-key");
    }
}

#[tokio::test]
async fn explicit_opt_out_skips_even_a_malformed_selected_file() {
    let mut harness = Harness::new().await;
    let path = harness.path("authn.json");
    harness.write(&path, "{private-secret-broken-json");
    let (output, authorization) = harness
        .run(
            &["--no-auth-file", "--auth-file", path.to_str().unwrap()],
            &[],
        )
        .await;
    assert!(authorization.is_some());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-secret"));
}

#[cfg(unix)]
#[tokio::test]
async fn insecure_permissions_fail_before_registration() {
    use std::os::unix::fs::PermissionsExt;
    let mut harness = Harness::new().await;
    let path = harness.path("authn.json");
    harness.auth(&path, "private-secret-key");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (output, authorization) = harness
        .run(&["--auth-file", path.to_str().unwrap()], &[])
        .await;
    assert!(authorization.is_none());
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(logs.contains("chmod 600"), "{logs}");
    assert!(!logs.contains("private-secret-key"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn discovers_authn_in_the_xdg_directory_even_when_global_yaml_is_disabled() {
    let mut harness = Harness::new().await;
    harness.auth(
        &harness.path("xdg/kittengrid-agent/authn.json"),
        "xdg-secret-key",
    );
    harness.auth(
        &harness.path("home/.config/kittengrid-agent/authn.json"),
        "home-secret-key",
    );
    let (output, authorization) = harness.run(&["--no-global-config"], &[]).await;
    assert_key(&output, authorization, "xdg-secret-key");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn defaults_to_home_config_directory_without_an_absolute_xdg_directory() {
    let mut harness = Harness::new().await;
    harness.auth(
        &harness.path("home/.config/kittengrid-agent/authn.json"),
        "home-secret-key",
    );
    let (output, authorization) = harness.run(&[], &[("XDG_CONFIG_HOME", "")]).await;
    assert_key(&output, authorization, "home-secret-key");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn missing_discovered_authn_is_optional_and_does_not_create_a_file() {
    let mut harness = Harness::new().await;
    let path = harness.path("xdg/kittengrid-agent/authn.json");
    let (_, authorization) = harness.run(&[], &[]).await;
    assert!(authorization.is_some());
    assert!(!path.exists());
}
