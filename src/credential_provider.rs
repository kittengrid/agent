//! Resolves runtime credentials and persists rotating OAuth sessions.
//! OAuth requests themselves belong to OAuthManager; file operations to authn.
use crate::{
    authn::{self, AuthError},
    config::Config,
    oauth::OAuthManager,
};

pub(crate) async fn bearer(config: &Config) -> Result<String, AuthError> {
    if !config.api_key.is_empty() {
        return Ok(config.api_key.clone());
    }
    if config.auth_file.as_os_str().is_empty() || authn::load(&config.auth_file, false)?.is_none() {
        return Ok(String::new());
    }
    // Refresh tokens rotate: reread under the storage lock and keep it until
    // replacement credentials have been persisted.
    let _lock = authn::lock(&config.auth_file).await?;
    let mut auth = authn::load(&config.auth_file, true)?
        .ok_or_else(|| authn::failure("Saved credentials missing; run login again"))?;
    if let Some(key) = auth.api_key {
        return Ok(key);
    }
    config
        .oauth_backend()
        .map_err(|error| authn::failure(&error.to_string()))?;
    let oauth = auth
        .oauth
        .as_mut()
        .ok_or_else(|| authn::failure("No OAuth credentials; run login again"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if oauth.expires_at <= now.saturating_add(60) {
        *oauth = OAuthManager::new(config).refresh(oauth).await?;
        // Never send an access token if its rotating refresh token wasn't saved.
        authn::save(&config.auth_file, &auth)?;
    }
    Ok(auth.oauth.unwrap().access_token)
}
