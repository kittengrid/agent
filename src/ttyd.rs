use regex::Regex;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::Command; // Needed for chmod on Unix
use thiserror::Error;

#[derive(Error)]
pub enum Error {
    #[error("Error: {0}")]
    ExecError(String),

    #[error("IoError: {0}")]
    IoError(#[from] std::io::Error),
}

// This is so we can use ? in main without having to unwrap the error
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

pub struct Executable {
    bin_path: String,
}

impl Drop for Executable {
    fn drop(&mut self) {
        fs::remove_file(&self.bin_path).expect("Failed to delete temporary binary");
    }
}

impl Default for Executable {
    fn default() -> Self {
        let bin_path = crate::binary_utils::install_binary(include_bytes!(env!("TTYD")));
        Self { bin_path }
    }
}

impl Executable {
    /// Starts the ttyd server with the given base path.
    pub async fn start(&self, base_path: &str) -> Result<u16, Error> {
        let mut port: u16 = 0;
        let mut child = Command::new(&self.bin_path)
            .arg("-W")
            .arg("-p")
            .arg("0")
            .arg("-b")
            .arg(base_path)
            .arg("bash")
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Get the stdout handle
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => return Err(Error::ExecError("Failed to get stderr".to_string())),
        };

        let reader = BufReader::new(stderr);
        let re = Regex::new(r".*istening on port:\s*(\d+)").unwrap();

        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if let Some(caps) = re.captures(&line) {
                        port = caps[1].parse().unwrap_or(0);
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Error reading stderr: {}", e);
                    return Err(Error::IoError(e));
                }
            }
        }

        tokio::spawn(async move {
            let _ = child.wait();
        });

        Ok(port)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_ttyd_start() {
        let ttyd = Executable::default();
        let port = ttyd.start().await.unwrap();
        assert!(port > 0, "TTYD should start on a valid port");
    }
}
