use std::sync::Once;
static INIT_LOGGER: Once = Once::new();

pub fn initialize_logger() {
    INIT_LOGGER.call_once(|| {
        env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .try_init();
    });
}
