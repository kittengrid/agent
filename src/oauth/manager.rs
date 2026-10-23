use super::callback;
use crate::{
    authn::{self, AuthError, OAuthCredentials},
    config::Config,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::TryRng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::sync::oneshot;

const OAUTH_CLIENT_ID: &str = "kittengrid-agent";

#[derive(Clone, Copy)]
pub(crate) enum AuthorizationMode {
    Loopback,
    Manual,
}

pub(crate) struct PendingAuthorization {
    pub(crate) url: url::Url,
    backend: url::Url,
    verifier: String,
    state: String,
    redirect_uri: String,
    receiver: Option<oneshot::Receiver<Result<String, AuthError>>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for PendingAuthorization {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

fn random_secret() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| authn::failure("Cannot obtain secure randomness"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Coordinates browser authorization, callbacks, token exchange, and refresh.
/// Returns credentials to callers without reading or writing credential files.
pub(crate) struct OAuthManager<'a> {
    config: &'a Config,
}

impl<'a> OAuthManager<'a> {
    pub(crate) fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub(crate) async fn begin_authorization(
        &self,
        mode: AuthorizationMode,
    ) -> Result<PendingAuthorization, AuthError> {
        let config = self.config;
        let backend = config
            .oauth_backend()
            .map_err(|error| authn::failure(&error.to_string()))?;
        let verifier = random_secret()?;
        let state = random_secret()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut server = None;
        let redirect_uri;
        let receiver;
        if matches!(mode, AuthorizationMode::Manual) {
            redirect_uri = backend
                .join("oauth/manual")
                .map_err(|_| authn::failure("Invalid manual callback URL"))?
                .to_string();
            receiver = None;
        } else {
            let listener = callback::listen(state.clone()).await?;
            redirect_uri = listener.redirect_uri;
            receiver = Some(listener.receiver);
            server = Some(listener.server);
        }
        let mut authorization = backend
            .join("oauth/authorize")
            .map_err(|_| authn::failure("Invalid authorization URL"))?;
        authorization.query_pairs_mut().extend_pairs([
            ("client_id", OAUTH_CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", &redirect_uri),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        Ok(PendingAuthorization {
            url: authorization,
            backend,
            verifier,
            state,
            redirect_uri,
            receiver,
            server,
        })
    }

    pub(crate) async fn complete_authorization(
        &self,
        mut pending: PendingAuthorization,
        manual_handoff: Option<oneshot::Receiver<Result<String, AuthError>>>,
    ) -> Result<OAuthCredentials, AuthError> {
        let code = if let Some(receiver) = pending.receiver.take() {
            receiver
                .await
                .map_err(|_| authn::failure("Login callback closed unexpectedly"))??
        } else {
            let handoff = manual_handoff
                .ok_or_else(|| authn::failure("Manual authorization requires a handoff"))?
                .await
                .map_err(|_| authn::failure("Manual handoff closed unexpectedly"))??;
            callback::manual_code(&handoff, &pending.state)?
        };
        let backend = &pending.backend;
        let redirect_uri = &pending.redirect_uri;
        let verifier = &pending.verifier;
        self.token(
            backend,
            OAUTH_CLIENT_ID,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", OAUTH_CLIENT_ID),
                ("code", &code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ],
        )
        .await
    }

    async fn token(
        &self,
        backend: &url::Url,
        client_id: &str,
        fields: &[(&str, &str)],
    ) -> Result<OAuthCredentials, AuthError> {
        let endpoint = backend
            .join("oauth/token")
            .map_err(|_| authn::failure("Invalid token endpoint"))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| authn::failure("Cannot initialize OAuth client"))?;
        let response = client
            .post(endpoint)
            .form(fields)
            .send()
            .await
            .map_err(|_| {
                authn::failure("OAuth token request failed; check connectivity and try login again")
            })?;
        if !response.status().is_success() {
            return Err(authn::failure(&format!(
                "OAuth token request rejected (HTTP {}); run kittengrid-agent login again",
                response.status().as_u16()
            )));
        }
        let token = response
            .json::<TokenResponse>()
            .await
            .map_err(|_| authn::failure("Invalid OAuth token response"))?;
        if !token.token_type.eq_ignore_ascii_case("bearer")
            || !valid_token(&token.access_token)
            || !valid_token(&token.refresh_token)
            || token.expires_in == 0
        {
            return Err(authn::failure("Invalid OAuth token response"));
        }
        let expires_at = now()
            .checked_add(token.expires_in)
            .ok_or_else(|| authn::failure("Invalid OAuth token lifetime"))?;
        Ok(OAuthCredentials {
            client_id: client_id.into(),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at,
        })
    }

    pub(crate) async fn refresh(
        &self,
        credentials: &OAuthCredentials,
    ) -> Result<OAuthCredentials, AuthError> {
        let backend = self
            .config
            .oauth_backend()
            .map_err(|error| authn::failure(&error.to_string()))?;
        self.token(
            &backend,
            &credentials.client_id,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &credentials.client_id),
                ("refresh_token", &credentials.refresh_token),
            ],
        )
        .await
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    token_type: String,
    expires_in: u64,
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
