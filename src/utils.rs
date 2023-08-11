use crate::config::get_config;
use std::sync::Once;
static INIT_LOGGER: Once = Once::new();
use std::net::TcpListener;

pub fn initialize_logger() {
    let config = get_config();
    INIT_LOGGER.call_once(|| {
        env_logger::Builder::new()
            .filter_level(config.log_level)
            .init();
    });
}

pub fn is_port_in_use(port: u16) -> bool {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => {
            // The port is not in use since binding succeeded
            false
        }
        Err(_) => {
            // The port is in use
            true
        }
    }
}
