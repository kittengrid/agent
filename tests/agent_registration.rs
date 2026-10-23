use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{post, put},
    Json, Router,
};
use lib::{
    config::Config,
    kittengrid_agent::KittengridAgent,
    kittengrid_api::{self, EnvironmentStatus, KittengridApiError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const REGISTRATION: &str = "5c7bbfe8-a6b1-4f48-aeb8-72d25dcc2399";
type Requests = Arc<Mutex<Vec<HashMap<String, String>>>>;

async fn server() -> (Config, Requests, tokio::task::JoinHandle<()>) {
    let requests = Arc::new(Mutex::new(Vec::<HashMap<String, String>>::new()));
    let app = Router::new()
        .route(
            "/api/v1/agents/register",
            post(|| async { Json(serde_json::json!({"registration_id":REGISTRATION})) }),
        )
        .route("/api/v1/agents/heartbeat", post(replaced))
        .route("/api/v1/agents/environment", put(replaced))
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        api_key: "test-key".into(),
        environment_id: "environment".into(),
        bind_address: "127.0.0.1".into(),
        ..Default::default()
    };
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (config, requests, handle)
}

async fn replaced(
    State(requests): State<Requests>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    requests.lock().await.push(query);
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"code":"agent_replaced"})),
    )
}

#[tokio::test]
async fn replaced_responses_cancel_all_clones_and_prevent_further_requests() {
    let (config, requests, server) = server().await;
    let (api, _) = kittengrid_api::from_registration(&config).await.unwrap();
    let clone = api.clone();
    assert_eq!(api.registration_id().to_string(), REGISTRATION);
    assert!(matches!(
        api.agents_update_environment(EnvironmentStatus::Running)
            .await,
        Err(KittengridApiError::AgentReplaced)
    ));
    assert!(clone.replacement_token().is_cancelled());
    assert!(matches!(
        clone.heartbeat().await,
        Err(KittengridApiError::AgentReplaced)
    ));
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["registration_id"], REGISTRATION);
    assert_eq!(requests[0]["environment_id"], "environment");
    server.abort();
}

#[tokio::test]
async fn an_idle_replaced_agent_shuts_down_on_its_heartbeat() {
    let (config, requests, server) = server().await;
    let mut agent = KittengridAgent::new(config);
    agent.register().await.unwrap();
    let listener = agent.bind().await;
    let address = listener.local_addr().unwrap();
    tokio::time::timeout(Duration::from_secs(3), agent.wait(listener))
        .await
        .expect("replaced agent should exit promptly");
    assert_eq!(requests.lock().await[0]["registration_id"], REGISTRATION);
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    server.abort();
}
