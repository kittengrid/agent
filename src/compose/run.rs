use crate::data_dir::DataDir;
use crate::docker_compose::DockerCompose;
use crate::git_manager::{GitHubRepo, GitManager, GitReference};
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::time::{sleep, Duration};
use uuid::uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RunState {
    Idle,
    FetchingRepo,
    RepoReady,
    ErrorFetchingRepo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub repo: GitHubRepo,
    pub reference: GitReference,
    pub paths: Vec<String>,
    pub state: RunState,
}

#[derive(Debug)]
struct RepoReady {}

impl Run {
    pub fn new(repo: GitHubRepo, paths: Vec<String>, reference: GitReference) -> Self {
        Run {
            state: RunState::Idle,
            repo,
            paths,
            reference,
        }
    }

    pub async fn fetch_repo(&mut self) {
        let mut data_dir = DataDir::new("/var/lib/kittengrid-agent".into());
        data_dir.init().unwrap();
        let _docker_compose = DockerCompose::new(&data_dir);

        let git_manager = GitManager::new(&data_dir).unwrap();

        let repo = GitHubRepo::new("kittengrid", "deb-s3");
        git_manager.fetch_remote(&repo).unwrap();
        git_manager
            .clone_local_branch(&repo, "main", uuid!("f37915a0-7195-11ed-a1eb-0242ac120002"))
            .unwrap();
        let ten_seconds = Duration::from_secs(10);
        println!("HOLA");
        sleep(ten_seconds).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::matches;

    #[test]
    fn first_state() {
        let repo = GitHubRepo::new("kittengrid", "kittengrid-agent");
        let run = Run::new(
            repo,
            Vec::from(["docker-compose.yaml".to_string()]),
            GitReference::Branch("main".to_string()),
        );
        assert!(matches!(run.state, RunState::Idle));
    }

    async fn fetch_repo() {
        let repo = GitHubRepo::new("kittengrid", "kittengrid-agent");
        let run = Run::new(
            repo,
            Vec::from(["docker-compose.yaml".to_string()]),
            GitReference::Branch("main".to_string()),
        );

        assert!(matches!(run.state, RunState::FetchingRepo { .. }));

        let ten_millis = Duration::from_secs(2);
        sleep(ten_millis);
        assert!(matches!(run.state, RunState::RepoReady));
    }
}
