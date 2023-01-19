use crate::config::get_config;
use std::sync::Once;
static INIT_LOGGER: Once = Once::new();

pub fn initialize_logger() {
    INIT_LOGGER.call_once(|| {
        let config = get_config();
        env_logger::Builder::new()
            .filter_level(config.log_level)
            .try_init();
    });
}
