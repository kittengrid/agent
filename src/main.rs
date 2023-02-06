// Rocket
#[macro_use]
extern crate rocket;

use std::collections::HashMap;
use uuid::Uuid;

use std::sync::{Arc, RwLock};
//Rocket

mod compose;
mod config;
mod data_dir;
mod docker_compose;
mod endpoints;
mod git_manager;
mod utils;

extern crate log;

// #[get("/fetch/<id>")]
// fn fetch(id: &str, state: &State<ComposeState>) -> String {
//     let data = state.data.lock().expect("lock shared data");
//     match data.get(id) {
//         Some(value) => value.to_string(),
//         None => String::from("NONE"),
//     }
// }

// POST /compose/
//
// Fetches the repository and starts a docker-compose up (it sets the run into fetching state)
//
// Parameters
//  repo:, :path = ./docker-compose.yaml
//
// Returns
//    HTTP/1.1 202 (Accepted)
//    Location: /compose/%{id}

// GET /compose/status/%{id}
// Returns Status of the component
//
// GET /compose/%{id}
// Returns the component

type AgentState = Arc<RwLock<HashMap<Uuid, Arc<compose::Context>>>>;

#[launch]
fn rocket_launch() -> _ {
    utils::initialize_logger();

    rocket()
}

fn rocket() -> rocket::Rocket<rocket::Build> {
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
