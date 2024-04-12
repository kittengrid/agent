#[cfg(test)]
use crate::launch;
use log::debug;
use std::process::Output;
use std::{thread, time};

#[allow(dead_code)]
pub fn debug_output(output: &Output) {
    debug!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    );
}

#[allow(dead_code)]
pub fn sleep(secs: u64) {
    let secs = time::Duration::from_secs(secs);
    thread::sleep(secs);
}

pub struct ServerTest {
    guard: tokio::task::JoinHandle<()>,
    pub client: reqwest::Client,
    addr: String,
    port: u16,
}

impl Drop for ServerTest {
    fn drop(&mut self) {
        self.guard.abort();
    }
}

impl ServerTest {
    pub fn url_for(&self, path: &str) -> String {
        format!("http://{}:{}{}", self.addr, self.port, path)
    }

    pub async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().ip().to_string();
        let port = listener.local_addr().unwrap().port();
        let client = reqwest::Client::new();

        let guard = tokio::task::spawn(async move { launch(listener).await });
        Self {
            guard,
            client,
            addr,
            port,
        }
    }
}
