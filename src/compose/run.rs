use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::time::{sleep, Duration};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RunState {
    Idle,
    FetchingRepo,
    RepoReady,
    ErrorFetchingRepo,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Run {
    pub repo: String,
    pub paths: Vec<String>,
    pub state: RunState,
}

#[derive(Debug)]
struct RepoReady {}

impl Run {
    pub fn new(repo: String, paths: Vec<String>) -> Self {
        Run {
            state: RunState::Idle,
            repo,
            paths,
        }
    }

    pub async fn fetch_repo(&mut self, ctx: crate::compose::Context) {
        let repo = self.repo.clone();
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
        let repo = String::from("https://github.com/kittengrid/kittengrid-agent.git");
        let run = Run::new(repo, Vec::from(["docker-compose.yaml".to_string()]));
        assert!(matches!(run.state, RunState::Idle));
    }

    async fn fetch_repo() {
        let repo = String::from("https://github.com/kittengrid/kittengrid-agent.git");
        let run = Run::new(repo, Vec::from(["docker-compose.yaml".to_string()]));

        assert!(matches!(run.state, RunState::FetchingRepo { .. }));

        let ten_millis = Duration::from_secs(2);
        sleep(ten_millis);
        assert!(matches!(run.state, RunState::RepoReady));
    }
}
