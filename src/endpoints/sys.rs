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

    #[tokio::test]
    async fn hello() {
        tokio::spawn(async move { crate::launch().await });

        let client = hyper::Client::new();
        let response = client
            .request(
                Request::builder()
                    .uri(format!("http://localhost:3000/sys/hello"))
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
