use crate::service::{ServiceStream, Services};

use axum::{
    body::Body,
    extract::{
        rejection::PathRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use log::{debug, error, info};
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
pub async fn index(State(services): State<Arc<Services>>) -> impl IntoResponse {
    Json(services.to_json().await["services"].clone())
}

/// POST /services/:id/start
///
/// Description: Starts the service by its id (404  if not found)
#[axum::debug_handler]
pub async fn start(
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(services): State<Arc<Services>>,
) -> Response {
    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    match services.start_service(id).await {
        Ok(_) => ok_response(),
        Err(e) => error_response(Box::new(e)),
    }
}

/// POST /services/:id/stop
///
/// Description: Stops the service by its id (404  if not found)
#[axum::debug_handler]
pub async fn stop(
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(services): State<Arc<Services>>,
) -> Response {
    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    match services.stop_service(id).await {
        Ok(_) => ok_response(),
        Err(e) => error_response(Box::new(e)),
    }
}

/// GET /services/:id/stdout
///
/// Description: Connects to the stdout of the service by its id (404  if not found)
pub async fn stdout(
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(services): State<Arc<Services>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let id = match find_service(path, &services).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    ws.on_upgrade(move |socket| handle_socket(socket, addr, id, services, ServiceStream::Stdout))
        .into_response()
}

/// GET /services/:id/stderr
///
/// Description: Connects to the stderr of the service by its id (404  if not found)
pub async fn stderr(
    path: Result<Path<uuid::Uuid>, PathRejection>,
    State(services): State<Arc<Services>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
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

    debug!("Closing {address}...");
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

#[cfg(test)]
mod test {

    use crate::test_utils::*;
    use futures_util::StreamExt;

    use reqwest::StatusCode;
    // we will use tungstenite for websocket client impl (same library as what axum is using)
    use tokio_tungstenite::connect_async;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout_bad() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let ws_stream = connect_async(
            server_test.url_for_with_protocol("ws", "/services/_I_AM_NOT_A_SERVICE/stdout"),
        )
        .await;
        assert!(ws_stream.is_err());
        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let service_id = first_service_id(&server_test.services()).await;
        let ws_stream = match connect_async(
            server_test.url_for_with_protocol("ws", &format!("/services/{service_id}/stdout")),
        )
        .await
        {
            Ok((stream, _)) => stream,
            Err(_) => {
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
            .post(server_test.url_for(&format!("/services/{service_id}/stop")))
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
            .post(server_test.url_for("/services/_i_am_not_a_known_service_/stop"))
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
            .post(server_test.url_for("/services/f4d916f7-1fcd-4dcd-8d08-f66f82c0735b/stop"))
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
            .post(server_test.url_for(&format!("/services/{service_id}/stop")))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/services/{service_id}/stop")))
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
            .post(server_test.url_for(&format!("/services/{service_id}/start")))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for(&format!("/services/{service_id}/stop")))
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
            .get(server_test.url_for("/services"))
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
            .post(server_test.url_for(&format!("/services/{service_id}/start")))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .get(server_test.url_for("/services"))
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
