// Rocket
#[macro_use]
extern crate rocket;

use rocket::State;
use std::collections::HashMap;
use std::sync::Mutex;
//Rocket

use clap::Parser;
mod config;

extern crate log;

#[get("/fetch/<id>")]
fn fetch(id: &str, state: &State<ComposeState>) -> String {
    let data = state.data.lock().expect("lock shared data");
    match data.get(id) {
        Some(value) => value.to_string(),
        None => String::from("NONE"),
    }
}

struct ComposeState {
    data: Mutex<HashMap<String, String>>,
}

#[launch]
fn rocket() -> _ {
    let args = config::Config::parse();
    env_logger::Builder::new()
        .filter_level(args.log_level)
        .init();
    println!("{:?}", args);

    let my_state = ComposeState {
        data: Mutex::new(HashMap::<String, String>::new()),
    };

    rocket::build().manage(my_state).mount("/", routes![fetch])
}
