use crate::data_dir::{DataDir, DataDirInitError};
use clap::Parser;
use log::LevelFilter;
use once_cell::sync::{Lazy, OnceCell};

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Log level (error, warn, info, debug and trace), defaults to info
    #[arg(short, long, default_value("info"), env("LOG_LEVEL"))]
    pub log_level: LevelFilter,

    #[arg(
        short,
        long,
        default_value("/var/lib/kittengrid-agent"),
        env("WORK_DIR")
    )]
    pub work_directory: String,
}

static CONFIG: Lazy<Config> = Lazy::new(Config::parse);

pub fn get_config() -> &'static Config {
    &CONFIG
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse() {
        let config = Config::parse();
        println!("{:?}", config);
    }
}
