use axum::{body::to_bytes, extract::Request, http::StatusCode, routing::any, Json, Router};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;
const REGISTRATION: &str = "5c7bbfe8-a6b1-4f48-aeb8-72d25dcc2399";

#[derive(Debug, Clone)]
struct Event {
    path: String,
    query: HashMap<String, String>,
    body: Value,
}
#[derive(Default)]
struct Api {
    events: Mutex<Vec<Event>>,
    creations: AtomicUsize,
    port: AtomicU16,
    running: AtomicBool,
    missing: AtomicBool,
}
struct Harness {
    directory: tempfile::TempDir,
    api_url: String,
    api: Arc<Api>,
    task: tokio::task::JoinHandle<()>,
    target: tokio::net::TcpListener,
}
impl Harness {
    async fn new() -> Self {
        let api = Arc::new(Api::default());
        let state = api.clone();
        let app=Router::new().route("/oauth/token", axum::routing::post(|| async { Json(json!({"access_token":"kttngrd_oauth_at_test", "refresh_token":"kttngrd_oauth_rt_test", "token_type":"Bearer", "expires_in":3600})) })).fallback(any(move |request: Request| { let state=state.clone(); async move {
            let path=request.uri().path().to_owned();
            let url=url::Url::parse(&format!("http://localhost{}", request.uri())).unwrap();
            let query: HashMap<String,String>=url.query_pairs().into_owned().collect();
            let bytes=to_bytes(request.into_body(),8192).await.unwrap();
            let body: Value=if bytes.is_empty() {Value::Null} else {serde_json::from_slice(&bytes).unwrap()};
            state.events.lock().await.push(Event{path:path.clone(),query:query.clone(),body:body.clone()});
            let response=match path.as_str() {
                "/api/v1/projects/resolve" | "/api/v1/projects/project12345" => json!({"id":"project12345","name":"app","vcs_provider":"github","vcs_path":"team/app"}),
                "/api/v1/projects/project12345/development-environments" => {
                    let number=state.creations.fetch_add(1,Ordering::SeqCst)+1;
                    json!({"id":format!("env{number:09}"),"project_id":"project12345","name":body["name"].as_str().unwrap_or("user-random"),"status":"created","created":true,"name_generated":body.get("name").is_none()})
                },
                path if path.starts_with("/api/v1/projects/project12345/development-environments/") => {
                    if state.missing.swap(false,Ordering::SeqCst) { return (StatusCode::NOT_FOUND,Json(json!({}))); }
                    json!({"id":path.rsplit('/').next().unwrap(),"project_id":"project12345","name":"user-random","status":"sleeping","created":false,"name_generated":true})
                },
                "/api/v1/agents/register" => json!({"registration_id":REGISTRATION,"start_services":true,"start_terminal":true}),
                "/api/v1/agents/heartbeat" => json!({}),
                "/api/v1/agents/environment" => { if body["status"]=="running" {state.running.store(true,Ordering::SeqCst);} json!({}) },
                "/api/v1/agents/service" => json!({}),
                "/api/v1/peers" => { state.port.store(body["bind_port"].as_u64().unwrap() as u16,Ordering::SeqCst); json!([]) },
                "/api/v1/peers/service" => json!({"public_url":"https://web.example.test"}),
                _ => return (StatusCode::NOT_FOUND,Json(json!({}))),
            };
            (StatusCode::OK,Json(response))
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        // Even a CI file asking for service startup must not opt dev into it.
        let yaml = json!({"start_services":true,"start_terminal":true,"services":[{"name":"web","port":target.local_addr().unwrap().port(),"cmd":"/bin/sh","args":["-c","touch SHOULD_NOT_EXIST"]}]});
        std::fs::write(
            directory.path().join("kittengrid.yaml"),
            serde_yaml::to_string(&yaml).unwrap(),
        )
        .unwrap();
        Self {
            directory,
            api_url,
            api,
            task,
            target,
        }
    }
    fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.directory.path())
            .env("XDG_CONFIG_HOME", self.directory.path())
            .env("KITTENGRID_API_KEY", "test-secret")
            .args([
                "--api-url",
                &self.api_url,
                "--no-global-config",
                "--bind-address",
                "127.0.0.1",
                "--bind-port",
                "0",
                "dev",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
    async fn start(&self, args: &[&str]) -> tokio::process::Child {
        self.api.running.store(false, Ordering::SeqCst);
        let child = self.command().args(args).spawn().unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            while !self.api.running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("dev must reach serving mode");
        child
    }
    async fn stop(&self, child: tokio::process::Child) -> String {
        let port = self.api.port.load(Ordering::SeqCst);
        let response = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/sys/shutdown"))
            .json(&json!({"message":"test stop","registration_id":REGISTRATION}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let output = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output())
            .await
            .expect("dev should stop gracefully")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("test-secret"));
        assert!(!self.directory.path().join("SHOULD_NOT_EXIST").exists());
        String::from_utf8(output.stdout).unwrap()
    }
    fn git(&self, args: &[&str]) {
        assert!(std::process::Command::new("git")
            .current_dir(self.directory.path())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn dev_exposes_without_spawning_and_keeps_the_existing_process_alive() {
    let harness = Harness::new().await;
    let mut child = harness.start(&["--project-id", "project12345"]).await;
    assert!(child.try_wait().unwrap().is_none());
    let port = harness.api.port.load(Ordering::SeqCst);
    let client = reqwest::Client::new();
    let list: Value = client
        .get(format!("http://127.0.0.1:{port}/public/services"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = list[0]["id"].as_str().unwrap();
    for action in ["start", "stop"] {
        let response = client
            .post(format!(
                "http://127.0.0.1:{port}/public/services/{id}/{action}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
    let output = harness.stop(child).await;
    assert!(output.contains("Exposing services"));
    assert!(
        tokio::net::TcpStream::connect(harness.target.local_addr().unwrap())
            .await
            .is_ok()
    );
    let events = harness.api.events.lock().await;
    assert_eq!(
        events
            .iter()
            .find(|e| e.path == "/api/v1/agents/service")
            .unwrap()
            .body["status"],
        "running"
    );
    assert_eq!(
        events
            .iter()
            .find(|e| e.path == "/api/v1/peers/service")
            .unwrap()
            .body["publish"],
        true
    );
    assert!(events.iter().any(|e| e.path == "/api/v1/agents/heartbeat"));
    assert_eq!(events.last().unwrap().body["status"], "sleeping");
    assert_eq!(
        events
            .iter()
            .find(|e| e.path == "/api/v1/agents/register")
            .unwrap()
            .body["inactivity_timeout"],
        0
    );
    for event in events
        .iter()
        .filter(|e| e.path.starts_with("/api/v1/agents/") && e.path != "/api/v1/agents/register")
    {
        assert_eq!(event.query["registration_id"], REGISTRATION);
    }
}

#[tokio::test]
async fn workspace_reuses_per_checkout_and_branch_and_recovers_after_pruning() {
    let harness = Harness::new().await;
    harness.git(&["init", "-q"]);
    harness.git(&["remote", "add", "origin", "git@github.com:team/app.git"]);
    let child = harness.start(&[]).await;
    harness.stop(child).await;
    let child = harness.start(&[]).await;
    harness.stop(child).await;
    assert_eq!(harness.api.creations.load(Ordering::SeqCst), 1);
    harness.git(&["symbolic-ref", "HEAD", "refs/heads/feature/second"]);
    let child = harness.start(&[]).await;
    harness.stop(child).await;
    assert_eq!(harness.api.creations.load(Ordering::SeqCst), 2);
    harness.api.missing.store(true, Ordering::SeqCst);
    let child = harness.start(&[]).await;
    harness.stop(child).await;
    assert_eq!(harness.api.creations.load(Ordering::SeqCst), 3);
    let state: Value = serde_json::from_slice(
        &std::fs::read(
            harness
                .directory
                .path()
                .join("kittengrid-agent/workspaces.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(state["workspaces"].as_object().unwrap().len(), 2);
    assert!(!harness.directory.path().join(".kittengrid").exists());
}

#[tokio::test]
async fn explicit_names_are_forwarded_and_missing_credentials_do_not_prompt_in_ci() {
    let harness = Harness::new().await;
    let child = harness
        .start(&["--project-id", "project12345", "--name", "my-feature"])
        .await;
    harness.stop(child).await;
    assert!(harness
        .api
        .events
        .lock()
        .await
        .iter()
        .any(|e| e.body.get("name") == Some(&json!("my-feature"))));
    let output = harness
        .command()
        .env_remove("KITTENGRID_API_KEY")
        .args(["--project-id", "project12345"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Login required"));
}

#[tokio::test]
async fn dev_automatically_logs_in_in_manual_mode_and_continues_startup() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let harness = Harness::new().await;
    let mut child = harness
        .command()
        .env_remove("KITTENGRID_API_KEY")
        .args(["--project-id", "project12345", "--manual"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let url = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let line = stdout
                .next_line()
                .await
                .unwrap()
                .expect("authorization URL");
            if line.starts_with("http://") {
                break url::Url::parse(&line).unwrap();
            }
        }
    })
    .await
    .unwrap();
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", json!({"code":"test-code", "state":params["state"]})).as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while !harness.api.running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    harness.stop(child).await;
    let auth: Value = serde_json::from_slice(
        &std::fs::read(harness.directory.path().join("kittengrid-agent/authn.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(auth["oauth"]["access_token"], "kttngrd_oauth_at_test");
    assert_eq!(harness.api.creations.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn ctrl_c_stops_the_agent_but_not_external_services() {
    let harness = Harness::new().await;
    let child = harness.start(&["--project-id", "project12345"]).await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if harness
                .api
                .events
                .lock()
                .await
                .iter()
                .any(|e| e.path == "/api/v1/agents/heartbeat")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        unsafe { libc::kill(child.id().unwrap() as i32, libc::SIGINT) },
        0
    );
    let output = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert!(
        tokio::net::TcpStream::connect(harness.target.local_addr().unwrap())
            .await
            .is_ok()
    );
    assert_eq!(
        harness.api.events.lock().await.last().unwrap().body["status"],
        "sleeping"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn explicit_start_services_manages_and_stops_only_its_child() {
    let harness = Harness::new().await;
    let config = json!({"services":[{"name":"web", "port":harness.target.local_addr().unwrap().port(), "cmd":"/bin/sh", "args":["-c", "echo $$ > MANAGED_PID; exec sleep 3600"]}]});
    std::fs::write(
        harness.directory.path().join("kittengrid.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();
    let child = harness
        .start(&["--project-id", "project12345", "--start-services"])
        .await;
    let pid = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(harness.directory.path().join("MANAGED_PID")) {
                if !pid.trim().is_empty() {
                    break pid.trim().to_owned();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
    let output = harness.stop(child).await;
    assert!(output.contains("Managing configured services"));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(
        tokio::net::TcpStream::connect(harness.target.local_addr().unwrap())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn dev_is_the_public_interface_and_start_services_is_a_bare_opt_in() {
    let harness = Harness::new().await;
    let output = harness
        .command()
        .args(["--start-services", "--help"])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--manual"));
    assert!(help.contains("--no-browser"));
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_kittengrid-agent"))
        .args(["environment", "create"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}
