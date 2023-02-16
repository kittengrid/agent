use crate::docker_compose::DockerCompose;
use crate::git_manager::{get_git_manager, GitHubRepo, GitReference};
use crate::{compose, data_dir};
use rocket::http::{Header, Status};
use rocket::serde::{json::Json, Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio;

use rocket::Responder;
use rocket::State;

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
    github_user: &'r str,
    github_repo: &'r str,
    path: &'r str,
    reference: GitReference,
}

#[post("/compose", data = "<request_data>")]
// TODO: meaningful errors (What the heck check https://github.com/SergioBenitez/Rocket/issues/749)
pub async fn new(
    agent_state: &State<crate::AgentState>,
    request_path: PathCatcher,
    request_data: Json<NewComposeRequest<'_>>,
) -> AcceptResponder {
    let id = Uuid::new_v4();
    let repo = GitHubRepo::new(request_data.github_user, request_data.github_repo);
    let reference = request_data.reference.clone();
    let path = String::from(request_data.path);

    let ctx = Arc::new(compose::Context::new(
        compose::Status::FetchingRepo,
        repo.clone(),
        reference.clone(),
        vec![],
    ));
    let cloned_ctx = ctx.clone();

    let handle = tokio::task::spawn(async move {
        let git_manager = get_git_manager();
        let data_dir = data_dir::get_data_dir();

        let fetch = git_manager.download_remote_repository(&repo);
        if fetch.is_err() {
            cloned_ctx.set_status(compose::Status::ErrorFetchingRepo(fetch.err().unwrap()));
        }
        match get_git_manager().clone_local_by_reference(&repo.clone(), &reference.clone(), id) {
            Ok(_) => cloned_ctx.set_status(compose::Status::RepoReady),
            Err(err) => {
                cloned_ctx.set_status(compose::Status::ErrorFetchingRepo(err));
                return;
            }
        };

        let mut docker_compose;
        match DockerCompose::new(data_dir) {
            Ok(ok) => docker_compose = ok,
            Err(err) => {
                cloned_ctx.set_status(compose::Status::ErrorInitializingDockerCompose(err));
                return;
            }
        }

        let wd = data_dir
            .work_path()
            .unwrap()
            .join(Path::new(&id.to_string()));

        docker_compose
            .cwd(wd.into_os_string().into_string().unwrap())
            .project_name(id.to_string())
            .compose_file(path);
        cloned_ctx.set_status(compose::Status::ComposeInitialized);
        match docker_compose.start() {
            Ok(_) => cloned_ctx.set_status(compose::Status::ComposeStarted),
            Err(err) => cloned_ctx.set_status(compose::Status::ComposeStartingError(err)),
        }
    });

    ctx.set_handle(handle);

    {
        let mut hash = agent_state.write().unwrap();
        hash.insert(id, ctx);
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
        let hash = agent_state.read().unwrap();
        match hash.get(&id) {
            Some(ctx) => Ok(Json(ctx.status())),
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
        let hash = agent_state.read().unwrap();
        match hash.get(&id) {
            Some(ctx) => Ok(Json(ctx.status())),
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
    use std::{thread, time};

    fn simple_new_compose_request_data(branch: String) -> String {
        serde_json::to_string(&NewComposeRequest {
            github_user: "docker",
            github_repo: "awesome-compose",
            path: "plex/compose.yaml",
            reference: crate::git_manager::GitReference::Branch(branch),
        })
        .unwrap()
    }

    #[test]
    fn new() {
        crate::utils::initialize_logger();
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.post(uri!(compose::new)).dispatch();
        assert_eq!(response.status(), Status::BadRequest);
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data(String::from("main")))
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        assert_ne!(location, String::from(""));
        assert!(location.to_string().starts_with("/compose/status/"));
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        warn!("{}", response.into_string().unwrap());
    }

    #[test]
    fn status_errored() {
        crate::utils::initialize_logger();
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data(String::from("maddin")))
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        let response = client.get(location).dispatch();
        assert_eq!(response.status(), Status::Ok);
        let mut status = response.into_json::<ComposeStatus>().unwrap();
        loop {
            match status {
                ComposeStatus::FetchingRepo => {
                    status = client
                        .get(location)
                        .dispatch()
                        .into_json::<ComposeStatus>()
                        .unwrap();
                }
                _ => break,
            }
        }
        assert!(matches!(status, ComposeStatus::ErrorFetchingRepo { .. }));
    }

    #[test]
    fn status_ok() {
        crate::utils::initialize_logger();
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data(String::from("master")))
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        let response = client.get(location).dispatch();
        assert_eq!(response.status(), Status::Ok);
        let mut status = response.into_json::<ComposeStatus>().unwrap();
        let one_sec = time::Duration::from_secs(1);
        loop {
            thread::sleep(one_sec);
            println!("status {:?}", status);
            match status {
                ComposeStatus::FetchingRepo => {
                    status = client
                        .get(location)
                        .dispatch()
                        .into_json::<ComposeStatus>()
                        .unwrap();
                }
                _ => break,
            }
        }
        println!("{:?}", status);
        assert!(matches!(status, ComposeStatus::RepoReady { .. }));
        let response = client
            .post(uri!(compose::new))
            .body(&simple_new_compose_request_data(String::from(
                "atomist/pin-docker-base-image/nginx-aspnet-mysql/backend/dockerfile",
            )))
            .dispatch();
        let location = response.headers().get_one("Location").unwrap();
        let response = client.get(location).dispatch();
        assert_eq!(response.status(), Status::Ok);
        let mut status = response.into_json::<ComposeStatus>().unwrap();
        let one_sec = time::Duration::from_secs(1);
        loop {
            thread::sleep(one_sec);
            match status {
                ComposeStatus::FetchingRepo => {
                    status = client
                        .get(location)
                        .dispatch()
                        .into_json::<ComposeStatus>()
                        .unwrap();
                }
                _ => break,
            }
        }
        debug!("{:?}", status);
        assert!(matches!(status, ComposeStatus::RepoReady { .. }));
    }
}
//     #[test]
//     fn status_ok() {
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client
//             .post(uri!(compose::new))
//             .body(&simple_new_compose_request_data())
//             .dispatch();
//         let location = response.headers().get_one("Location").unwrap();
//         let response = client.get(location).dispatch();
//         assert_eq!(response.status(), Status::Ok);
//         //        println!("{}", response.into_string().unwrap());
//         let status = response.into_json::<ComposeStatus>().unwrap();
//         assert!(matches!(status, ComposeStatus::Fetching { .. }));
//     }

//     #[test]
//     fn status_bad_uuid() {
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client.get("/compose/status/BAD_UUID").dispatch();
//         assert_eq!(response.status(), Status::BadRequest);
//     }

//     #[test]
//     fn status_not_found() {
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client
//             .get("/compose/status/e35b1626-bfd9-4220-bd25-1b9527fa290a")
//             .dispatch();
//         assert_eq!(response.status(), Status::NotFound);
//     }
// }
