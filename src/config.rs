use clap_serde_derive::{
    clap::{self, Parser},
    ClapSerde,
};
use serde::Deserialize;

use once_cell::sync::Lazy;
use std::{collections::HashMap, fs::File, io::BufReader};

// Returns a reference to a lazily created Config object.
// TODO: FIX TESTS ARGUMENTS
static CONFIG: Lazy<Config> = Lazy::new(|| {
    if cfg!(test) {
        Config {
            log_level: String::from("error"),
            work_directory: String::from("/tmp/test"),
            bind_address: String::from("127.0.0.1"),
            bind_port: 8000,
            api_key: String::from("_some_token_"),
            api_url: String::from("http://web:3000"),
            vcs_provider: String::from("github"),
            vcs_id: String::from("1337"),
            workflow_id: String::from("12345678"),
            services: vec![],
        }
    } else {
        let mut args = Args::parse();

        let mut config = if let Ok(f) = File::open(&args.config_path) {
            // Parse config with serde
            match serde_yaml::from_reader::<_, <Config as ClapSerde>::Opt>(BufReader::new(f)) {
                // merge config already parsed from clap
                Ok(config) => Config::from(config).merge(&mut args.config),
                Err(err) => panic!("Error in configuration file:\n{}", err),
            }
        } else {
            // If there is not config file return only config parsed from clap
            Config::from(&mut args.config)
        };
        config.set_defaults_if_missing();
        config
    }
});

pub fn get_config() -> &'static Config {
    &CONFIG
}

// Inspiration from https://stackoverflow.com/questions/55133351/is-there-a-way-to-get-clap-to-use-default-values-from-a-file

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Config file with configuration using yaml, values in this file will be superseeded by command line arguments that in turn will be superseeded by environment variables
    #[clap(short, long = "config", default_value = "kittengrid.yml")]
    config_path: std::path::PathBuf,

    /// Rest of arguments
    #[clap(flatten)]
    pub config: <Config as ClapSerde>::Opt,
}

impl Config {
    fn set_defaults_if_missing(&mut self) -> &mut Self {
        if self.log_level.is_empty() {
            self.log_level = "info".to_string();
        }
        if self.work_directory.is_empty() {
            self.work_directory = "/var/lib/kittengrid-agent".to_string();
        }
        if self.bind_address.is_empty() {
            self.bind_address = "127.0.0.1".to_string();
        }
        if self.bind_port == 0 {
            self.bind_port = 3000;
        }
        self
    }
}

#[derive(Parser, Debug, Clone, ClapSerde)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Log level (error, warn, info, debug and trace), defaults to info
    #[arg(short, long, env("KITTENGRID_LOG_LEVEL"))]
    pub log_level: String,

    #[arg(short, long, env("KITTENGRID_WORK_DIR"))]
    pub work_directory: String,

    /// Bind address for the agent. [default: 127.0.0.1]
    #[arg(long, env("KITTENGRID_BIND_ADDRESS"))]
    pub bind_address: String,

    /// Bind port for the agent. [default: 3000]
    #[arg(long, env("KITTENGRID_BIND_PORT"))]
    pub bind_port: u16,

    #[arg(long, env("KITTENGRID_API_KEY"))]
    pub api_key: String,

    #[arg(long, env("KITTENGRID_API_URL"))]
    pub api_url: String,

    #[arg(long, env("KITTENGRID_VCS_PROVIDER"))]
    pub vcs_provider: String,

    #[arg(long, env("KITTENGRID_VCS_ID"))]
    pub vcs_id: String,

    #[arg(long, env("KITTENGRID_WORKFLOW_ID"))]
    pub workflow_id: String,

    #[clap(skip)]
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceConfig {
    pub name: String,
    pub port: u16,
    pub cmd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub args: Option<Vec<String>>,
    pub health_check: Option<HealthCheck>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HealthCheck {
    pub interval: Option<u64>,
    pub timeout: Option<u64>,
    pub retries: Option<u64>,
    pub path: Option<String>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse() {
        let config = get_config();
        assert_eq!(config.bind_address, "127.0.0.1");
    }
}
