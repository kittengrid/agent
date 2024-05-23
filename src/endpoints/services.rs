use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::Path,
    response::IntoResponse,
};
use log::debug;
use std::borrow::Cow;

use std::net::SocketAddr;

//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::CloseFrame;

//allows to split the websocket stream into separate TX and RX branches
use futures::{sink::SinkExt, stream::StreamExt};

// GET /services/:service_name/stdout
//
// Description: Returns the cutiest Http response
pub async fn stdout(
    Path(service_name): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    println!("{service_name} connected.");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| handle_socket(socket, addr, service_name.clone()))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(mut socket: WebSocket, who: SocketAddr, service_name: String) {
    let stdout = match crate::stdout_receiver_for_service(&service_name).await {
        None => {
            debug!("Service {service_name} not found.");
            return;
        }
        Some(receiver) => receiver,
    };

    // send a ping (unsupported by some browsers) just to kick things off and get a response
    if socket.send(Message::Ping(vec![1, 2, 3])).await.is_ok() {
        println!("Pinged {who}...");
    } else {
        println!("Could not send ping {who}!");
        // no Error here since the only thing we can do is to close the connection.
        // If we can not send messages, there is no way to salvage the statemachine anyway.
        return;
    }

    // By splitting socket we can send and receive at the same time. In this example we will send
    // unsolicited messages to client based on some sort of server's internal event (i.e .timer).
    let (mut sender, _) = socket.split();
    let mut receiver = stdout.subscribe().await;

    // Spawn a task that will push several messages to the client (does not matter what client does)
    tokio::spawn(async move {
        while let Some(data) = receiver.recv().await {
            let data = std::str::from_utf8(&data).unwrap();

            if sender.send(Message::Text(data.to_string())).await.is_err() {
                return;
            }
        }

        println!("Sending close to {who}...");
        if let Err(e) = sender
            .send(Message::Close(Some(CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: Cow::from("Goodbye"),
            })))
            .await
        {
            println!("Could not send close to {who}! {e}");
        };
    });

    // returning from the handler closes the websocket connection
    println!("Websocket context {who} destroyed");
}

#[cfg(test)]
mod test {
    use crate::test_utils::*;
    use futures_util::{SinkExt, StreamExt};
    use std::borrow::Cow;
    use std::ops::ControlFlow;

    // we will use tungstenite for websocket client impl (same library as what axum is using)
    use tokio_tungstenite::{
        connect_async,
        tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message},
    };
    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn stdout() {
        let server_test = ServerTest::new().await;
        let who = 10;
        let ws_stream =
            match connect_async(server_test.url_for_with_protocol("ws", "/services/test/stdout"))
                .await
            {
                Ok((stream, response)) => {
                    println!("Handshake for client {who} has been completed");
                    // This will be the HTTP response, same as with server this is the last moment we
                    // can still access HTTP stuff.
                    println!("Server response was {response:?}");
                    stream
                }
                Err(e) => {
                    println!("WebSocket handshake for client {who} failed with {e}!");
                    return;
                }
            };

        let (mut sender, mut receiver) = ws_stream.split();

        //we can ping the server for start
        sender
            .send(Message::Ping("Hello, Server!".into()))
            .await
            .expect("Can not send!");

        //spawn an async sender to push some more messages into the server
        let mut send_task = tokio::spawn(async move {
            for i in 1..30 {
                // In any websocket error, break loop.
                if sender
                    .send(Message::Text(format!("Message number {i}...")))
                    .await
                    .is_err()
                {
                    //just as with server, if send fails there is nothing we can do but exit.
                    return;
                }

                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }

            // When we are done we may want our client to close connection cleanly.
            println!("Sending close to {who}...");
            if let Err(e) = sender
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: Cow::from("Goodbye"),
                })))
                .await
            {
                println!("Could not send Close due to {e:?}, probably it is ok?");
            };
        });

        //receiver just prints whatever it gets
        let mut recv_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = receiver.next().await {
                // print message and break if instructed to do so
                if process_message(msg, who).is_break() {
                    break;
                }
            }
        });

        //wait for either task to finish and kill the other task
        tokio::select! {
            _ = (&mut send_task) => {
                recv_task.abort();
            },
            _ = (&mut recv_task) => {
                send_task.abort();
            }
        }
    }

    /// Function to handle messages we get (with a slight twist that Frame variant is visible
    /// since we are working with the underlying tungstenite library directly without axum here).
    fn process_message(msg: Message, who: usize) -> ControlFlow<(), ()> {
        match msg {
            Message::Text(t) => {
                println!(">>> {who} got str: {t:?}");
            }
            Message::Binary(d) => {
                println!(">>> {} got {} bytes: {:?}", who, d.len(), d);
            }
            Message::Close(c) => {
                if let Some(cf) = c {
                    println!(
                        ">>> {} got close with code {} and reason `{}`",
                        who, cf.code, cf.reason
                    );
                } else {
                    println!(">>> {who} somehow got close message without CloseFrame");
                }
                return ControlFlow::Break(());
            }

            Message::Pong(v) => {
                println!(">>> {who} got pong with {v:?}");
            }
            // Just as with axum server, the underlying tungstenite websocket library
            // will handle Ping for you automagically by replying with Pong and copying the
            // v according to spec. But if you need the contents of the pings you can see them here.
            Message::Ping(v) => {
                println!(">>> {who} got ping with {v:?}");
            }

            Message::Frame(_) => {
                unreachable!("This is never supposed to happen")
            }
        }
        ControlFlow::Continue(())
    }
}
