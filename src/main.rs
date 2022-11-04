// Rocket
#[macro_use]
extern crate rocket;

use std::collections::HashMap;
use uuid::Uuid;

use std::sync::{Arc, Mutex};
//Rocket

use clap::Parser;
mod compose;
mod config;
mod endpoints;

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
pub type AgentState = Arc<Mutex<HashMap<Uuid, compose::Context>>>;

#[launch]
fn rocket_launch() -> _ {
    let args = config::Config::parse();
    env_logger::Builder::new()
        .filter_level(args.log_level)
        .init();
    println!("{:?}", args);
    rocket()
}

fn rocket() -> rocket::Rocket<rocket::Build> {
    let state = AgentState::new(Mutex::new(HashMap::<Uuid, compose::Context>::new()));
    rocket::build()
        .manage(state)
        .mount("/", routes![endpoints::compose::new])
        .mount("/", routes![endpoints::compose::status])
        .mount("/", routes![endpoints::compose::show])
        .mount("/", routes![endpoints::sys::hello])
}
