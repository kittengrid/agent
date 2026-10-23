use axum::{
    extract::Form,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};
use url::Url;

struct Harness {
    directory: tempfile::TempDir,
    api_url: String,
    tokens: mpsc::UnboundedReceiver<HashMap<String, String>>,
    registrations: mpsc::UnboundedReceiver<String>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new() -> Self {
        let (tx, tokens) = mpsc::unbounded_channel();
        let (api_tx, registrations) = mpsc::unbounded_channel();
        let app = Router::new().route("/oauth/token", post(move |Form(fields): Form<HashMap<String, String>>| {
            let tx = tx.clone(); async move {
                let refresh = fields.get("grant_type").is_some_and(|s| s == "refresh_token");
                let reject = fields.get("refresh_token").is_some_and(|s| s == "revoked-secret");
                tx.send(fields).unwrap();
                if reject { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant", "error_description":"revoked-secret"}))); }
                let prefix = if refresh { "rotated" } else { "initial" };
                (StatusCode::OK, Json(serde_json::json!({"access_token":format!("{prefix}-access-secret"), "refresh_token":format!("{prefix}-refresh-secret"), "expires_in":3600, "token_type":"Bearer"})))
            }
        })).route("/api/v1/agents/register", post(move |headers: HeaderMap| {
            let tx = api_tx.clone(); async move {
                tx.send(headers["authorization"].to_str().unwrap().to_owned()).unwrap();
                (StatusCode::UNAUTHORIZED, "test registration refused")
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("global.yaml"), "{}\n").unwrap();
        Self {
            directory,
            api_url,
            tokens,
            registrations,
            server,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
    fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("xdg"))
            .env("BROWSER", self.path("test-browser"))
            .env("BROWSER_CAPTURE", self.path("browser-url"))
            .args([
                "--global-config",
                self.path("global.yaml").to_str().unwrap(),
                "--api-url",
                &self.api_url,
                "--auth-file",
                self.path("authn.json").to_str().unwrap(),
                "--bind-address",
                "127.0.0.1",
                "--bind-port",
                "0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
    async fn login(
        &self,
        args: &[&str],
    ) -> (
        tokio::process::Child,
        BufReader<tokio::process::ChildStdout>,
        Url,
    ) {
        let mut command = self.command();
        command.arg("login").args(args);
        let mut child = command.spawn().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let url = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut line = String::new();
                assert_ne!(
                    reader.read_line(&mut line).await.unwrap(),
                    0,
                    "login exited before printing URL"
                );
                if line.starts_with("http") {
                    break Url::parse(line.trim()).unwrap();
                }
            }
        })
        .await
        .unwrap();
        (child, reader, url)
    }
    fn saved(&self) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.path("authn.json")).unwrap()).unwrap()
    }
    fn save_oauth(&self, expires_at: u64, refresh: &str) {
        let path = self.path("authn.json");
        std::fs::write(&path, serde_json::to_vec(&serde_json::json!({"oauth":{"client_id":"kittengrid-agent", "access_token":"initial-access-secret", "refresh_token":refresh, "expires_at":expires_at}})).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn params(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}
async fn finish(
    child: tokio::process::Child,
    mut stdout: BufReader<tokio::process::ChildStdout>,
    success: bool,
) {
    let mut rest = String::new();
    let (output, _) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(child.wait_with_output(), stdout.read_to_string(&mut rest))
    })
    .await
    .expect("login must terminate");
    let output = output.unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.success(), success, "{stderr} {rest}");
    for secret in [
        "access-secret",
        "refresh-secret",
        "test-authorization-code",
        "revoked-secret",
    ] {
        assert!(!format!("{stderr}{rest}").contains(secret));
    }
}

#[tokio::test]
async fn no_browser_receives_callback_checks_state_and_exchanges_pkce() {
    let mut harness = Harness::new().await;
    let (child, stdout, url) = harness.login(&["--no-browser"]).await;
    let parameters = params(&url);
    assert_eq!(parameters["code_challenge_method"], "S256");
    let callback = &parameters["redirect_uri"];
    assert!(callback.starts_with("http://127.0.0.1:"));
    let client = reqwest::Client::new();
    let invalid = client
        .get(callback)
        .query(&[
            ("state", "wrong-state"),
            ("code", "test-authorization-code"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let response = client
        .get(callback)
        .query(&[
            ("state", parameters["state"].as_str()),
            ("code", "test-authorization-code"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    finish(child, stdout, true).await;
    let fields = harness.tokens.try_recv().unwrap();
    assert_eq!(fields["grant_type"], "authorization_code");
    assert_eq!(fields["redirect_uri"], *callback);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(fields["code_verifier"].as_bytes())),
        parameters["code_challenge"]
    );
    assert_eq!(
        harness.saved()["oauth"]["refresh_token"],
        "initial-refresh-secret"
    );
    assert!(harness.saved().get("api_key").is_none());
    assert!(
        !harness.path("browser-url").exists(),
        "--no-browser must not launch a browser"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(harness.path("authn.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(
        client.get(callback).send().await.is_err(),
        "callback port must be closed"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn default_mode_uses_the_native_browser_launcher() {
    use std::os::unix::fs::PermissionsExt;
    let harness = Harness::new().await;
    let browser = harness.path("test-browser");
    std::fs::write(
        &browser,
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$BROWSER_CAPTURE\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (child, stdout, url) = harness.login(&[]).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(harness.path("browser-url"))
            .ok()
            .as_deref()
            != Some(url.as_str())
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(harness.path("browser-url")).unwrap(),
        url.as_str()
    );
    let parameters = params(&url);
    reqwest::Client::new()
        .get(&parameters["redirect_uri"])
        .query(&[
            ("state", parameters["state"].as_str()),
            ("code", "test-authorization-code"),
        ])
        .send()
        .await
        .unwrap();
    finish(child, stdout, true).await;
}

#[tokio::test]
async fn manual_handoff_does_not_require_a_local_server() {
    let mut harness = Harness::new().await;
    let (mut child, stdout, url) = harness.login(&["--manual", "--no-browser"]).await;
    let parameters = params(&url);
    assert_eq!(
        parameters["redirect_uri"],
        format!("{}/oauth/manual", harness.api_url)
    );
    let handoff =
        serde_json::json!({"state":parameters["state"], "code":"test-authorization-code"});
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{handoff}\n").as_bytes())
        .await
        .unwrap();
    finish(child, stdout, true).await;
    assert_eq!(
        harness.tokens.try_recv().unwrap()["client_id"],
        "kittengrid-agent"
    );
    assert_eq!(harness.saved()["oauth"]["client_id"], "kittengrid-agent");
}

#[tokio::test]
async fn invalid_manual_state_or_denial_never_exchanges_or_overwrites_credentials() {
    for denied in [false, true] {
        let mut harness = Harness::new().await;
        harness.save_oauth(0, "old-refresh-secret");
        let before = std::fs::read(harness.path("authn.json")).unwrap();
        let (mut child, stdout, url) = harness.login(&["--manual"]).await;
        let parameters = params(&url);
        let value = if denied {
            serde_json::json!({"state":parameters["state"], "error":"access_denied"})
        } else {
            serde_json::json!({"state":"wrong-state", "code":"test-authorization-code"})
        };
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
        finish(child, stdout, false).await;
        assert!(harness.tokens.try_recv().is_err());
        assert_eq!(std::fs::read(harness.path("authn.json")).unwrap(), before);
    }
}

#[tokio::test]
async fn fresh_oauth_credentials_authenticate_without_refresh() {
    let mut harness = Harness::new().await;
    harness.save_oauth(u64::MAX, "initial-refresh-secret");
    let output = harness.command().output().await.unwrap();
    assert!(!output.status.success()); // mock registration intentionally returns 401
    assert_eq!(
        harness.registrations.try_recv().unwrap(),
        "Bearer initial-access-secret"
    );
    assert!(harness.tokens.try_recv().is_err());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("access-secret"));
}

#[tokio::test]
async fn running_clients_refresh_before_followup_api_requests() {
    let harness = Harness::new().await;
    let (sender, mut requests) = mpsc::unbounded_channel();
    let registration_sender = sender.clone();
    let app = Router::new()
        .route("/api/v1/agents/register", post(move |headers: HeaderMap| {
            let sender = registration_sender.clone();
            async move { sender.send(headers["authorization"].to_str().unwrap().to_owned()).unwrap(); Json(serde_json::json!({})) }
        }))
        .route("/api/v1/agents/environment", axum::routing::put(move |headers: HeaderMap| {
            let sender = sender.clone();
            async move { sender.send(headers["authorization"].to_str().unwrap().to_owned()).unwrap(); StatusCode::OK }
        }))
        .route("/oauth/token", post(|| async { Json(serde_json::json!({"access_token":"followup-access-secret", "refresh_token":"followup-refresh-secret", "expires_in":3600, "token_type":"Bearer"})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    harness.save_oauth(u64::MAX, "initial-refresh-secret");
    let config = lib::config::Config {
        api_url,
        auth_file: harness.path("authn.json"),
        ..Default::default()
    };
    let (api, _) = lib::kittengrid_api::from_registration(&config)
        .await
        .unwrap();
    assert_eq!(requests.try_recv().unwrap(), "Bearer initial-access-secret");
    harness.save_oauth(0, "initial-refresh-secret");
    api.agents_update_environment(lib::kittengrid_api::EnvironmentStatus::Running)
        .await
        .unwrap();
    assert_eq!(
        requests.try_recv().unwrap(),
        "Bearer followup-access-secret"
    );
    assert_eq!(
        harness.saved()["oauth"]["refresh_token"],
        "followup-refresh-secret"
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_agents_refresh_once_and_persist_rotated_credentials() {
    let mut harness = Harness::new().await;
    harness.save_oauth(0, "initial-refresh-secret");
    let mut a = harness.command();
    let mut b = harness.command();
    let (a, b) = tokio::join!(a.output(), b.output());
    assert!(!a.unwrap().status.success());
    assert!(!b.unwrap().status.success());
    let fields = harness.tokens.try_recv().unwrap();
    assert_eq!(fields["refresh_token"], "initial-refresh-secret");
    assert!(
        harness.tokens.try_recv().is_err(),
        "refresh must be serialized across processes"
    );
    for _ in 0..2 {
        assert_eq!(
            harness.registrations.try_recv().unwrap(),
            "Bearer rotated-access-secret"
        );
    }
    assert_eq!(
        harness.saved()["oauth"]["refresh_token"],
        "rotated-refresh-secret"
    );
}

#[tokio::test]
async fn refresh_failure_preserves_saved_credentials_and_sends_no_api_request() {
    let mut harness = Harness::new().await;
    harness.save_oauth(0, "revoked-secret");
    let before = std::fs::read(harness.path("authn.json")).unwrap();
    let output = harness.command().output().await.unwrap();
    assert!(!output.status.success());
    assert!(harness.registrations.try_recv().is_err());
    assert_eq!(std::fs::read(harness.path("authn.json")).unwrap(), before);
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(logs.contains("login again"));
    assert!(!logs.contains("revoked-secret"));
}

#[tokio::test]
async fn login_help_does_not_expose_environment_api_keys() {
    let harness = Harness::new().await;
    let output = harness
        .command()
        .args(["login", "--help"])
        .env("KITTENGRID_API_KEY", "explicit-api-secret")
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--no-browser"));
    assert!(help.contains("--manual"));
    assert!(!help.contains("--oauth-client-id"));
    assert!(!help.contains("--login-timeout-secs"));
    assert!(!help.contains("explicit-api-secret"));
}

#[tokio::test]
async fn explicit_api_key_bypasses_saved_oauth_and_refresh() {
    let mut harness = Harness::new().await;
    harness.save_oauth(0, "revoked-secret");
    let output = harness
        .command()
        .args(["--api-key", "explicit-api-secret"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        harness.registrations.try_recv().unwrap(),
        "Bearer explicit-api-secret"
    );
    assert!(harness.tokens.try_recv().is_err());
}
