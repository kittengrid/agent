// Rocket
#[macro_use]
extern crate rocket;

use std::collections::HashMap;
use uuid::Uuid;

//Rocket
pub mod compose;
mod config;
pub mod data_dir;
mod docker_compose;
mod endpoints;
pub mod git_manager;
mod utils;

extern crate log;
use std::sync::{Arc, RwLock};
pub type AgentState = Arc<RwLock<HashMap<Uuid, Arc<compose::Context>>>>;

pub fn rocket() -> rocket::Rocket<rocket::Build> {
    utils::initialize_logger();

    let state: AgentState = Arc::new(RwLock::new(HashMap::<Uuid, Arc<compose::Context>>::new()));
    rocket::build()
        .manage(state)
        .mount("/", routes![endpoints::compose::new])
        .mount("/", routes![endpoints::compose::status])
        .mount("/", routes![endpoints::compose::show])
        .mount("/", routes![endpoints::sys::hello])
}

#[cfg(test)]
mod test_utils;
