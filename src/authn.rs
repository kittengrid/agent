use serde::Deserialize;
use std::{
    fs::File,
    io::{self, BufReader},
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
}

// Intentionally no Debug implementation: credentials must not be dumped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthFile {
    api_key: String,
}

pub(crate) fn load_api_key(path: &Path, required: bool) -> Result<Option<String>, AuthError> {
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
    if auth.api_key.is_empty()
        || auth
            .api_key
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(AuthError::InvalidKey(path.into()));
    }
    Ok(Some(auth.api_key))
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
