use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub path: String,
    pub repository: String,
    pub branch: String,
}
impl Identity {
    pub fn key(&self) -> Result<String, String> {
        let encoded = serde_json::to_vec(&[&self.path, &self.repository, &self.branch])
            .map_err(|_| "Cannot encode workspace identity")?;
        Ok(Sha256::digest(encoded)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Workspace {
    #[serde(flatten)]
    pub identity: Identity,
    pub environment_id: String,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    workspaces: BTreeMap<String, Workspace>,
}
pub(crate) struct Store {
    path: PathBuf,
    state: State,
    _lock: filelock::OwnedFileLockGuard,
}

impl Store {
    pub async fn open(path: PathBuf) -> Result<Self, String> {
        if path.as_os_str().is_empty() {
            return Err("Workspace state path cannot be empty".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut directory = std::fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory
            .create(parent)
            .map_err(|_| "Cannot create workspace state directory")?;
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| "Cannot open workspace state lock")?;
        let lock = tokio::time::timeout(
            Duration::from_secs(90),
            filelock::FileLock::from_file(lock).lock_owned_async(),
        )
        .await
        .map_err(|_| "Workspace state is busy; try again")?
        .map_err(|_| "Cannot lock workspace state")?;
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
                "Invalid workspaces.json; back it up and remove it to reset workspace mappings"
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(_) => return Err("Cannot read workspace state".into()),
        };
        Ok(Self {
            path,
            state,
            _lock: lock,
        })
    }
    pub fn get(&self, identity: &Identity) -> Result<Option<&str>, String> {
        Ok(self
            .state
            .workspaces
            .get(&identity.key()?)
            .filter(|w| w.identity == *identity)
            .map(|w| w.environment_id.as_str()))
    }
    pub fn save(&mut self, identity: Identity, environment_id: String) -> Result<(), String> {
        self.state.workspaces.insert(
            identity.key()?,
            Workspace {
                identity,
                environment_id,
            },
        );
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Cannot create workspace state file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect workspace state")?;
        }
        serde_json::to_writer_pretty(&mut file, &self.state)
            .map_err(|_| "Cannot encode workspace state")?;
        file.write_all(b"\n")
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| "Cannot persist workspace state")?;
        file.persist(&self.path)
            .map_err(|_| "Cannot replace workspace state")?;
        Ok(())
    }
}

pub(crate) async fn identity(
    project: &crate::development::ResolvedProject,
) -> Result<Identity, String> {
    let cwd = std::env::current_dir().map_err(|_| "Cannot find working directory")?;
    let (path, branch) = match crate::repository::git(&cwd, &["rev-parse", "--show-toplevel"]).await
    {
        Ok(root) => {
            let branch =
                match crate::repository::git(&cwd, &["symbolic-ref", "--quiet", "HEAD"]).await {
                    Ok(reference) => reference
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&reference)
                        .to_owned(),
                    Err(_) => format!(
                        "detached:{}",
                        crate::repository::git(&cwd, &["rev-parse", "--verify", "HEAD"]).await?
                    ),
                };
            (PathBuf::from(root), branch)
        }
        Err(error) => {
            if cwd.ancestors().any(|dir| dir.join(".git").exists()) {
                return Err(error);
            }
            (cwd, "directory".into())
        }
    };
    let path = path
        .canonicalize()
        .map_err(|_| "Cannot canonicalize checkout directory")?
        .into_os_string()
        .into_string()
        .map_err(|_| "Workspace path must be valid UTF-8")?;
    Ok(Identity {
        path,
        repository: format!(
            "{}:{}",
            project.vcs_provider.to_lowercase(),
            project.vcs_path.to_lowercase()
        ),
        branch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn state_is_atomic_and_branch_specific() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspaces.json");
        let first = Identity {
            path: "/checkout".into(),
            repository: "github:team/app".into(),
            branch: "feature/a".into(),
        };
        let second = Identity {
            branch: "feature/b".into(),
            ..first.clone()
        };
        assert_ne!(first.key().unwrap(), second.key().unwrap());
        {
            let mut store = Store::open(path.clone()).await.unwrap();
            store.save(first.clone(), "environment".into()).unwrap();
        }
        let store = Store::open(path).await.unwrap();
        assert_eq!(store.get(&first).unwrap(), Some("environment"));
        assert!(store.get(&second).unwrap().is_none());
    }
}
