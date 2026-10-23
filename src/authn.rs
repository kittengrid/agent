use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AuthError {
    #[error("Failed to read authentication file '{}': {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("Authentication file '{}' must be a regular file", .0.display())]
    NotAFile(PathBuf),
    #[cfg(unix)]
    #[error("Authentication file '{}' must be private; restrict it with chmod 600", .0.display())]
    InsecurePermissions(PathBuf),
    // Do not retain serde's error text: unexpected field names or values can
    // themselves be secrets. Report the location without dumping JSON data.
    #[error("Invalid authentication file '{}': expected JSON with api_key (line {line}, column {column})", path.display())]
    InvalidJson {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    #[error("Authentication file '{}': api_key must be nonempty and contain no whitespace or control characters", .0.display())]
    InvalidKey(PathBuf),
    #[error("{0}")]
    OAuth(String),
}

// Intentionally no Debug implementation: credentials must not be dumped.
#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthCredentials>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OAuthCredentials {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

pub(crate) fn failure(message: &str) -> AuthError {
    AuthError::OAuth(message.into())
}

fn valid_secret(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub(crate) fn load_api_key(path: &Path, required: bool) -> Result<Option<String>, AuthError> {
    Ok(load(path, required)?.and_then(|auth| auth.api_key))
}

pub(crate) fn load(path: &Path, required: bool) -> Result<Option<AuthFile>, AuthError> {
    let read_error = |source| AuthError::Read {
        path: path.into(),
        source,
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(source) if !required && source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(read_error(source)),
    };
    let metadata = file.metadata().map_err(read_error)?;
    if !metadata.is_file() {
        return Err(AuthError::NotAFile(path.into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AuthError::InsecurePermissions(path.into()));
        }
    }
    let auth: AuthFile =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| AuthError::InvalidJson {
            path: path.into(),
            line: error.line(),
            column: error.column(),
        })?;
    if let Some(key) = &auth.api_key {
        if !valid_secret(key) {
            return Err(AuthError::InvalidKey(path.into()));
        }
    }
    if let Some(oauth) = &auth.oauth {
        if !valid_secret(&oauth.client_id)
            || !valid_secret(&oauth.access_token)
            || !valid_secret(&oauth.refresh_token)
        {
            return Err(failure(
                "Invalid saved OAuth credentials; run kittengrid-agent login again",
            ));
        }
    }
    if auth.api_key.is_none() && auth.oauth.is_none() {
        return Err(AuthError::InvalidJson {
            path: path.into(),
            line: 0,
            column: 0,
        });
    }
    Ok(Some(auth))
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn prepare_parent(path: &Path) -> Result<(), AuthError> {
    if path.as_os_str().is_empty() {
        return Err(failure(
            "No authentication file path available; supply --auth-file",
        ));
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(parent(path))
        .map_err(|_| failure("Cannot create authentication directory"))
}

// Cross-process serialization matters because refresh tokens rotate. The file
// must be reread after taking this lock, not refreshed from a stale snapshot.
pub(crate) async fn lock(path: &Path) -> Result<filelock::OwnedFileLockGuard, AuthError> {
    prepare_parent(path)?;
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(lock_path)
        .map_err(|_| failure("Cannot open authentication lock file"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file
            .metadata()
            .map_err(|_| failure("Cannot inspect authentication lock file"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(failure(
                "Authentication lock file must be private (chmod 600)",
            ));
        }
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        filelock::FileLock::from_file(file).lock_owned_async(),
    )
    .await
    .map_err(|_| failure("Timed out waiting for authentication file lock"))?
    .map_err(|_| failure("Cannot lock authentication file"))
}

pub(crate) fn save(path: &Path, auth: &AuthFile) -> Result<(), AuthError> {
    prepare_parent(path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent(path))
        .map_err(|_| failure("Cannot create private authentication file"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| failure("Cannot restrict authentication file permissions"))?;
    }
    serde_json::to_writer_pretty(&mut temporary, auth)
        .map_err(|_| failure("Cannot encode authentication credentials"))?;
    temporary
        .write_all(b"\n")
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| failure("Cannot persist authentication credentials"))?;
    temporary.persist(path).map_err(|_| {
        failure("Cannot replace authentication file; login again after fixing file permissions")
    })?;
    Ok(())
}

pub(crate) async fn save_login(path: &Path, oauth: OAuthCredentials) -> Result<(), AuthError> {
    let _lock = lock(path).await?;
    save(
        path,
        &AuthFile {
            api_key: None,
            oauth: Some(oauth),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_auth(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[tokio::test]
    async fn lock_wait_is_cancellable_and_guard_drop_releases_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authn.json");
        let guard = lock(&path).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), lock(&path),)
                .await
                .is_err()
        );
        drop(guard);
        let _guard = tokio::time::timeout(std::time::Duration::from_secs(1), lock(&path))
            .await
            .expect("lock should be released when the guard is dropped")
            .unwrap();
    }

    #[test]
    fn loads_saved_api_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authn.json");
        write_auth(&path, r#"{"api_key":"private-key"}"#);
        assert_eq!(
            load_api_key(&path, true).unwrap().as_deref(),
            Some("private-key")
        );
    }

    #[test]
    fn missing_discovered_file_is_optional_but_explicit_file_is_required() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.json");
        assert!(load_api_key(&path, false).unwrap().is_none());
        assert!(matches!(
            load_api_key(&path, true),
            Err(AuthError::Read { .. })
        ));
    }

    #[test]
    fn malformed_or_unknown_json_fields_never_echo_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authn.json");
        for invalid in [
            r#"{"private-secret-field":"private-secret-value"}"#,
            "{}",
            r#"{"api_key":123456789}"#,
            "{broken-json-private-secret-value",
        ] {
            write_auth(&path, invalid);
            let error = load_api_key(&path, true).unwrap_err();
            assert!(matches!(error, AuthError::InvalidJson { .. }));
            let output = format!("{error} {error:?}");
            for secret in ["private-secret-field", "private-secret-value", "123456789"] {
                assert!(!output.contains(secret), "{output}");
            }
        }
    }

    #[test]
    fn rejects_empty_or_invalid_header_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authn.json");
        for key in [
            "",
            " ",
            "private-key\n",
            "private key",
            "private-key\u{0000}",
        ] {
            write_auth(&path, &serde_json::json!({"api_key":key}).to_string());
            assert!(matches!(
                load_api_key(&path, true),
                Err(AuthError::InvalidKey(_))
            ));
        }
    }

    #[test]
    fn rejects_directories() {
        let directory = tempfile::tempdir().unwrap();
        assert!(load_api_key(directory.path(), true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_or_world_accessible_credential_files() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authn.json");
        write_auth(&path, r#"{"api_key":"private-key"}"#);
        for mode in [0o644, 0o640, 0o660] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            let error = load_api_key(&path, true).unwrap_err();
            assert!(matches!(error, AuthError::InsecurePermissions(_)));
            assert!(error.to_string().contains("chmod 600"));
        }
        for mode in [0o600, 0o400] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                load_api_key(&path, true).unwrap().as_deref(),
                Some("private-key")
            );
        }
    }
}
