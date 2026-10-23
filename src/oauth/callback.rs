use crate::authn::{self, AuthError};
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub(super) struct CallbackListener {
    pub redirect_uri: String,
    pub receiver: oneshot::Receiver<Result<String, AuthError>>,
    pub server: tokio::task::JoinHandle<()>,
}

pub(super) async fn listen(state: String) -> Result<CallbackListener, AuthError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| authn::failure("Cannot bind localhost callback; use login --manual"))?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener
            .local_addr()
            .map_err(|_| authn::failure("Cannot inspect callback listener"))?
            .port()
    );
    let (sender, receiver) = oneshot::channel();
    let context = Arc::new(CallbackState {
        state,
        sender: Mutex::new(Some(sender)),
    });
    let app = Router::new()
        .route("/oauth/callback", get(callback))
        .with_state(context);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(CallbackListener {
        redirect_uri,
        receiver,
        server,
    })
}

pub(super) fn manual_code(handoff: &str, expected_state: &str) -> Result<String, AuthError> {
    serde_json::from_str::<Callback>(handoff)
        .map_err(|_| authn::failure("Paste the complete JSON handoff shown in the browser"))?
        .code(expected_state)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

impl Callback {
    fn code(self, expected_state: &str) -> Result<String, AuthError> {
        if self.state != expected_state {
            return Err(authn::failure("Login state did not match; restart login"));
        }
        if self.error.is_some() {
            return Err(authn::failure("Authorization was denied or failed"));
        }
        self.code
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 8192
                    && !s.chars().any(|c| c.is_whitespace() || c.is_control())
            })
            .ok_or_else(|| authn::failure("Missing or invalid authorization code"))
    }
}

struct CallbackState {
    state: String,
    sender: Mutex<Option<oneshot::Sender<Result<String, AuthError>>>>,
}

async fn callback(
    State(context): State<Arc<CallbackState>>,
    Query(payload): Query<Callback>,
) -> impl IntoResponse {
    // Invalid state must not consume the pending login or stop its listener.
    let valid_state = payload.state == context.state;
    let result = payload.code(&context.state);
    let success = result.is_ok();
    if valid_state {
        if let Some(sender) = context.sender.lock().unwrap().take() {
            let _ = sender.send(result);
        }
    }
    let status = if success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; frame-ancestors 'none'",
            ),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        Html(if success {
            "Authorization received. Return to your terminal to finish login."
        } else {
            "Invalid or denied authorization. Return to your terminal."
        }),
    )
}
