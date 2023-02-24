// Rocket
#[macro_use]
extern crate rocket;

use std::collections::HashMap;
use uuid::Uuid;
use std::net::Ipv4Addr;

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
pub type AgentState<'a> = Arc<RwLock<HashMap<Uuid, Arc<compose::Context<'a>>>>>;

pub fn rocket() -> rocket::Rocket<rocket::Build> {
    let config = rocket::Config {
        port: 8000,
        address: Ipv4Addr::new(0, 0, 0, 0).into(),
        ..rocket::Config::default()
    };

    utils::initialize_logger();

    let state: AgentState = Arc::new(RwLock::new(HashMap::<Uuid, Arc<compose::Context>>::new()));
    rocket::custom(&config)
        .manage(state)
        .mount("/", routes![endpoints::compose::new])
        .mount("/", routes![endpoints::compose::status])
        .mount("/", routes![endpoints::compose::stop])
        .mount("/", routes![endpoints::compose::start])
        .mount("/", routes![endpoints::compose::show])
        .mount("/", routes![endpoints::sys::hello])
}

#[cfg(test)]
mod test_utils;
