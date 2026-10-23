use crate::{
    authn::{self, AuthError},
    config::Config,
    oauth::{AuthorizationMode, OAuthManager},
};
use std::{
    io::{BufRead, Read, Write},
    time::Duration,
};
use tokio::sync::oneshot;

const LOGIN_TIMEOUT_SECS: u64 = 600;

#[derive(clap::Args, Clone)]
pub struct LoginOptions {
    /// Print the URL instead of opening the browser; still use a localhost callback.
    #[arg(long)]
    pub no_browser: bool,
    /// Use a pasted browser handoff instead of starting a localhost callback server.
    #[arg(long)]
    pub manual: bool,
}

pub async fn run(config: &Config, options: &LoginOptions) -> Result<(), String> {
    run_login(config, options)
        .await
        .map_err(|error| error.to_string())
}

async fn run_login(config: &Config, options: &LoginOptions) -> Result<(), AuthError> {
    if config.auth_file.as_os_str().is_empty() {
        return Err(authn::failure(
            "Login requires an authentication file; remove --no-auth-file or supply --auth-file",
        ));
    }
    let manager = OAuthManager::new(config);
    let mode = if options.manual {
        AuthorizationMode::Manual
    } else {
        AuthorizationMode::Loopback
    };
    let pending = manager.begin_authorization(mode).await?;
    println!(
        "Open this URL to authorize Kittengrid Agent:\n{}",
        pending.url
    );
    if options.manual {
        println!("After authorizing, paste the complete JSON handoff here and press Enter:");
    }
    std::io::stdout()
        .flush()
        .map_err(|_| authn::failure("Cannot print authorization URL"))?;
    if !options.no_browser && !options.manual {
        open_browser(pending.url.to_string()).await;
    }
    let handoff = options.manual.then(manual_input);
    // Dropping the pending authorization on timeout/cancellation also stops
    // its callback listener. The stdin thread cannot hold runtime shutdown open.
    let credentials = tokio::select! {
        result = tokio::time::timeout(
            Duration::from_secs(LOGIN_TIMEOUT_SECS),
            manager.complete_authorization(pending, handoff),
        ) => result.map_err(|_| authn::failure("Login timed out; run login again"))??,
        _ = tokio::signal::ctrl_c() => return Err(authn::failure("Login cancelled")),
    };
    authn::save_login(&config.auth_file, credentials).await?;
    println!(
        "Logged in. OAuth credentials saved to {}.",
        config.auth_file.display()
    );
    if !config.api_key.is_empty() {
        eprintln!("An API key is configured in flags/environment/YAML and will still take priority over OAuth.");
    }
    Ok(())
}

async fn open_browser(url: String) {
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(webbrowser::open(&url));
    });
    let opened = tokio::time::timeout(Duration::from_secs(10), receiver).await;
    if !matches!(opened, Ok(Ok(Ok(())))) {
        eprintln!("Could not open your browser. Open the URL above yourself.");
    }
}

fn manual_input() -> oneshot::Receiver<Result<String, AuthError>> {
    let (sender, receiver) = oneshot::channel();
    // Tokio's stdin uses an uncancellable blocking worker which can prevent
    // runtime shutdown on timeout. An unjoined OS thread does not hold it open.
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        let result = std::io::BufReader::new(stdin.lock().take(16385))
            .read_line(&mut line)
            .map(|_| line)
            .map_err(|_| authn::failure("Cannot read manual handoff"));
        let _ = sender.send(result);
    });
    receiver
}
