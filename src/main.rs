use std::net::TcpListener;

use lib::config::get_config;

#[tokio::main]
async fn main() {
    let config = get_config();
    let listener: TcpListener =
        TcpListener::bind(format!("{}:{}", config.bind_address, config.bind_port)).unwrap();
    lib::launch(listener).await;
}
