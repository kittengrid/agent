use crate::config::HealthCheck as HealthCheckConfig;
use crate::HealthStatus;
use log::{debug, error, info};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::process::{Child, ExitStatus};
use std::sync::Arc;

use thiserror::Error;

use tokio::net::{TcpListener, TcpStream};
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

    #[error("Error sending shutdown message: {0}")]
    ChannelSendError(#[from] tokio::sync::broadcast::error::SendError<ServiceCommand>),

    #[error("TCP proxy listener error: {0}")]
    TcpProxyBind(std::io::Error),
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
        proxy_target: Option<(String, u16)>,
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

        if let Some((target_host, port)) = proxy_target {
            set.spawn(Self::spawn_tcp_proxy_task(
                target_host,
                port,
                stop_rx.resubscribe(),
            ));
        }

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
                _ = time::sleep(Duration::from_secs(health_check.interval)) => {
                    let new_status = reqwest::get(&format!("http://{}:{}/{}", health_check.host, health_check.port, health_check.path))
                        .await
                        .map(|_| HealthStatus::Healthy)
                        .unwrap_or_else(|_| HealthStatus::Unhealthy);

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

    /// Listens locally for tunneled traffic and forwards it to a service in another
    /// network namespace, such as GitLab's `docker:dind` service container.
    async fn spawn_tcp_proxy_task(
        target_host: String,
        port: u16,
        mut stop_rx: broadcast::Receiver<ServiceCommand>,
    ) -> Result<(), ProcessControllerError> {
        let listener = TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(ProcessControllerError::TcpProxyBind)?;
        info!("Proxying local port {} to {}:{}", port, target_host, port);

        loop {
            tokio::select! {
                message = stop_rx.recv() => match message {
                    Ok(ServiceCommand::Stop) => break,
                    Err(error) => return Err(ProcessControllerError::BroadcastReceiveError(error)),
                },
                accepted = listener.accept() => {
                    let (mut client, _) = accepted.map_err(ProcessControllerError::TcpProxyBind)?;
                    let target_host = target_host.clone();
                    tokio::spawn(async move {
                        match TcpStream::connect((target_host.as_str(), port)).await {
                            Ok(mut target) => {
                                let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
                            }
                            Err(error) => {
                                error!("Failed to connect TCP proxy to {}:{}: {}", target_host, port, error);
                            }
                        }
                    });
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
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None, None).await;

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
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None, None).await;

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
            ProcessController::new(child, Arc::new(closure(data_clone)), None, None, None).await;

        controller.wait().await.unwrap();
        assert_eq!(*data.lock().unwrap(), Some(1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_process_controller_with_health_check() {
        let data = Arc::new(Mutex::new(None));
        let data_clone = data.clone();
        let data_clone_2 = data.clone();

        let child = std::process::Command::new("python")
            .arg("-m")
            .arg("http.server")
            .arg("8000")
            .spawn()
            .unwrap();

        let mut controller = ProcessController::new(
            child,
            Arc::new(closure(data_clone)),
            Some(HealthCheck {
                interval: 1,
                timeout: 10,
                retries: 10,
                path: "/".to_string(),
                host: "127.0.0.1".to_string(),
                port: 8000,
            }),
            None,
            Some(Arc::new({
                let data = data_clone_2.clone();
                move |status: crate::HealthStatus| {
                    Box::pin({
                        let data = data.clone();
                        async move {
                            {
                                let mut data = data.lock().unwrap();
                                match status {
                                    crate::HealthStatus::Healthy => *data = Some(1),
                                    crate::HealthStatus::Unhealthy => *data = Some(0),
                                }
                            };
                        }
                    })
                }
            })),
        )
        .await;
        assert_eq!(*data.lock().unwrap(), None);
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(*data.lock().unwrap(), Some(1));

        controller.stop().await.unwrap();
        controller.wait().await.unwrap();
    }
}
