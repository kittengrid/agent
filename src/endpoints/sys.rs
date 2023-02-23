// GET /sys/hello
//
// Description: Returns the cutiest Http response
pub async fn hello() -> &'static str {
    "kitty"
}

#[cfg(test)]
mod test {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };

    use crate::config::get_config;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn hello() {
        tokio::task::spawn(async { crate::launch().await });
        let config = get_config();

        let client = hyper::Client::new();
        let response = client
            .request(
                Request::builder()
                    .uri(format!(
                        "http://{}:{}/sys/hello",
                        config.bind_address, config.bind_port
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        assert_eq!(&body[..], b"kitty");
    }
}
