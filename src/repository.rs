use std::path::Path;
use tokio::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub provider: String,
    pub path: String,
}

impl RepositoryIdentity {
    pub fn from_remote(remote: &str) -> Result<Self, String> {
        let remote = remote.trim();
        let (host, path) = if remote.contains("://") {
            let url = url::Url::parse(remote).map_err(|_| "Invalid Git remote URL")?;
            if !matches!(url.scheme(), "https" | "http" | "ssh" | "git")
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("Unsupported Git remote URL".into());
            }
            // Only extract repository identity. Never retain or forward embedded
            // credentials, and never include the original remote in errors.
            (
                url.host_str().unwrap_or_default().to_ascii_lowercase(),
                url.path().to_owned(),
            )
        } else {
            let (authority, path) = remote
                .split_once(':')
                .ok_or("Expected a GitHub/GitLab SSH or HTTPS remote")?;
            (
                authority
                    .rsplit('@')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                path.to_owned(),
            )
        };
        let provider = match host.as_str() {
            "github.com" => "github",
            "gitlab.com" => "gitlab",
            _ => return Err("Cannot infer the Git provider; use --project-id or configure --vcs-provider and --project-vcs-path".into()),
        };
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let segments: Vec<_> = path.split('/').collect();
        if segments.len() < 2
            || (provider == "github" && segments.len() != 2)
            || segments.iter().any(|segment| {
                segment.is_empty()
                    || matches!(*segment, "." | "..")
                    || !segment
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            })
        {
            return Err("Invalid repository path in Git remote".into());
        }
        Ok(Self {
            provider: provider.into(),
            path: path.to_owned(),
        })
    }

    pub async fn discover(directory: &Path, remote: &str) -> Result<Self, String> {
        git(directory, &["rev-parse", "--show-toplevel"]).await?;
        // get-url also expands Git's url.*.insteadOf aliases, without networking.
        let url = git(directory, &["remote", "get-url", "--", remote]).await?;
        Self::from_remote(&url)
    }
}

pub(crate) async fn git(directory: &Path, arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .map_err(|_| "Git discovery timed out")?
        .map_err(|_| "Cannot run Git; use --project-id or --repository")?;
    if !output.status.success() {
        return Err(
            "Cannot discover this repository/remote; use --project-id, --repository, or --remote"
                .into(),
        );
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "Git returned invalid UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_ssh_and_https_and_drops_embedded_credentials() {
        for remote in [
            "git@github.com:Owner/Repo.git",
            "ssh://git@github.com/Owner/Repo.git",
            "https://github.com/Owner/Repo",
            "https://user:private-token@github.com/Owner/Repo.git/",
        ] {
            assert_eq!(
                RepositoryIdentity::from_remote(remote).unwrap(),
                RepositoryIdentity {
                    provider: "github".into(),
                    path: "Owner/Repo".into()
                }
            );
        }
    }

    #[test]
    fn preserves_nested_gitlab_groups() {
        assert_eq!(
            RepositoryIdentity::from_remote("git@gitlab.com:group/subgroup/repo.git")
                .unwrap()
                .path,
            "group/subgroup/repo"
        );
    }

    #[test]
    fn invalid_or_unknown_remotes_never_echo_sensitive_input() {
        for remote in [
            "private-token",
            "https://private-token@unknown.example/a/b",
            "https://github.com/a/b?private-token",
            "https://github.com/a",
            "git@github.com:a/../b",
            "https://github.com/a/b/c",
            "file:///private-token",
        ] {
            let error = RepositoryIdentity::from_remote(remote).unwrap_err();
            assert!(!error.contains("private-token"));
        }
    }
}
