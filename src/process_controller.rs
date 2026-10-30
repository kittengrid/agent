use crate::config::HealthCheck as HealthCheckConfig;
use crate::HealthStatus;
use log::{debug, error};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::process::{Child, ExitStatus};
use std::sync::Arc;

use thiserror::Error;

use tokio::sync::broadcast;

use tokio::task::JoinSet;
use tokio::time::{self, Duration};

#[derive(Debug, Error)]
pub enum ProcessControllerError {
    #[error("Error receiving message: {0}")]
    ReceiveError(#[from] tokio::sync::oneshot::error::RecvError),

    #[error("Error receiving broadcast message: {0}")]
    BroadcastReceiveError(#[from] tokio::sync::broadcast::error::RecvError),

    #[error("Error Waiting for the process: {0}")]
    WaitError(#[from] std::io::Error),

    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),

    #[error("Health-check client error: {0}")]
    HealthCheckClient(#[from] reqwest::Error),

    #[error("Error sending shutdown message: {0}")]
    ChannelSendError(#[from] tokio::sync::broadcast::error::SendError<ServiceCommand>),
}

type OnStopCallback = dyn Fn(ExitStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;
type OnStateChangedCallback =
    dyn Fn(crate::HealthStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

/// This struct is used to control a process, it allows you to stop the process and wait for it to finish.
/// The whole point of this is being able to execute a callback when the process stops, either
/// because it crashed or because it was explictly stopped.
/// The callback is executed in the same thread that created the ProcessController, and the status
/// of the process is passed to the callback.
pub struct ProcessController {
    stop_signal_sender: Option<broadcast::Sender<ServiceCommand>>,
    tasks: JoinSet<Result<(), ProcessControllerError>>,
}

impl fmt::Debug for ProcessController {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "ProcessController")
    }
}

pub struct HealthCheck {
    pub interval: u64,
    pub timeout: u64,
    pub retries: u64,
    pub path: String,
    pub host: String,
    pub port: u16,
}

impl HealthCheck {
    pub fn from_config(health_check: HealthCheckConfig, host: String, port: u16) -> Self {
        Self {
            interval: health_check.interval,
            timeout: health_check.timeout,
            retries: health_check.retries,
            path: health_check.path,
            host,
            port,
        }
    }
}

#[derive(PartialEq, Clone, Debug)]
pub enum ServiceCommand {
    Stop,
}

impl ProcessController {
    pub async fn stop(&mut self) -> Result<(), ProcessControllerError> {
        let sender = self.stop_signal_sender.take().unwrap();
        if let Err(e) = sender.send(ServiceCommand::Stop) {
            error!("There was an error stopping: {}", e);
        }
        self.wait().await?;

        Ok(())
    }

    pub async fn wait(&mut self) -> Result<(), ProcessControllerError> {
        while let Some(res) = self.tasks.join_next().await {
            res??
        }
        Ok(())
    }

    /// Creates a new ProcessController that will control the given child process.
    ///
    /// Arguments:
    /// - `child`: The child process to control.
    /// - `on_stop`: The callback to execute when the process stops.
    ///   It will receive the status of the process.
    /// - `health_check`: The health check to execute to determine if the process is
    ///   still running.
    pub async fn new(
        child: Child,
        on_stop: Arc<OnStopCallback>,
        health_check: Option<HealthCheck>,
        health_state_changed: Option<Arc<OnStateChangedCallback>>,
    ) -> Self {
        let (stop_tx, stop_rx) = broadcast::channel(1);
        let mut set = JoinSet::new();

        // Spawn the process monitor task
        set.spawn(Self::spawn_process_monitor_task(
            child,
            on_stop,
            stop_rx.resubscribe(),
            stop_tx.clone(),
        ));

        // Spawn the health check task if configured
        if let Some(health_check) = health_check {
            set.spawn(Self::spawn_health_check_task(
                health_check,
                stop_rx,
                health_state_changed,
            ));
        }

        Self {
            stop_signal_sender: Some(stop_tx),
            tasks: set,
        }
    }

    /// This is the task that gets spawned to monitor the health of the process.
    /// It will send a request to the health check endpoint and will execute the callback
    /// with the status of the health check when it changes.
    async fn spawn_health_check_task(
        health_check: HealthCheck,
        mut stop_rx: broadcast::Receiver<ServiceCommand>,
        on_stop_health_state_changed: Option<Arc<OnStateChangedCallback>>,
    ) -> Result<(), ProcessControllerError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(health_check.timeout.max(1)))
            .build()?;
        let url = format!(
            "http://{}:{}/{}",
            health_check.host,
            health_check.port,
            health_check.path.trim_start_matches('/')
        );
        let mut status = HealthStatus::Unhealthy;
        loop {
            tokio::select! {
                msg = stop_rx.recv() => {
                    match msg {
                        Ok(ServiceCommand::Stop) => {
                            debug!("Received stop signal, shutting down health check task");
                            break;
                        }
                        Err(e) => {
                            error!("Health check channel error: {}, shutting down", e);
                            return Err(ProcessControllerError::BroadcastReceiveError(e));
                        }
                    }
                }
                result = async {
                    time::sleep(Duration::from_secs(health_check.interval)).await;
                    client.get(&url).send().await
                } => {
                    // Keep the request inside select! so stop also cancels an
                    // in-flight check against a server that never responds.
                    let new_status = match result {
                        Ok(response) if response.status().is_success() => HealthStatus::Healthy,
                        _ => HealthStatus::Unhealthy,
                    };

                    if status != new_status {
                        status = new_status;
                        if let Some(on_state_changed) = on_stop_health_state_changed.as_ref() {
                            on_state_changed(status).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// This is the task that gets spawned to monitor the process.
    /// It will wait for the process to finish and will gather the status,
    /// and will execute the callback with the status.
    /// It will also listen for the stop signal and will kill the process
    /// if it receives it.
    async fn spawn_process_monitor_task(
        mut child: Child,
        on_stop: Arc<OnStopCallback>,
        mut stop_rx: broadcast::Receiver<ServiceCommand>,
        stop_tx: broadcast::Sender<ServiceCommand>,
    ) -> Result<(), ProcessControllerError> {
        loop {
            tokio::select! {
                msg = stop_rx.recv() => {
                    match msg {
                        Ok(ServiceCommand::Stop) => {
                            child.kill()?;
                            let status = child.wait()?;
                            on_stop(status).await;
                            return Ok(())
                        },
                        Err(e) => return Err(ProcessControllerError::BroadcastReceiveError(e)),
                    }
                }
                _ = time::sleep(Duration::from_secs(1)) => {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // We signal the health check task to stop
                            stop_tx.send(ServiceCommand::Stop)?;

                            on_stop(status).await;
                            return Ok(());
                        },
                        Ok(None) => {}
                        Err(e) => {
                            stop_tx.send(ServiceCommand::Stop)?;

                            return Err(ProcessControllerError::WaitError(e))
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Mutex;

    fn closure(
        data: Arc<Mutex<Option<i32>>>,
    ) -> impl Fn(ExitStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync {
        move |status| {
            let data = data.clone();
            Box::pin(async move {
                let _ = data.clone();
                {
                    let mut data = data.lock().unwrap();
                    *data = status.code();
                };
            })
        }
    }

    #[tokio::test]
    async fn test_process_controller_stopping() {
        let data = Arc::new(Mutex::new(Some(-1)));
        let data_clone = data.clone();

        let child = std::process::Command::new("sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let mut controller =
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None).await;

        assert!(controller.stop().await.is_ok());
        controller.wait().await.unwrap();
        assert_eq!(*data.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn test_process_controller_exiting_0() {
        let data = Arc::new(Mutex::new(None));
        let data_clone = data.clone();

        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let mut controller =
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None).await;

        controller.wait().await.unwrap();
        assert_eq!(*data.lock().unwrap(), Some(0));
    }

    #[tokio::test]
    async fn test_process_controller_exiting_1() {
        let data = Arc::new(Mutex::new(None));
        let data_clone = data.clone();

        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 1")
            .spawn()
            .unwrap();
        let mut controller =
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None).await;

        controller.wait().await.unwrap();
        assert_eq!(*data.lock().unwrap(), Some(1));
    }

    #[tokio::test]
    async fn test_process_controller_with_health_check() {
        // Keep the health result separate from the process exit status, and
        // bind an ephemeral port rather than depending on Python and port 8000.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new().route("/health", axum::routing::get(|| async { "ok" }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let exit_status = Arc::new(Mutex::new(Some(-1)));
        let (health_tx, mut health_rx) = tokio::sync::mpsc::channel(1);
        let child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let mut controller = ProcessController::new(
            child,
            Arc::new(closure(exit_status.clone())),
            Some(HealthCheck {
                interval: 0,
                timeout: 2,
                retries: 1,
                path: "/health".into(),
                host: "127.0.0.1".into(),
                port,
            }),
            Some(Arc::new(move |status| {
                let health_tx = health_tx.clone();
                Box::pin(async move {
                    let _ = health_tx.send(status).await;
                })
            })),
        )
        .await;
        let healthy = time::timeout(Duration::from_secs(5), health_rx.recv()).await;
        // Clean up even when the assertion below fails.
        controller.stop().await.unwrap();
        server.abort();
        assert_eq!(healthy.unwrap(), Some(HealthStatus::Healthy));
        assert_eq!(*exit_status.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn stopping_cancels_an_in_flight_health_check() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            // Accept the connection but never send any HTTP response.
            std::future::pending::<()>().await;
        });
        let (stop_tx, stop_rx) = broadcast::channel(1);
        let mut checker = tokio::spawn(ProcessController::spawn_health_check_task(
            HealthCheck {
                interval: 0,
                timeout: 60,
                retries: 1,
                path: "/".into(),
                host: "127.0.0.1".into(),
                port,
            },
            stop_rx,
            None,
        ));
        let accepted = time::timeout(Duration::from_secs(5), accepted_rx).await;
        stop_tx.send(ServiceCommand::Stop).unwrap();
        let stopped = time::timeout(Duration::from_secs(1), &mut checker).await;
        checker.abort();
        server.abort();
        accepted.unwrap().unwrap();
        stopped
            .expect("stop must not wait for the HTTP request timeout")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn health_check_uses_the_configured_request_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // First respond successfully, then leave the next request hanging.
        let (first_response_tx, first_response_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 1024];
            stream.read(&mut buffer).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            drop(stream);
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = first_response_tx.send(());
            std::future::pending::<()>().await;
        });
        let (stop_tx, stop_rx) = broadcast::channel(1);
        let (health_tx, mut health_rx) = tokio::sync::mpsc::channel(4);
        let checker = tokio::spawn(ProcessController::spawn_health_check_task(
            HealthCheck {
                interval: 0,
                timeout: 1,
                retries: 1,
                path: "/".into(),
                host: "127.0.0.1".into(),
                port,
            },
            stop_rx,
            Some(Arc::new(move |status| {
                let health_tx = health_tx.clone();
                Box::pin(async move {
                    let _ = health_tx.send(status).await;
                })
            })),
        ));
        let result = time::timeout(Duration::from_secs(5), async {
            assert_eq!(health_rx.recv().await, Some(HealthStatus::Healthy));
            first_response_rx.await.unwrap();
            assert_eq!(health_rx.recv().await, Some(HealthStatus::Unhealthy));
        })
        .await;
        let _ = stop_tx.send(ServiceCommand::Stop);
        checker.abort();
        server.abort();
        result.expect("a stalled health request must time out and mark the service unhealthy");
    }
}
