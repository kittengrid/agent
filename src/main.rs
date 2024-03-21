use lib::config::get_config;

#[tokio::main]
async fn main() {
    let config = get_config();
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.bind_address, config.bind_port))
            .await
            .unwrap();

    lib::publish_advertise_address(
        config.advertise_address.clone(),
        config.agent_token.clone(),
        config.api_url.clone(),
    )
    .await;

    lib::launch(listener).await;
}
