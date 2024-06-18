use crate::service::{ServiceStream, Services};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{Response, StatusCode},
    response::{IntoResponse, Json},
};
use log::{debug, error, info};
use serde_json::json;
use std::{borrow::Cow, sync::Arc};

use std::net::SocketAddr;

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;

/// POST /services/:service_name/start
///
/// Description: Starts the service by its name (404  if not found)
#[axum::debug_handler]
pub async fn start(
    Path(service_name): Path<String>,
    State(services): State<Arc<Services>>,
) -> impl IntoResponse {
    if services.fetch(&service_name).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "Service not found"
            })),
        );
    }

    match services.start_service(&service_name).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
              "status": "ok"
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("{}", e)
            })),
        ),
    }
}

/// POST /services/:service_name/stop
///
/// Description: Stops the service by its name (404  if not found)
#[axum::debug_handler]
pub async fn stop(
    Path(service_name): Path<String>,
    State(services): State<Arc<Services>>,
) -> impl IntoResponse {
    if services.fetch(&service_name).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "Service not found"
            })),
        );
    }

    match services.stop_service(&service_name).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
              "status": "ok"
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("{}", e)
            })),
        ),
    }
}

/// GET /services/:service_name/stdout
///
/// Description: Connects to the stdout of the service by its name (404  if not found)
pub async fn stdout(
    Path(service_name): Path<String>,
    State(services): State<Arc<Services>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    if services.fetch(&service_name).await.is_none() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"error": "Service not found"}).to_string(),
            ))
            .unwrap();
    }

    ws.on_upgrade(move |socket| {
        handle_socket(
            socket,
            addr,
            service_name.clone(),
            services,
            ServiceStream::Stdout,
        )
    })
}

/// GET /services/:service_name/stderr
///
/// Description: Connects to the stderr of the service by its name (404  if not found)
pub async fn stderr(
    Path(service_name): Path<String>,
    State(services): State<Arc<Services>>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    if services.fetch(&service_name).await.is_none() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"error": "Service not found"}).to_string(),
            ))
            .unwrap();
    }

    ws.on_upgrade(move |socket| {
        handle_socket(
            socket,
            addr,
            service_name.clone(),
            services,
            ServiceStream::Stderr,
        )
    })
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(
    mut socket: WebSocket,
    address: SocketAddr,
    service_name: String,
    services: Arc<crate::service::Services>,
    stream: ServiceStream,
) {
    let mut stream_channel_receiver =
        match services.subscribe_to_stream(&service_name, stream).await {
            Some(receiver) => receiver,
            None => {
                error!("Could not subscribe to {service_name} {stream} channel");
                return;
            }
        };

    while let Some(data) = stream_channel_receiver.recv().await {
        info!("Received data from {service_name}:");

        if socket.send(Message::Binary(data.to_vec())).await.is_err() {
            error!("Could not send data to {address}!");
            break;
        }
    }

    debug!("Closing {address}...");
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

#[cfg(test)]
mod test {
    use crate::test_utils::*;
    use futures_util::StreamExt;
    use log::debug;

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
        let ws_stream =
            match connect_async(server_test.url_for_with_protocol("ws", "/services/test/stdout"))
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
        let response = server_test
            .client
            .post(server_test.url_for("/services/test/stop"))
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

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        debug!("Stopping server");

        server_test.services().stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn double_stop() {
        initialize_tests();
        let server_test = ServerTest::new(true).await;
        let response = server_test
            .client
            .post(server_test.url_for("/services/test/stop"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for("/services/test/stop"))
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
        let response = server_test
            .client
            .post(server_test.url_for("/services/test/start"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response = server_test
            .client
            .post(server_test.url_for("/services/test/stop"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        server_test.services().stop().await.unwrap();
    }
}
