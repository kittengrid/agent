#[cfg(test)]
use crate::data_dir::DataDir;
use std::env;
use std::process::Command;
use tempfile::{tempdir, TempDir};

pub fn temp_data_dir() -> (TempDir, DataDir) {
    let directory = tempdir().unwrap();
    let mut data_dir = DataDir::new(directory.path().to_path_buf());
    data_dir.init().unwrap();

    (directory, data_dir)
}

// Creates an empty repo, needs git binary.
pub fn git_empty_repo() -> TempDir {
    let target_dir: TempDir = tempdir().unwrap();
    env::set_current_dir(&target_dir).expect("can change to tempdir");
    Command::new("git")
        .arg("init")
        .output()
        .expect("git init ok");
    Command::new("git")
        .arg("branch")
        .arg("main")
        .output()
        .expect("git branch ok");

    target_dir
}

pub fn git_commit_all(temp_dir: &TempDir) -> String {
    env::set_current_dir(temp_dir).expect("can change to tempdir");
    let output = Command::new("git")
        .arg("add")
        .arg(".")
        .output()
        .expect("git add ok");

    let output = Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Files added")
        .output()
        .expect("git commit ok");

    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .expect("git rev-parse ok");

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
