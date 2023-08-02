use clap::Parser;
use log::LevelFilter;
use once_cell::sync::Lazy;

// Returns a reference to a lazily created Config object.
// TODO: FIX TESTS ARGUMENTS
static CONFIG: Lazy<Config> = Lazy::new(|| {
    if cfg!(test) {
        Config {
            log_level: LevelFilter::Error,
            work_directory: String::from("/tmp/test"),
            bind_address: String::from("127.0.0.1"),
            bind_port: 8000,
            advertise_address: String::from("http://127.0.0.1:8000"),
            agent_token: String::from("t0k3n"),
            api_url: String::from("http://localhost:3001"),
        }
    } else {
        Config::parse()
    }
});

pub fn get_config() -> &'static Config {
    &CONFIG
}

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

    #[arg(long, default_value("127.0.0.1"), env("BIND_ADDRESS"))]
    pub bind_address: String,

    #[arg(long, default_value("3000"), env("BIND_PORT"))]
    pub bind_port: u16,

    #[arg(long, default_value("http://127.0.0.1:8000/"), env("ADVERTISE_ADDRESS"))]
    pub advertise_address: String,

    #[arg(long, default_value("t0k3n"), env("AGENT_TOKEN"))]
    pub agent_token: String,

    #[arg(long, default_value("http://localhost:3001"), env("API_URL"))]
    pub api_url: String,
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
