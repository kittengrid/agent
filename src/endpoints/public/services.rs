use crate::service::{ServiceStream, Services};
use crate::AxumState;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::RequestPartsExt;

use axum::{
    body::Body,
    extract::{
        rejection::PathRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
        FromRequestParts, Path, Query, State,
    },
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Json, Response},
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use log::{debug, error, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{borrow::Cow, sync::Arc};

use std::net::SocketAddr;

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;
/// GET /services
///
/// Description: Shows all services
///
/// Response example:
/// [
///    {
///       "description" : {
///          "args" : [
///             "-b",
///             "4",
///             "-s",
///             "1"
///          ],
///          "cmd" : "target/debug/log-generator",
///          "env" : {},
///          "health_check" : null,
///          "name" : "test",
///          "port" : 8080
///       },
///       "id" : "bbfc62db-eae5-4d8f-ae3a-20e267ac4e76",
///       "status" : "Stopped"
///    }
/// ]
pub async fn index(_claims: Claims, State(state): State<Arc<AxumState>>) -> impl IntoResponse {
    let services = state.services.clone();
    Json(services.to_json().await["services"].clone())
}

/// POST /public/services/:id/start
///
/// Description: Starts the service by its id (404  if not found)
pub async fn start(
    _claims: Claims,
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(state): State<Arc<AxumState>>,
) -> Response {
    let services = state.services.clone();
    let kittengrid_api = state.kittengrid_api.clone();
    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    match services.start_service(id, kittengrid_api).await {
        Ok(_) => ok_response(),
        Err(e) => error_response(Box::new(e)),
    }
}

/// POST /public/services/:id/stop
///
/// Description: Stops the service by its id (404  if not found)
#[axum::debug_handler]
pub async fn stop(
    _claims: Claims,
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(state): State<Arc<AxumState>>,
) -> Response {
    let services = state.services.clone();

    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    match services.stop_service(id).await {
        Ok(_) => ok_response(),
        Err(e) => error_response(Box::new(e)),
    }
}

/// GET /public/services/:id/stdout
///
/// Description: Connects to the stdout of the service by its id (404  if not found)
pub async fn stdout(
    Query(params): Query<OutputStreamParams>,
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(state): State<Arc<AxumState>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let services = state.services.clone();

    match validate_token(&params.token) {
        Ok(_) => (),
        Err(response) => return response.into_response(),
    }

    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    ws.on_upgrade(move |socket| handle_socket(socket, addr, id, services, ServiceStream::Stdout))
        .into_response()
}

/// GET /public/services/:id/combined_output
///
/// Description: Connects to the stdout and stderr of the service by its id (404  if not found)
/// it will stream the stdout and stderr to the client using a json structure of:
/// {
///     "type": "stdout" | "stderr",
///     "data": "data"
///     "timestamp": "2023-01-01T00:00:00Z"
/// }
pub async fn combined_output(
    Query(params): Query<OutputStreamParams>,
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(state): State<Arc<AxumState>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let services = state.services.clone();

    match validate_token(&params.token) {
        Ok(_) => (),
        Err(response) => return response.into_response(),
    }

    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    ws.on_upgrade(move |socket| handle_socket_combined(socket, addr, id, services))
        .into_response()
}
#[derive(Debug, Deserialize)]
pub struct OutputStreamParams {
    pub token: String,
}

/// GET /public/services/:id/stderr
///
/// Description: Connects to the stderr of the service by its id (404  if not found)
pub async fn stderr(
    Query(params): Query<OutputStreamParams>,
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(state): State<Arc<AxumState>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let services = state.services.clone();
    match validate_token(&params.token) {
        Ok(_) => (),
        Err(response) => return response.into_response(),
    }
    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    ws.on_upgrade(move |socket| handle_socket(socket, addr, id, services, ServiceStream::Stderr))
        .into_response()
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(
    mut socket: WebSocket,
    address: SocketAddr,
    id: uuid::Uuid,
    services: Arc<crate::service::Services>,
    stream: ServiceStream,
) {
    let mut stream_channel_receiver = match services.subscribe_to_stream(id, stream).await {
        Some(receiver) => receiver,
        None => {
            error!("Could not subscribe to {id} {stream} channel");
            return;
        }
    };

    while let Some(data) = stream_channel_receiver.recv().await {
        info!("Received data from {id}:");

        if socket.send(Message::Binary(data.to_vec())).await.is_err() {
            error!("Could not send data to {address}!");
            break;
        }
    }

    info!("Websocket disconnected, dropping internal stream.");
    if let Err(e) = services
        .unsubscribe_from_stream(id, stream, stream_channel_receiver)
        .await
    {
        error!("Could not unsubscribe from {id} {stream} channel! {e}");
    }

    if let Err(e) = socket
        .send(Message::Close(Some(CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: Cow::from("Closed by server"),
        })))
        .await
    {
        error!("Could not send close to {address}! {e}");
    };
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket_combined(
    mut socket: WebSocket,
    address: SocketAddr,
    id: uuid::Uuid,
    services: Arc<crate::service::Services>,
) {
    let mut stdout_stream_channel_receiver = match services
        .subscribe_to_stream(id, ServiceStream::Stdout)
        .await
    {
        Some(receiver) => receiver,
        None => {
            error!("Could not subscribe to {id} stdout channel");
            return;
        }
    };

    let mut stderr_stream_channel_receiver = match services
        .subscribe_to_stream(id, ServiceStream::Stderr)
        .await
    {
        Some(receiver) => receiver,
        None => {
            error!("Could not subscribe to {id} stderr channel");
            return;
        }
    };

    while let (Some(data), source) = tokio::select! {
        data = stdout_stream_channel_receiver.recv() => (data, ServiceStream::Stdout),
        data = stderr_stream_channel_receiver.recv() => (data, ServiceStream::Stderr),
    } {
        debug!("Received data from {id}:");
        let data = create_stream_output_json(&source, &data);

        if socket.send(Message::Text(data.to_string())).await.is_err() {
            error!("Could not send data to {address}!");
            break;
        }
    }

    info!("Websocket disconnected, dropping internal stream.");
    if let Err(e) = services
        .unsubscribe_from_stream(id, ServiceStream::Stdout, stdout_stream_channel_receiver)
        .await
    {
        error!("Could not unsubscribe from {id} stdout channel! {e}");
    }

    if let Err(e) = services
        .unsubscribe_from_stream(id, ServiceStream::Stderr, stderr_stream_channel_receiver)
        .await
    {
        error!("Could not unsubscribe from {id} stderr channel! {e}");
    }

    if let Err(e) = socket
        .send(Message::Close(Some(CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: Cow::from("Closed by server"),
        })))
        .await
    {
        error!("Could not send close to {address}! {e}");
    };
}

// Process the path and return the service id if ok, or a response error if not
async fn find_service(
    path: Result<Path<uuid::Uuid>, PathRejection>,
    services: &Arc<Services>,
) -> Result<uuid::Uuid, Response> {
    let id = match path {
        Ok(id) => id.0,
        Err(_) => {
            return Err(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Body::from(json!({"error": "Invalid UUID"}).to_string()))
                .unwrap());
        }
    };

    if services.fetch(id).await.is_none() {
        return Err(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"error": "Service not found"}).to_string(),
            ))
            .unwrap());
    }

    Ok(id)
}

fn ok_response() -> Response {
    (StatusCode::OK, Json(json!({"status": "ok"}))).into_response()
}

fn error_response(err: Box<dyn std::error::Error>) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": err.to_string()})),
    )
        .into_response()
}

fn create_stream_output_json(
    stream_type: &ServiceStream,
    data: &bytes::Bytes,
) -> serde_json::Value {
    json!({
        "type": stream_type.to_string(),
        "data": serde_json::to_vec(&data.to_vec()).unwrap(),
        "timestamp": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    })
}

// For auth using jwt

static KEY: Lazy<DecodingKey> = Lazy::new(|| {
    let secret = crate::config::get_config().clone().api_key;
    DecodingKey::from_secret(secret.as_bytes())
});

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub bearer_id: String,
    pub bearer_type: String,
    pub exp: u64,
}

#[async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Sized,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract the token from the authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::InvalidToken)?;

        validate_token(bearer.token())
    }
}

pub fn validate_token(token: &str) -> Result<Claims, AuthError> {
    let token_data = decode::<Claims>(token, &KEY, &Validation::default())
        .map_err(|_| AuthError::InvalidToken)?;

    let current_time_in_seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(n) => n.as_secs(),
        Err(_) => return Err(AuthError::InvalidToken),
    };

    if current_time_in_seconds > token_data.claims.exp {
        return Err(AuthError::ExpiredToken);
    }

    Ok(token_data.claims)
}

pub enum AuthError {
    ExpiredToken,
    InvalidToken,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AuthError::ExpiredToken => (StatusCode::FORBIDDEN, "Token expired"),
            AuthError::InvalidToken => (StatusCode::FORBIDDEN, "Invalid token"),
        };
        let body = Json(json!({
            "error": error_message,
        }));
        (status, body).into_response()
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::*;

    use futures_util::StreamExt;

    use reqwest::StatusCode;
    // we will use tungstenite for websocket client impl (same library as what axum is using)
    use tokio_tungstenite::connect_async;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn unauthenticated_request() {
        initialize_tests();
        let server_test = ServerTest::new(false).await;
        let response = server_test
            .client
            .get(server_test.url_for("/public/services"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn expired_token_request() {
        initialize_tests();
        let server_test = ServerTest::new(false).await;
        let response = server_test
            .client
            .get(server_test.url_for("/public/services"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.invalid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn valid_token_request() {
        initialize_tests();
        let server_test = ServerTest::new(false).await;
        let response = server_test
            .client
            .get(server_test.url_for("/public/services"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout_bad() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let ws_stream = connect_async(
            server_test.url_for_with_protocol("ws", "/public/services/_I_AM_NOT_A_SERVICE/stdout"),
        )
        .await;
        assert!(ws_stream.is_err());
        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout_invalid_token() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let ws_stream = connect_async(server_test.url_for_with_protocol(
            "ws",
            &format!("/public/services/{service_id}/stdout?token=1234"),
        ))
        .await;
        assert!(ws_stream.is_err());
        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let ws_stream = match connect_async(server_test.url_for_with_protocol(
            "ws",
            &format!(
                "/public/services/{service_id}/stdout?token={}",
                server_test.valid_token()
            ),
        ))
        .await
        {
            Ok((stream, _)) => stream,
            Err(err) => {
                eprintln!("Error: {:?}", err);

                panic!("Could not connect to server");
            }
        };

        let (_, mut receiver) = ws_stream.split();

        assert!(receiver.next().await.is_some());
        assert!(receiver.next().await.is_some());

        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_combined_output() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let ws_stream = match connect_async(server_test.url_for_with_protocol(
            "ws",
            &format!(
                "/public/services/{service_id}/combined_output?token={}",
                server_test.valid_token()
            ),
        ))
        .await
        {
            Ok((stream, _)) => stream,
            Err(err) => {
                eprintln!("Error: {:?}", err);

                panic!("Could not connect to server");
            }
        };

        let (_, mut receiver) = ws_stream.split();
        let data_received = receiver
            .next()
            .await
            .unwrap()
            .expect("Failed to receive data");

        // Check if the data is in the expected format
        let data: serde_json::Value = serde_json::from_slice(&data_received.into_data()).unwrap();

        assert!(data["type"] == "stdout" || data["type"] == "stderr");
        assert!(data["data"].is_array());
        assert!(data["timestamp"].is_number());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_stderr() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let ws_stream = match connect_async(server_test.url_for_with_protocol(
            "ws",
            &format!(
                "/public/services/{service_id}/stderr?token={}",
                server_test.valid_token()
            ),
        ))
        .await
        {
            Ok((stream, _)) => stream,
            Err(err) => {
                eprintln!("Error: {:?}", err);

                panic!("Could not connect to server");
            }
        };

        let (_, mut receiver) = ws_stream.split();

        assert!(receiver.next().await.is_some());
        assert!(receiver.next().await.is_some());

        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn valid_stop() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/stop")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn invalid_stop() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let response = server_test
            .client
            .post(server_test.url_for("/public/services/_i_am_not_a_known_service_/stop"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );

        println!("{}", response.text().await.unwrap());

        let response = server_test
            .client
            .post(server_test.url_for("/public/services/f4d916f7-1fcd-4dcd-8d08-f66f82c0735b/stop"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn double_stop() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/stop")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/stop")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn start() {
        initialize_tests();
        let server_test = ServerTest::new(false).await;
        let service_id = first_service_id(&server_test.services()).await;
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/start")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/stop")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn index() {
        initialize_tests();
        let server_test = ServerTest::new(false).await;
        let response = server_test
            .client
            .get(server_test.url_for("/public/services"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );

        let data = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(data[0]["description"]["name"].as_str().unwrap(), "test");
        assert_eq!(data[0]["status"].as_str().unwrap(), "Stopped");

        let service_id = first_service_id(&server_test.services()).await;
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/public/services/{service_id}/start")))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .get(server_test.url_for("/public/services"))
            .header(
                "Authorization",
                format!("Bearer {}", server_test.valid_token()),
            )
            .send()
            .await
            .unwrap();
        let data = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(data[0]["description"]["name"].as_str().unwrap(), "test");
        assert_eq!(data[0]["status"].as_str().unwrap(), "Running");

        server_test.services().stop().await.unwrap();
    }

    async fn first_service_id(services: &crate::service::Services) -> uuid::Uuid {
        *services.descriptions().await.keys().next().unwrap()
    }
}
