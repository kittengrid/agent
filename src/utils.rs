use std::sync::Once;
static INIT_LOGGER: Once = Once::new();

pub fn initialize_logger(config: crate::config::Config) {
    INIT_LOGGER.call_once(|| {
        env_logger::Builder::new()
            .filter_level(config.log_level)
            .try_init();
    });
}
