// GET /sys/hello
//
// Description: Returns the cutiest Http response
pub async fn hello() -> &'static str {
    "kitty"
}

#[cfg(test)]
mod test {
    use crate::test_utils::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn hello() {
        let server_test = ServerTest::new().await;

        let response = server_test
            .client
            .request(
                Request::builder()
                    .uri(server_test.url_for("/sys/hello"))
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
