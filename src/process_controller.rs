use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::process::{Child, ExitStatus};
use std::sync::Arc;
use thiserror::Error;

use tokio::sync::oneshot;

use tokio::time::{self, Duration};

#[derive(Debug, Error)]
pub enum ProcessControllerError {
    #[error("Error receiving message: {0}")]
    ReceiveError(#[from] tokio::sync::oneshot::error::RecvError),

    #[error("Error Waiting for the process: {0}")]
    WaitError(#[from] std::io::Error),

    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

type OnStopCallback = dyn Fn(ExitStatus) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

/// This struct is used to control a process, it allows you to stop the process and wait for it to finish.
/// The whole point of this is being able to execute a callback when the process stops, either
/// because it crashed or because it was explictly stopped.
/// The callback is executed in the same thread that created the ProcessController, and the status
/// of the process is passed to the callback.
pub struct ProcessController {
    handle: tokio::task::JoinHandle<Result<(), ProcessControllerError>>,
    signal_channel: Option<oneshot::Sender<ServiceCommand>>,
    on_stop: Arc<OnStopCallback>,
}

impl fmt::Debug for ProcessController {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "ProcessController")
    }
}

#[derive(PartialEq, Debug)]
pub enum ServiceCommand {
    Stop,
}

impl ProcessController {
    pub async fn stop(&mut self) -> Result<(), ServiceCommand> {
        if let Some(sender) = self.signal_channel.take() {
            sender.send(ServiceCommand::Stop)
        } else {
            Err(ServiceCommand::Stop)
        }
    }

    pub async fn wait(self) -> Result<(), ProcessControllerError> {
        self.handle.await?
    }

    /// Creates a new ProcessController that will control the given child process.
    ///
    /// Arguments:
    /// - `child`: The child process to control.
    /// - `on_stop`: The callback to execute when the process stops.
    ///              it will receive the status of the process.
    pub async fn new(mut child: Child, on_stop: Arc<OnStopCallback>) -> Self {
        let (signal_channel, rx) = oneshot::channel();
        let handle = tokio::spawn({
            let on_stop = on_stop.clone();

            async move {
                tokio::select! {
                    msg = rx => {
                        match msg {
                            Ok(ServiceCommand::Stop) => {
                                child.kill()?;
                                let status = child.wait()?;
                                on_stop(status).await;
                                Ok(())
                            },
                            Err(e) =>                             Err(ProcessControllerError::ReceiveError(e)),
                        }
                    }
                    _ = time::sleep(Duration::from_secs(1)) => {
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                on_stop(status).await;
                                Ok(())
                            },
                            Ok(None) => {
                                let status = child.wait()?;
                                on_stop(status).await;
                                Ok(())
                            }
                            Err(e) => Err(ProcessControllerError::WaitError(e)),
                        }
                    }
                }
            }
        });

        Self {
            handle,
            signal_channel: Some(signal_channel),
            on_stop,
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
        let mut controller = ProcessController::new(child, Arc::new(closure(data_clone))).await;

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
        let controller = ProcessController::new(child, Arc::new(closure(data_clone))).await;

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
        let controller = ProcessController::new(child, Arc::new(closure(data_clone))).await;

        controller.wait().await.unwrap();
        assert_eq!(*data.lock().unwrap(), Some(1));
    }
}
