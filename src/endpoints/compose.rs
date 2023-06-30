use crate::agent_state;
use crate::api_error::ApiError;
use crate::compose::Context;
use crate::docker_compose::DockerCompose;
use crate::git_manager::{get_git_manager, GitHubRepo, GitReference};
use crate::{compose, data_dir};
use axum::{
    extract::Path as AxumPath,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use axum_extra::extract::WithRejection;

use log::info;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio;

use uuid::Uuid;

// Structs for create request and response
#[derive(Deserialize, Serialize, Debug)]
pub struct CreateRequest {
    github_user: String,
    github_repo: String,
    token: Option<String>,
    path: String,
    reference: GitReference,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Uuid>,
}

/// POST /compose/
///
/// Fetches the repository and starts a docker-compose. This endpoint is asynchronous
/// if everything goes well, it schedules the job and returns a 202 Accepted with a Location header
/// containinig the resource url.
///
/// # Arguments
///
///  * `github_user` - Github user to construct the url for fetching the repo.
///  * `github_repo` - Github repo to construct the url for fetching the repo.
///  * `path`        - Path of docker-compose file relative to repository.
///  * `reference`   - Either a branch or a commit.
///  * `token`       - Github Token to be used
///
/// # Example
///
/// ```bash
/// curl -d '{
///   "token": "ghs_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
///   "github_user":"docker", \
///   "github_repo": "awesome-compose", \
///   "path": "minecraft/compose.yaml", \
///   "reference": {"branch": "master"}}' \
///   -H "Content-Type: application/json" \
///   -X POST http://localhost:3000/compose
/// ```
///
/// # Response
///
///    HTTP/1.1 202 (Accepted)
///    Location: /compose/%{id}
///
pub async fn create(
    WithRejection(Json(payload), _): WithRejection<Json<CreateRequest>, ApiError>,
) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let repo = GitHubRepo::new(&payload.github_user, &payload.github_repo, payload.token);
    let reference = payload.reference.clone();
    let path = payload.path;

    let ctx = Arc::new(compose::Context::new(
        id,
        compose::Status::FetchingRepo,
        repo.clone(),
        reference.clone(),
        vec![],
    ));

    {
        let ctx = ctx.clone();
        let mut hash = match agent_state().write() {
            Ok(hash) => hash,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::LOCATION, String::from(""))],
                    Json(CreateResponse {
                        id: None,
                        error: Some(format!("{:?}", err)),
                    }),
                )
            }
        };
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
        [(header::LOCATION, format!("/compose/{}", id))],
        Json(CreateResponse {
            id: Some(id),
            error: None,
        }),
    )
}

#[derive(Serialize, Deserialize)]
pub struct ShowResponse<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    inner: Option<Arc<Context<'a>>>,
}

/// GET /compose/:id
///
/// Returns the information related to a given compose execution.
///
/// # Arguments
///
///  * `id` - Id returned by create endpoint.
///
/// # Example
///
/// ```bash
/// curl http://localhost:3000/compose/ID
/// ```
///
/// # Response
///
/// ```bash
///    HTTP/1.1 200 Ok
///    ...
///    {
///      "id" : "1a7fbdd3-84ca-4693-8935-1f93c152a1d8",
///      "paths" : ["docker-compose.yaml"],
///      "repo" : {
///         "repo" : "awesome-compose",
///         "user" : "docker"
///      },
///      "repo_reference" : {
///         "branch" : "master"
///      },
///      "status" : "ComposeStarted"
///   }
/// ```
///
pub async fn show<'a>(AxumPath(id): AxumPath<String>) -> (StatusCode, impl IntoResponse) {
    // @TODO: Use the extractor here
    let uid = match Uuid::parse_str(&id) {
        Ok(uid) => uid,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ShowResponse {
                    error: Some(format!("Malformed Id: {}. Error: {}", id, err)),
                    inner: None,
                }),
            )
        }
    };

    {
        let hash = agent_state().read().unwrap();
        match hash.get(&uid) {
            Some(ctx) => (
                StatusCode::OK,
                Json(ShowResponse {
                    error: None,
                    inner: Some(ctx.clone()),
                }),
            ),
            None => (
                StatusCode::NOT_FOUND,
                Json(ShowResponse {
                    error: Some(format!("Not Found Id: {}", id)),
                    inner: None,
                }),
            ),
        }
    }
}

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

#[cfg(test)]
mod test {
    use crate::test_utils::*;

    use super::*;
    use axum::http::Request;
    use hyper::body::Body;

    fn simple_new_compose_request_data(branch: &str) -> CreateRequest {
        CreateRequest {
            github_user: String::from("docker"),
            github_repo: String::from("awesome-compose"),
            path: String::from("traefik-golang/compose.yaml"),
            reference: crate::git_manager::GitReference::Branch(branch.to_string()),
            token: Some(String::from("token")),
        }
    }

    async fn create_request(server_test: &ServerTest, body: Body) -> hyper::Response<Body> {
        server_test
            .client
            .request(
                Request::builder()
                    .method("POST")
                    .uri(server_test.url_for("/compose"))
                    .header("Content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn create_response_from_body(body: Body) -> CreateResponse {
        let body_bytes = hyper::body::to_bytes(body).await.unwrap();
        let data = String::from_utf8(body_bytes.to_vec()).unwrap();
        serde_json::from_str(&data).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create() {
        let server_test = ServerTest::new().await;

        let response = create_request(
            &server_test,
            Body::from(serde_json::to_string(&simple_new_compose_request_data("main")).unwrap()),
        )
        .await;

        let headers = response.headers();
        let location = headers
            .get("Location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(location, String::from(""));
        assert!(location.to_string().starts_with("/compose/"));
        let location_id = &location[9..];

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let response = create_response_from_body(response.into_body()).await;

        assert_eq!(format!("{}", response.id.unwrap()), location_id.to_string());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_parameter_error() {
        let server_test = ServerTest::new().await;

        let response = create_request(&server_test, Body::from("INCORRECT_BODY")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = create_response_from_body(response.into_body()).await;
        assert_ne!(response.error.unwrap(), "");
    }

    async fn show_request(server_test: &ServerTest, id: Uuid) -> hyper::Response<Body> {
        server_test
            .client
            .request(
                Request::builder()
                    .method("GET")
                    .uri(server_test.url_for(&format!("/compose/{}", id)))
                    .header("Content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn show_response_from_body<'a>(body: Body) -> ShowResponse<'a> {
        let body_bytes = hyper::body::to_bytes(body).await.unwrap();
        let data = String::from_utf8(body_bytes.to_vec()).unwrap();
        serde_json::from_str(&data).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn show_of_a_created_compose() {
        let server_test = ServerTest::new().await;

        let response = create_request(
            &server_test,
            Body::from(serde_json::to_string(&simple_new_compose_request_data("main")).unwrap()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let response = create_response_from_body(response.into_body()).await;
        let id = response.id.unwrap();
        let response = show_request(&server_test, id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = show_response_from_body(response.into_body()).await;
        assert_eq!(response.inner.unwrap().id(), id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn show_with_incorrect_uuid() {
        let server_test = ServerTest::new().await;

        let response = server_test
            .client
            .request(
                Request::builder()
                    .method("GET")
                    .uri(server_test.url_for("/compose/NOT_AN_UUID"))
                    .header("Content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = show_response_from_body(response.into_body()).await;
        assert_ne!(response.error.unwrap(), String::from(""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn show_with_missing_uuid() {
        let server_test = ServerTest::new().await;

        let response = show_request(&server_test, Uuid::new_v4()).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
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
