#[cfg(test)]
use crate::data_dir::DataDir;
use log::debug;

use std::process::{Command, Output};
use std::{thread, time};
use tempfile::{tempdir, TempDir};

#[allow(dead_code)]
pub fn temp_data_dir() -> (TempDir, DataDir) {
    let directory = tempdir().unwrap();
    let mut data_dir = DataDir::new(directory.path().to_path_buf());
    data_dir.init().unwrap();

    (directory, data_dir)
}

pub fn debug_output(output: &Output) {
    debug!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    );
}

// Creates an empty repo, needs git binary.
pub fn git_empty_repo() -> TempDir {
    let target_dir: TempDir = tempdir().unwrap();
    let path = target_dir.path().to_str().unwrap();
    debug!("Creating empty repo in {:?}", target_dir);
    let output = git_command()
        .arg("init")
        .arg("-b")
        .arg("main")
        .arg(path)
        .output()
        .expect("git init ok");
    debug_output(&output);
    let output = Command::new("touch")
        .arg(format!("{}/.keepme", path))
        .output()
        .expect("git touch ok");
    debug_output(&output);

    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("config")
        .arg("receive.denyCurrentBranch")
        .arg("false")
        .output()
        .expect("git config ok");
    debug_output(&output);

    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("add")
        .arg(".")
        .output()
        .expect("git add ok");
    debug_output(&output);
    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .output()
        .expect("git commit ok");
    debug_output(&output);

    target_dir
}

pub fn git_commit_all(temp_dir: &TempDir) -> String {
    let path = temp_dir.path().to_str().unwrap();

    git_command()
        .arg("-C")
        .arg(path)
        .arg("add")
        .arg(".")
        .output()
        .expect("git add ok");

    git_command()
        .arg("-C")
        .arg(path)
        .arg("commit")
        .arg("-m")
        .arg("Files added")
        .output()
        .expect("git commit ok");

    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .expect("git rev-parse ok");

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn git_clone(source_repo: &str) -> TempDir {
    let target_dir: TempDir = tempdir().unwrap();
    let path = target_dir.path().to_str().unwrap();
    let output = git_command()
        .arg("clone")
        .arg(source_repo)
        .arg(path)
        .output()
        .expect("git clone ok");
    debug_output(&output);
    target_dir
}

pub fn git_commit_amend_and_push(temp_dir: &TempDir) {
    let path = temp_dir.path().to_str().unwrap();
    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("add")
        .arg(".")
        .output()
        .expect("git add ok");
    debug_output(&output);

    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("commit")
        .arg("--amend")
        .arg("--no-edit")
        .output()
        .expect("git commit ok");
    debug_output(&output);

    let output = git_command()
        .arg("-C")
        .arg(path)
        .arg("push")
        .arg("origin")
        .arg("-f")
        .arg("main")
        .output()
        .expect("git force push ok");
    debug_output(&output);
}

fn git_command() -> Command {
    let mut command = Command::new("git");

    command
        .env("GIT_AUTHOR_NAME", "ci")
        .env("GIT_AUTHOR_EMAIL", "ci@kittengrid.com")
        .env("GIT_COMMITTER_NAME", "ci")
        .env("GIT_COMMITTER_EMAIL", "ci@kittengrid.com")
        .env("EMAIL", "ci@kittengrid.com");

    command
}
#[allow(dead_code)]
pub fn sleep(secs: u64) {
    let secs = time::Duration::from_secs(secs);
    thread::sleep(secs);
}
