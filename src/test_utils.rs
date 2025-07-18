use crate::kittengrid_agent::KittengridAgent;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::service::Service;
use crate::utils;
use jsonwebtoken::{encode, EncodingKey, Header};
use std::sync::Once;

use log::debug;
use std::env;
use std::io::BufReader;

use crate::endpoints::public::services::Claims;
use std::process::Command;
use std::process::Output;
use std::sync::Arc;
use std::{thread, time};

use libc::{dup, dup2, fflush, STDOUT_FILENO};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::io::{AsRawFd, RawFd};

use tempfile::tempfile;

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

static INIT: Once = Once::new();

pub fn initialize_tests() {
    INIT.call_once(|| {
        if env::var("DEBUG").is_ok() {
            env::set_var("KITTENGRID_LOG_LEVEL", "debug");
            utils::initialize_logger()
        }
    });
}

// @TODO: use the Args to pass to this function
pub fn log_generator_service() -> Service {
    let config = crate::config::ServiceConfig {
        name: "target/debug/log-generator".to_string(),
        ..Default::default()
    };
    Service::from(config)
}

pub struct ServerTest {
    guard: tokio::task::JoinHandle<()>,
    pub client: reqwest::Client,
    addr: String,
    port: u16,
    services: Arc<crate::service::Services>,
}

impl Drop for ServerTest {
    fn drop(&mut self) {
        self.guard.abort();
    }
}

impl ServerTest {
    pub fn services(&self) -> Arc<crate::service::Services> {
        self.services.clone()
    }

    pub fn url_for(&self, path: &str) -> String {
        format!("http://{}:{}{}", self.addr, self.port, path)
    }
    pub fn url_for_with_protocol(&self, protocol: &str, path: &str) -> String {
        format!("{}://{}:{}{}", protocol, self.addr, self.port, path)
    }

    pub fn compile_log_generator() {
        if File::open("target/debug/log-generator").is_ok() {
            return;
        }

        Command::new("cargo")
            .args(["build", "--bin", "log-generator"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
    }

    pub fn valid_token(&self) -> String {
        let current_time_in_seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_secs(),
            Err(_) => panic!("SystemTime before UNIX EPOCH!"),
        };
        self.token(current_time_in_seconds + 3600)
    }

    pub fn invalid_token(&self) -> String {
        self.token(0)
    }

    fn token(&self, expires_at: u64) -> String {
        let claims = Claims {
            bearer_id: "test".to_string(),
            bearer_type: "test".to_string(),
            exp: expires_at,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(crate::config::get_config().clone().api_key.as_bytes()),
        )
        .unwrap()
    }

    pub async fn new(spawn_services: bool) -> Self {
        let agent = KittengridAgent::new(crate::config::get_config().clone());

        // In tests, we only set up the logger when the KITTENGRID_LOG_LEVEL is found
        agent.init().await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().ip().to_string();
        let port = listener.local_addr().unwrap().port();
        let client = reqwest::Client::new();
        let services = agent.services();

        if spawn_services {
            // We need to compile the log generator before we can spawn the services
            ServerTest::compile_log_generator();

            agent
                .spawn_services(false)
                .await
                .expect("Failed to spawn services");
        }

        let guard = tokio::task::spawn(async move {
            agent.wait(listener).await;
            debug!("ServerTest: Server stopped")
        });
        Self {
            guard,
            client,
            addr,
            port,
            services,
        }
    }
}

// This is a simple struct used in test to be able
// to have control over what is written to stdout.
pub struct StdoutWriter {
    pub stdout: BufReader<std::process::ChildStdout>,
    pub stdin: std::process::ChildStdin,
}

impl StdoutWriter {
    /// This function internally creates a new process which only
    /// function is to write to stdout whetever it reads from stdin.
    pub fn new() -> (Self, std::process::Child) {
        let mut cmd = Command::new("/usr/bin/cat");
        cmd.arg("-");
        cmd.stdout(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("failed to execute child");

        let stdout = BufReader::new(child.stdout.take().expect("stdout is None"));
        let stdin = child.stdin.take().expect("stdin is None");
        (Self { stdout, stdin }, child)
    }
}

pub fn capture_stdout<F: FnOnce()>(f: F) -> String {
    unsafe {
        // Create a temporary file to redirect stdout into
        let mut temp_file = tempfile().expect("Failed to create tempfile");

        // Save the original stdout
        let original_fd: RawFd = dup(STDOUT_FILENO);

        // Redirect stdout to the temporary file
        dup2(temp_file.as_raw_fd(), STDOUT_FILENO);

        // Call the function that writes to stdout
        f();

        // Flush stdout to ensure all output is written
        fflush(std::ptr::null_mut());

        // Restore original stdout
        dup2(original_fd, STDOUT_FILENO);
        libc::close(original_fd);

        // Read from the temporary file
        temp_file.seek(SeekFrom::Start(0)).unwrap();
        let mut output = String::new();
        temp_file.read_to_string(&mut output).unwrap();

        output
    }
}
