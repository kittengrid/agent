// GET /sys/hello
//
// Description: Returns the cutiest Http response
pub async fn hello() -> &'static str {
    "kitty"
}

#[cfg(test)]
mod test {
    use crate::test_utils::*;
    use axum::http::StatusCode;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn hello() {
        let server_test = ServerTest::new().await;

        let response = server_test
            .client
            .get(server_test.url_for("/sys/hello"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "kitty");
    }
}
