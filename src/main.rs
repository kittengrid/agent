// Rocket
#[macro_use]
extern crate rocket;

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

#[launch]
fn rocket_launch() -> _ {
    lib::rocket()
}
