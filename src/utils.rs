use crate::config::get_config;
use std::str::FromStr;
use std::sync::Once;
static INIT_LOGGER: Once = Once::new();

pub fn initialize_logger() {
    let config = get_config();
    let log_level = match log::LevelFilter::from_str(&config.log_level) {
        Ok(level) => level,
        Err(_) => {
            log::error!("Invalid log level: {}", config.log_level);
            log::error!("Setting log level to INFO");
            log::LevelFilter::Info
        }
    };

    INIT_LOGGER.call_once(|| {
        env_logger::Builder::new().filter_level(log_level).init();
    });
}
