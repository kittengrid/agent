use clap::Parser;
use log::LevelFilter;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Log level (error, warn, info, debug and trace), defaults to info
    #[arg(short, long, default_value("info"), env("LOG_LEVEL"))]
    pub log_level: LevelFilter,

    /// Docker Compose binary location
    #[arg(
        short,
        long,
        default_value("/usr/bin/docker-compose"),
        env("DOCKER_COMPOSE_PATH")
    )]
    pub docker_compose_path: String,
}
