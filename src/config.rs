use clap::Parser;
use log::LevelFilter;
use once_cell::sync::Lazy;

// Returns a reference to a lazily created Config object.
// TODO: FIX TESTS ARGUMENTS
static CONFIG: Lazy<Config> = Lazy::new(|| Config {
    log_level: LevelFilter::Debug,
    work_directory: String::from("/tmp/test"),
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
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse() {
        let config = Config {
            log_level: LevelFilter::Debug,
            work_directory: String::from("/tmp/test"),
        };
        println!("{:?}", config);
    }
}
