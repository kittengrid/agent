use crate::compose;
use rocket::http::{Header, Status};
use rocket::serde::{json::Json, Deserialize, Serialize};
use rocket::Responder;
use rocket::State;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Responder)]
#[response(status = 202, content_type = "json")]
pub struct AcceptResponder {
    inner: rocket::response::status::Accepted<String>,
    location: Header<'static>,
}

pub struct PathCatcher {
    path: String,
}
use rocket::request::{self, FromRequest, Request};

#[rocket::async_trait]
impl<'r> FromRequest<'r> for PathCatcher {
    type Error = std::fmt::Error;
    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        let path = req.uri().path();
        request::Outcome::Success(PathCatcher {
            path: path.to_string(),
        })
    }
}

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

// Struct for Request data

#[derive(Deserialize, Serialize)]
pub struct NewComposeRequest<'r> {
    repo: &'r str,
    path: &'r str,
}

#[post("/compose", data = "<request_data>")]
// TODO: meaningful errors (What the heck check https://github.com/SergioBenitez/Rocket/issues/749)
pub fn new(
    agent_state: &State<crate::AgentState>,
    request_path: PathCatcher,
    request_data: Json<NewComposeRequest<'_>>,
) -> AcceptResponder {
    let mutex = Arc::clone(agent_state);
    let id = Uuid::new_v4();

    {
        let mut hash = mutex.lock().unwrap();
        hash.insert(
            id,
            compose::Context {
                status: compose::Status::Fetching,
                run: compose::Run::new(
                    request_data.repo.to_string(),
                    Vec::from([request_data.path.to_string()]),
                ),
                id,
            },
        );
    }

    AcceptResponder {
        inner: rocket::response::status::Accepted(Some(format!("{{\"id\":\"{}\"}}", id))),
        location: Header::new(
            String::from("Location"),
            format!("{}/status/{}", request_path.path, id),
        ),
    }
}

// GET /compose/status/%{id}
// Returns Status of the component
//

#[get("/compose/status/<id>")]
pub fn status(
    agent_state: &State<crate::AgentState>,
    id: String,
) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
    let mutex = Arc::clone(agent_state);
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_err) => {
            return Err(rocket::response::status::Custom(
                Status::BadRequest,
                "Malformed id",
            ))
        }
    };

    {
        let hash = mutex.lock().unwrap();
        match hash.get(&id) {
            Some(value) => Ok(Json(value.status)),
            None => Err(rocket::response::status::Custom(
                Status::NotFound,
                "Not found",
            )),
        }
    }
}

// GET /compose/%{id}
// Returns the component
#[get("/compose/<id>")]
pub fn show(
    agent_state: &State<crate::AgentState>,
    id: String,
) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
    let mutex = Arc::clone(agent_state);
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_err) => {
            return Err(rocket::response::status::Custom(
                Status::BadRequest,
                "Malformed id",
            ))
        }
    };

    {
        let hash = mutex.lock().unwrap();
        match hash.get(&id) {
            Some(value) => Ok(Json(value.status)),
            None => Err(rocket::response::status::Custom(
                Status::NotFound,
                "Not found",
            )),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::compose::Status as ComposeStatus;
    use crate::endpoints::compose::{self, NewComposeRequest};
    use crate::rocket;
    use rocket::http::{ContentType, Status};
    use rocket::local::blocking::Client;

    fn simple_new_compose_request_data() -> String {
        serde_json::to_string(&NewComposeRequest {
            repo: "pepe",
            path: "juan",
        })
        .unwrap()
    }

    #[test]
    fn new() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.post(uri!(compose::new)).dispatch();
        assert_eq!(response.status(), Status::BadRequest);
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data())
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        assert!(location.to_string().starts_with("/compose/status/"));
    }

    #[test]
    fn status_ok() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data())
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        let response = client.get(location).dispatch();
        assert_eq!(response.status(), Status::Ok);
        //        println!("{}", response.into_string().unwrap());
        let status = response.into_json::<ComposeStatus>().unwrap();
        assert!(matches!(status, ComposeStatus::Fetching { .. }));
    }

    #[test]
    fn status_bad_uuid() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.get("/compose/status/BAD_UUID").dispatch();
        assert_eq!(response.status(), Status::BadRequest);
    }

    #[test]
    fn status_not_found() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/compose/status/e35b1626-bfd9-4220-bd25-1b9527fa290a")
            .dispatch();
        assert_eq!(response.status(), Status::NotFound);
    }
}
