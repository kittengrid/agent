use crate::agent_state;
use crate::docker_compose::DockerCompose;
use crate::git_manager::{get_git_manager, GitHubRepo, GitReference};
use crate::{compose, data_dir};
use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use log::info;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio;

use uuid::Uuid;

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

#[derive(Deserialize)]
pub struct NewComposeRequest {
    github_user: String,
    github_repo: String,
    path: String,
    reference: GitReference,
}

pub async fn new(Json(payload): Json<NewComposeRequest>) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let repo = GitHubRepo::new(&payload.github_user, &payload.github_repo);
    let reference = payload.reference.clone();
    let path = payload.path;

    let ctx = Arc::new(compose::Context::new(
        compose::Status::FetchingRepo,
        repo.clone(),
        reference.clone(),
        vec![],
    ));

    {
        let ctx = ctx.clone();
        let mut hash = agent_state().write().unwrap();
        hash.insert(id, ctx);
    }

    let handle = tokio::task::spawn({
        let ctx = ctx.clone();
        async move {
            info!(
		    "Starting docker-compose for repository: {:?}, reference: {:?}, compose_files: {:?}",
		ctx.repo(),
		    ctx.repo_reference(),
		    ctx.paths()
		);
            let data_dir = data_dir::get_data_dir();

            match get_git_manager().clone_local(&repo, &reference.clone(), id) {
                Ok(_) => ctx.set_status(compose::Status::RepoReady),
                Err(err) => {
                    ctx.set_status(compose::Status::ErrorFetchingRepo(err));
                    return;
                }
            };

            let wd = data_dir
                .work_path()
                .unwrap()
                .join(Path::new(&id.to_string()));

            let docker_compose = match DockerCompose::new(data_dir) {
                Ok(mut ok) => {
                    ctx.set_status(compose::Status::ComposeInitialized);
                    ok.cwd(wd.into_os_string().into_string().unwrap())
                        .project_name(id.to_string())
                        .compose_file(path);
                    ctx.set_docker_compose(ok);
                    ctx.docker_compose().unwrap()
                }
                Err(err) => {
                    ctx.set_status(compose::Status::ErrorInitializingDockerCompose(err));
                    return;
                }
            };

            match docker_compose.create() {
                Ok(_) => ctx.set_status(compose::Status::ComposeCreated),
                Err(err) => ctx.set_status(compose::Status::ComposeCreatingError(err)),
            }

            {
                match docker_compose.start() {
                    Ok(_) => ctx.set_status(compose::Status::ComposeStarted),
                    Err(err) => ctx.set_status(compose::Status::ComposeStartingError(err)),
                }
            }
        }
    });

    ctx.set_handle(handle);

    (
        StatusCode::ACCEPTED,
        [(header::CONTENT_TYPE, "application/json")],
        Json(format!("{{\"id\":\"{}\"}}", id)),
    )
}

// GET /compose/%{id}/status
// Returns Status of the component
//

// #[get("/compose/<id>/status")]
// pub fn status(
//     agent_state: &State<crate::AgentState>,
//     id: String,
// ) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
//     let id = match Uuid::parse_str(&id) {
//         Ok(id) => id,
//         Err(_err) => {
//             return Err(rocket::response::status::Custom(
//                 Status::BadRequest,
//                 "Malformed id",
//             ))
//         }
//     };

//     {
//         let hash = agent_state.read().unwrap();
//         match hash.get(&id) {
//             Some(ctx) => Ok(Json(ctx.status())),
//             None => Err(rocket::response::status::Custom(
//                 Status::NotFound,
//                 "Not found",
//             )),
//         }
//     }
// }

// // POST /compose/%{id}/stop
// // Stops the docker-compose run.
// //

// #[post("/compose/<id>/stop")]
// pub fn stop(
//     agent_state: &State<crate::AgentState>,
//     id: String,
// ) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
//     let id = match Uuid::parse_str(&id) {
//         Ok(id) => id,
//         Err(_err) => {
//             return Err(rocket::response::status::Custom(
//                 Status::BadRequest,
//                 "Malformed id",
//             ))
//         }
//     };

//     {
//         let hash = agent_state.read().unwrap();
//         match hash.get(&id) {
//             Some(ctx) => {
//                 ctx.set_status(compose::Status::ComposeStopping);
//                 match ctx.docker_compose() {
//                     Some(docker_compose) => match docker_compose.stop() {
//                         Ok(_) => {
//                             ctx.set_status(compose::Status::ComposeStopped);
//                             Ok(Json(ctx.status()))
//                         }
//                         Err(err) => {
//                             ctx.set_status(compose::Status::ComposeStoppingError(err));
//                             Err(rocket::response::status::Custom(
//                                 Status::InternalServerError,
//                                 "ERROR Stopping",
//                             ))
//                         }
//                     },
//                     None => Err(rocket::response::status::Custom(
//                         Status::InternalServerError,
//                         "Docker compose fail",
//                     )),
//                 }
//             }
//             None => Err(rocket::response::status::Custom(
//                 Status::NotFound,
//                 "Not found",
//             )),
//         }
//     }
// }

// // POST /compose/%{id}/start
// // Starts the docker-compose run.
// //

// #[get("/compose/<id>/start")]
// pub fn start(
//     agent_state: &State<crate::AgentState>,
//     id: String,
// ) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
//     let id = match Uuid::parse_str(&id) {
//         Ok(id) => id,
//         Err(_err) => {
//             return Err(rocket::response::status::Custom(
//                 Status::BadRequest,
//                 "Malformed id",
//             ))
//         }
//     };

//     {
//         let hash = agent_state.read().unwrap();
//         match hash.get(&id) {
//             Some(ctx) => Ok(Json(ctx.status())),
//             None => Err(rocket::response::status::Custom(
//                 Status::NotFound,
//                 "Not found",
//             )),
//         }
//     }
// }

// // GET /compose/%{id}
// // Returns the component
// #[get("/compose/<id>")]
// pub fn show(
//     agent_state: &State<crate::AgentState>,
//     id: String,
// ) -> Result<Json<compose::Status>, rocket::response::status::Custom<&'static str>> {
//     let id = match Uuid::parse_str(&id) {
//         Ok(id) => id,
//         Err(_err) => {
//             return Err(rocket::response::status::Custom(
//                 Status::BadRequest,
//                 "Malformed id",
//             ))
//         }
//     };

//     {
//         let hash = agent_state.read().unwrap();
//         match hash.get(&id) {
//             Some(ctx) => Ok(Json(ctx.status())),
//             None => Err(rocket::response::status::Custom(
//                 Status::NotFound,
//                 "Not found",
//             )),
//         }
//     }
// }

// #[cfg(test)]
// mod test {

//     use crate::compose::Status as ComposeStatus;
//     use crate::endpoints::compose::{self, NewComposeRequest};
//     use crate::rocket;
//     use rocket::http::{ContentType, Status};
//     use rocket::local::blocking::Client;

//     fn simple_new_compose_request_data(branch: String) -> String {
//         serde_json::to_string(&NewComposeRequest {
//             github_user: "docker",
//             github_repo: "awesome-compose",
//             path: "traefik-golang/compose.yaml",
//             reference: crate::git_manager::GitReference::Branch(branch),
//         })
//         .unwrap()
//     }

//     #[test]
//     fn new() {
//         crate::utils::initialize_logger();
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client.post(uri!(compose::new)).dispatch();
//         assert_eq!(response.status(), Status::BadRequest);
//         let response = client
//             .post(uri!(compose::new))
//             .body(&simple_new_compose_request_data(String::from("main")))
//             .dispatch();
//         let location = response.headers().get_one("Location").unwrap();
//         assert_ne!(location, String::from(""));
//         println!("{}", location.to_string());
//         assert!(location.to_string().starts_with("/compose/"));
//         assert_eq!(response.content_type(), Some(ContentType::JSON));
//         warn!("{}", response.into_string().unwrap());
//     }

//     #[test]
//     fn status_errored() {
//         crate::utils::initialize_logger();
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client
//             .post(uri!(compose::new))
//             .body(&simple_new_compose_request_data(String::from("maddin")))
//             .dispatch();
//         let location = response.headers().get_one("Location").unwrap();
//         let response = client.get(location).dispatch();
//         assert_eq!(response.status(), Status::Ok);
//         let mut status = response.into_json::<ComposeStatus>().unwrap();
//         loop {
//             match status {
//                 ComposeStatus::FetchingRepo => {
//                     status = client
//                         .get(location)
//                         .dispatch()
//                         .into_json::<ComposeStatus>()
//                         .unwrap();
//                 }
//                 _ => break,
//             }
//         }
//         assert!(matches!(status, ComposeStatus::ErrorFetchingRepo { .. }));
//     }

//     #[test]
//     fn status_ok() {
//         crate::utils::initialize_logger();
//         let client = Client::tracked(rocket()).expect("valid rocket instance");
//         let response = client
//             .post(uri!(compose::new))
//             .body(&simple_new_compose_request_data(String::from("master")))
//             .dispatch();
//         let location = response.headers().get_one("Location").unwrap();
//         let response = client.get(location).dispatch();
//         assert_eq!(response.status(), Status::Ok);
//         let mut status = response.into_json::<ComposeStatus>().unwrap();
//         loop {
//             crate::test_utils::sleep(1);
//             match status {
//                 ComposeStatus::FetchingRepo => {
//                     status = client
//                         .get(location)
//                         .dispatch()
//                         .into_json::<ComposeStatus>()
//                         .unwrap();
//                 }
//                 _ => break,
//             }
//         }
//         let response = client
//             .post(uri!(compose::new))
//             .body(&simple_new_compose_request_data(String::from(
//                 "atomist/pin-docker-base-image/nginx-aspnet-mysql/backend/dockerfile",
//             )))
//             .dispatch();
//         let location = response.headers().get_one("Location").unwrap();
//         let response = client.get(location).dispatch();
//         assert_eq!(response.status(), Status::Ok);
//         let mut status = response.into_json::<ComposeStatus>().unwrap();
//         loop {
//             crate::test_utils::sleep(1);
//             match status {
//                 ComposeStatus::ComposeStarted => break,
//                 _ => {
//                     status = client
//                         .get(location)
//                         .dispatch()
//                         .into_json::<ComposeStatus>()
//                         .unwrap()
//                 }
//             }
//         }
//         debug!("{:?}", status);
//         assert!(matches!(status, ComposeStatus::ComposeStarted { .. }));
//     }
// }
// //     #[test]
// //     fn status_ok() {
// //         let client = Client::tracked(rocket()).expect("valid rocket instance");
// //         let response = client
// //             .post(uri!(compose::new))
// //             .body(&simple_new_compose_request_data())
// //             .dispatch();
// //         let location = response.headers().get_one("Location").unwrap();
// //         let response = client.get(location).dispatch();
// //         assert_eq!(response.status(), Status::Ok);
// //         //        println!("{}", response.into_string().unwrap());
// //         let status = response.into_json::<ComposeStatus>().unwrap();
// //         assert!(matches!(status, ComposeStatus::Fetching { .. }));
// //     }

// //     #[test]
// //     fn status_bad_uuid() {
// //         let client = Client::tracked(rocket()).expect("valid rocket instance");
// //         let response = client.get("/compose/status/BAD_UUID").dispatch();
// //         assert_eq!(response.status(), Status::BadRequest);
// //     }

// //     #[test]
// //     fn status_not_found() {
// //         let client = Client::tracked(rocket()).expect("valid rocket instance");
// //         let response = client
// //             .get("/compose/status/e35b1626-bfd9-4220-bd25-1b9527fa290a")
// //             .dispatch();
// //         assert_eq!(response.status(), Status::NotFound);
// //     }
// // }
