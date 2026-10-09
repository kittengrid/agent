use axum::{body::to_bytes, extract::Request, http::StatusCode, routing::any, Json, Router};
use lib::{
    config::Config,
    kittengrid_api::{from_registration, EnvironmentStatus, ServiceStatus},
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug)]
struct ApiRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    authorization: String,
    body: Value,
}

async fn api_server() -> (
    String,
    mpsc::UnboundedReceiver<ApiRequest>,
    tokio::task::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let app = Router::new().fallback(any(move |request: Request| {
        let sender = sender.clone();
        async move {
            let method = request.method().to_string();
            let path = request.uri().path().to_string();
            let url = url::Url::parse(&format!("http://localhost{}", request.uri())).unwrap();
            let query = url.query_pairs().into_owned().collect();
            let authorization = request.headers()["Authorization"]
                .to_str()
                .unwrap()
                .to_string();
            let bytes = to_bytes(request.into_body(), 8192).await.unwrap();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            let response = match path.as_str() {
                "/api/agents/register" => json!({"start_services": true, "start_terminal": false}),
                "/api/peers" => json!([]),
                "/api/peers/service" => json!({"public_url": "https://web.example.test"}),
                "/api/peers/endpoint" => json!({
                    "public_url": "192.0.2.1:51820",
                    "address": "10.121.0.1",
                    "public_key": "public-key",
                    "network": "10.121.0.0/16"
                }),
                _ => json!({}),
            };
            sender
                .send(ApiRequest {
                    method,
                    path,
                    query,
                    authorization,
                    body,
                })
                .unwrap();
            (StatusCode::OK, Json(response))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, receiver, server)
}

#[tokio::test]
async fn environment_identity_is_used_for_registration_and_all_followup_requests() {
    exercise_api(true).await;
}

#[tokio::test]
async fn legacy_ci_identity_and_metadata_are_preserved() {
    exercise_api(false).await;
}

async fn exercise_api(environment_mode: bool) {
    let (api_url, mut requests, server) = api_server().await;
    let config = Config {
        api_url,
        api_key: "organization-key".to_string(),
        environment_id: if environment_mode { "7ia79uwdfc4j" } else { "" }.to_string(),
        // Even if stale CI metadata is configured, environment mode must never
        // send mixed identity parameters to Rails.
        vcs_provider: "github".to_string(),
        project_vcs_path: "org/repository".to_string(),
        pull_request_vcs_id: "42".to_string(),
        workflow_run_id: if environment_mode { "" } else { "123" }.to_string(),
        last_commit_sha: if environment_mode { "" } else { "abc123" }.to_string(),
        ..Config::default()
    };
    let expected_identity: HashMap<String, String> = if environment_mode {
        HashMap::from([("environment_id".to_string(), config.environment_id.clone())])
    } else {
        HashMap::from([
            ("vcs_provider".to_string(), config.vcs_provider.clone()),
            (
                "project_vcs_path".to_string(),
                config.project_vcs_path.clone(),
            ),
            (
                "pull_request_vcs_id".to_string(),
                config.pull_request_vcs_id.clone(),
            ),
        ])
    };

    let (mut api, options) = from_registration(&config).await.unwrap();
    assert!(options.start_services);
    assert!(!options.start_terminal);
    api.set_startup_options(options.start_services, options.start_terminal);
    let request = requests.recv().await.unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/agents/register");
    assert_eq!(request.authorization, "Bearer organization-key");
    let mut expected_registration = serde_json::to_value(&expected_identity).unwrap();
    if !environment_mode {
        expected_registration["workflow_run_id"] = json!("123");
    }
    assert_eq!(request.body, expected_registration);

    api.agents_update_environment(EnvironmentStatus::Running)
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.path,
        if environment_mode {
            "/api/agents/environment"
        } else {
            "/api/agents/pull_request"
        }
    );
    assert_eq!(request.query, expected_identity);
    assert_eq!(request.authorization, "Bearer organization-key");
    assert_eq!(request.body, json!({"status": "running"}));

    let id = Uuid::new_v4();
    api.agents_create_service(id, "web".to_string())
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert_eq!(request.path, "/api/agents/service");
    assert_eq!(request.query, expected_identity);
    let mut expected_service = json!({"id": id.to_string(), "name": "web", "status": "created"});
    if !environment_mode {
        expected_service["sha"] = json!("abc123");
    }
    assert_eq!(request.body, expected_service);

    assert!(api.peers_create(3000).await.unwrap().is_empty());
    let request = requests.recv().await.unwrap();
    assert_eq!(request.path, "/api/peers");
    assert_eq!(request.query, expected_identity);
    assert_eq!(request.body, json!({"bind_port": 3000}));

    let endpoint = api
        .peers_get_endpoint("10.121.0.0/16".to_string())
        .await
        .unwrap();
    assert_eq!(endpoint.public_url(), "192.0.2.1:51820");
    let request = requests.recv().await.unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/peers/endpoint");
    let mut endpoint_query = expected_identity.clone();
    endpoint_query.insert("cidr".to_string(), "10.121.0.0/16".to_string());
    assert_eq!(request.query, endpoint_query);

    let url = api
        .peers_create_service(id, "web", 3000, None, None, false)
        .await
        .unwrap();
    assert_eq!(url, "https://web.example.test");
    let request = requests.recv().await.unwrap();
    assert_eq!(request.path, "/api/peers/service");
    assert_eq!(request.query, expected_identity);
    assert_eq!(request.body["publish"], true);

    api.services_update_status(id, Some(ServiceStatus::Running), None, None)
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert_eq!(request.method, "PUT");
    assert_eq!(request.path, format!("/api/services/{id}"));
    assert_eq!(request.query, expected_identity);
    assert_eq!(request.body, json!({"status": "running"}));
    server.abort();
}

#[tokio::test]
async fn environment_previews_can_still_supply_workflow_and_commit_metadata() {
    let (api_url, mut requests, server) = api_server().await;
    let config = Config {
        api_url,
        api_key: "organization-key".to_string(),
        environment_id: "previewenv01".to_string(),
        workflow_run_id: "456".to_string(),
        last_commit_sha: "abc123".to_string(),
        ..Config::default()
    };
    let (api, _) = from_registration(&config).await.unwrap();
    assert_eq!(
        requests.recv().await.unwrap().body,
        json!({"environment_id": "previewenv01", "workflow_run_id": "456"})
    );
    api.agents_create_service(Uuid::new_v4(), "web".to_string())
        .await
        .unwrap();
    assert_eq!(requests.recv().await.unwrap().body["sha"], "abc123");
    server.abort();
}
