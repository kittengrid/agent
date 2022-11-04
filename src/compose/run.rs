use rocket::serde::{Deserialize, Serialize};
use std::{thread, time};

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

    pub fn fetch_repo(&mut self) {
        let repo = self.repo.clone();
        let fetching_thread = thread::spawn(move || {
            println!("I am fetching the repo, please wait");
            let ten_millis = time::Duration::from_secs(1);
            thread::sleep(ten_millis);
            println!("I have fetched the repo");
        });
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

    fn fetch_repo() {
        let repo = String::from("https://github.com/kittengrid/kittengrid-agent.git");
        let mut run = Run::new(repo, Vec::from(["docker-compose.yaml".to_string()]));
        run.fetch_repo();
        assert!(matches!(run.state, RunState::FetchingRepo { .. }));

        let ten_millis = time::Duration::from_secs(2);
        thread::sleep(ten_millis);
        assert!(matches!(run.state, RunState::RepoReady));
    }
}
