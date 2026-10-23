use clap_serde_derive::{
    clap::{self, Parser},
    ClapSerde,
};
use serde::{Deserialize, Serialize};

use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    #[error("Failed to open configuration file '{}': {source}", path.display())]
    Open { path: PathBuf, source: io::Error },
    #[error("Failed to parse configuration file '{}': {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("Global configuration file '{}' cannot define services; put services in the project configuration instead", .0.display())]
    GlobalServices(PathBuf),
    #[error(transparent)]
    Authentication(#[from] crate::authn::AuthError),
    #[error("Invalid api_url: expected an HTTP(S) server origin without userinfo, path, query, or fragment")]
    InvalidApiUrl,
    #[error("OAuth requires an HTTPS api_url (HTTP is allowed for localhost)")]
    InsecureOAuthApiUrl,
}

struct ParsedConfig {
    config: Config,
    command: Option<Command>,
}

// Returns a reference to a lazily created Config object.
// TODO: FIX TESTS ARGUMENTS
static CONFIG: Lazy<ParsedConfig> = Lazy::new(|| {
    let mut args;

    if cfg!(test) {
        args = Args::parse_from([
            "kittengrid-agent",
            "--no-global-config",
            "--no-auth-file",
            "--config",
            "kittengrid.test.yml",
        ]);
    } else {
        use clap::{CommandFactory, FromArgMatches};
        // Root settings are also accepted after `login`.
        let matches = Args::command()
            .mut_args(|arg| arg.global(true))
            .get_matches();
        args = Args::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    }

    let command = args.command.clone();
    let config = process_args(&mut args).unwrap_or_else(|error| {
        // Configuration is loaded before the logger is initialized.
        eprintln!("{error}");
        std::process::exit(1);
    });
    ParsedConfig { config, command }
});

fn configuration_directory() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("kittengrid-agent"))
}

fn default_global_config_path() -> Option<PathBuf> {
    configuration_directory().map(|directory| directory.join("config.yaml"))
}

fn read_config(
    path: &Path,
    required: bool,
) -> Result<Option<<Config as ClapSerde>::Opt>, ConfigError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(source) if !required && source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Open {
                path: path.into(),
                source,
            })
        }
    };
    // An empty file or YAML null is an empty layer, not a missing file.
    let parsed: Option<<Config as ClapSerde>::Opt> = serde_yaml::from_reader(BufReader::new(file))
        .map_err(|source| ConfigError::Parse {
            path: path.into(),
            source,
        })?;
    Ok(Some(parsed.unwrap_or_default()))
}

fn process_args(args: &mut Args) -> Result<Config, ConfigError> {
    let mut config = Config::default();

    if !args.no_global_config {
        let global_path = args
            .global_config_path
            .clone()
            .or_else(default_global_config_path);
        if let Some(path) = global_path {
            if let Some(settings) = read_config(&path, args.global_config_path.is_some())? {
                if settings.services.is_some() {
                    return Err(ConfigError::GlobalServices(path));
                }
                config.update(settings);
            }
        }
    }

    if let Some(path) = &args.config_path {
        if let Some(settings) = read_config(path, true)? {
            config.update(settings);
        }
    } else {
        for path in ["kittengrid.yml", "kittengrid.yaml"] {
            if let Some(settings) = read_config(Path::new(path), false)? {
                config.update(settings);
                break;
            }
        }
    }

    // Clap resolves flags over environment variables. Only supplied options
    // override the file layers, so explicit false and zero values are retained.
    config.update(&mut args.config);
    let explicit_auth_file = !config.auth_file.as_os_str().is_empty();
    config.set_defaults_if_missing();
    config.validate()?;

    // Saved credentials are a fallback, never an override of explicitly
    // configured keys. An unused auth file is not opened or validated.
    if args.no_auth_file {
        config.auth_file.clear();
    }
    if args.command.is_none()
        && !args.no_auth_file
        && config.api_key.is_empty()
        && !config.auth_file.as_os_str().is_empty()
    {
        if let Some(api_key) = crate::authn::load_api_key(&config.auth_file, explicit_auth_file)? {
            config.api_key = api_key;
        }
    }
    Ok(config)
}

pub fn get_config() -> &'static Config {
    &CONFIG.config
}

pub fn get_command() -> Option<&'static Command> {
    CONFIG.command.as_ref()
}

#[derive(clap::Subcommand, Clone)]
pub enum Command {
    /// Authorize this agent through your browser.
    Login(crate::login::LoginOptions),
}
// Inspiration from https://stackoverflow.com/questions/55133351/is-there-a-way-to-get-clap-to-use-default-values-from-a-file

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Project YAML configuration (overrides global settings; flags and environment variables take precedence).
    #[clap(short, long = "config", env = "KITTENGRID_CONFIG_PATH")]
    config_path: Option<PathBuf>,

    /// Global YAML settings file (defaults to the platform configuration directory).
    #[arg(long = "global-config", env = "KITTENGRID_GLOBAL_CONFIG")]
    global_config_path: Option<PathBuf>,

    /// Ignore global settings, including an explicitly selected global file.
    #[arg(long)]
    no_global_config: bool,

    /// Ignore saved credentials, including an explicitly selected authentication file.
    #[arg(long)]
    no_auth_file: bool,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Rest of arguments
    #[clap(flatten)]
    pub config: <Config as ClapSerde>::Opt,
}

impl Config {
    fn validate(&self) -> Result<(), ConfigError> {
        self.backend().map(|_| ())
    }

    pub(crate) fn backend(&self) -> Result<url::Url, ConfigError> {
        let url = url::Url::parse(&self.api_url).map_err(|_| ConfigError::InvalidApiUrl)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ConfigError::InvalidApiUrl);
        }
        Ok(url)
    }

    pub(crate) fn oauth_backend(&self) -> Result<url::Url, ConfigError> {
        let url = self.backend()?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        if url.scheme() != "https" && !(url.scheme() == "http" && local) {
            return Err(ConfigError::InsecureOAuthApiUrl);
        }
        Ok(url)
    }

    fn set_defaults_if_missing(&mut self) -> &mut Self {
        if self.log_level.is_empty() {
            self.log_level = "info".to_string();
        }
        if self.work_directory.is_empty() {
            self.work_directory = "/var/lib/kittengrid-agent".to_string();
        }
        if self.bind_address.is_empty() {
            self.bind_address = "0.0.0.0".to_string();
        }
        if self.auth_file.as_os_str().is_empty() {
            self.auth_file = configuration_directory()
                .map(|directory| directory.join("authn.json"))
                .unwrap_or_default();
        }
        self
    }
}

#[derive(Parser, Clone, ClapSerde)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Log level (error, warn, info, debug and trace), defaults to info
    #[arg(short, long, env("KITTENGRID_LOG_LEVEL"))]
    pub log_level: String,

    #[arg(long, env("KITTENGRID_SHOW_SERVICES_OUTPUT"))]
    pub show_services_output: bool,

    #[arg(short, long, env("KITTENGRID_WORK_DIR"))]
    pub work_directory: String,

    /// Bind address for the agent. [default: 0.0.0.0]
    #[arg(long, env("KITTENGRID_BIND_ADDRESS"))]
    pub bind_address: String,

    /// Bind port for the agent. [default: 0]
    #[arg(long, env("KITTENGRID_BIND_PORT"))]
    pub bind_port: u16,

    #[arg(long, env("KITTENGRID_API_KEY"), hide_env_values = true)]
    pub api_key: String,

    #[arg(long, env("KITTENGRID_API_URL"))]
    pub api_url: String,

    /// Saved authentication JSON (defaults to the platform config directory's authn.json).
    #[arg(long, env("KITTENGRID_AUTH_FILE"))]
    pub auth_file: PathBuf,

    /// Existing environment's public ID. When set, replaces the VCS/PR identity.
    #[arg(long, env("KITTENGRID_ENVIRONMENT_ID"))]
    pub environment_id: String,

    #[arg(long, env("KITTENGRID_VCS_PROVIDER"))]
    pub vcs_provider: String,

    #[arg(long, env("KITTENGRID_PULL_REQUEST_VCS_ID"))]
    pub pull_request_vcs_id: String,

    #[arg(long, env("KITTENGRID_PROJECT_VCS_PATH"))]
    pub project_vcs_path: String,

    #[arg(long, env("KITTENGRID_WORKFLOW_RUN_ID"))]
    pub workflow_run_id: String,

    /// Starts services defined in the configuration file [default: false]
    #[arg(long, env("KITTENGRID_START_SERVICES"))]
    pub start_services: bool,

    #[arg(long, env("KITTENGRID_LAST_COMMIT_SHA"))]
    pub last_commit_sha: String,

    /// Whether to start a terminal service using ttyd [default: false]
    #[arg(long, env("KITTENGRID_START_TERMINAL"))]
    pub start_terminal: bool,

    #[clap(skip)]
    pub services: Vec<ServiceConfig>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("log_level", &self.log_level)
            .field("show_services_output", &self.show_services_output)
            .field("work_directory", &self.work_directory)
            .field("bind_address", &self.bind_address)
            .field("bind_port", &self.bind_port)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    ""
                } else {
                    "[REDACTED]"
                },
            )
            .field("api_url", &self.api_url)
            .field("auth_file", &self.auth_file)
            .field("environment_id", &self.environment_id)
            .field("vcs_provider", &self.vcs_provider)
            .field("pull_request_vcs_id", &self.pull_request_vcs_id)
            .field("project_vcs_path", &self.project_vcs_path)
            .field("workflow_run_id", &self.workflow_run_id)
            .field("start_services", &self.start_services)
            .field("last_commit_sha", &self.last_commit_sha)
            .field("start_terminal", &self.start_terminal)
            .field("services", &self.services)
            .finish()
    }
}

#[derive(Clone, Deserialize, Default)]
pub struct ServiceConfig {
    pub name: String,
    pub port: u16,
    /// Hostname where the service is reachable from the agent. Defaults to localhost.
    pub host: Option<String>,
    pub cmd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub args: Option<Vec<String>>,
    pub health_check: Option<HealthCheck>,
}

struct RedactedEnvironment<'a>(&'a HashMap<String, String>);

impl fmt::Debug for RedactedEnvironment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, "[REDACTED]")))
            .finish()
    }
}

impl fmt::Debug for ServiceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceConfig")
            .field("name", &self.name)
            .field("port", &self.port)
            .field("host", &self.host)
            .field("cmd", &self.cmd)
            .field("env", &self.env.as_ref().map(RedactedEnvironment))
            .field("args", &self.args)
            .field("health_check", &self.health_check)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthCheck {
    pub interval: u64,
    pub timeout: u64,
    pub retries: u64,
    pub path: String,
}

#[cfg(test)]
mod test {
    use super::*;
    fn test_args() -> Args {
        Args {
            config_path: Some("kittengrid.test.yml".into()),
            global_config_path: None,
            no_global_config: true,
            no_auth_file: true,
            command: None,
            config: <Config as ClapSerde>::Opt::default(),
        }
    }

    #[test]
    fn debug_output_masks_api_keys_and_service_environment_values() {
        let config = Config {
            api_key: "private-api-key".into(),
            api_url: "https://backend.example".into(),
            services: vec![ServiceConfig {
                name: "web".into(),
                port: 3000,
                cmd: Some("server".into()),
                env: Some(HashMap::from([
                    ("API_TOKEN".into(), "private-service-token".into()),
                    ("DATABASE_URL".into(), "private-database-url".into()),
                ])),
                ..Default::default()
            }],
            ..Default::default()
        };
        for dump in [format!("{config:?}"), format!("{config:#?}")] {
            assert!(!dump.contains("private-api-key"));
            assert!(!dump.contains("private-service-token"));
            assert!(!dump.contains("private-database-url"));
            assert!(dump.contains("[REDACTED]"));
            assert!(dump.contains("https://backend.example"));
            assert!(dump.contains("API_TOKEN"));
            assert!(dump.contains("web"));
            assert!(dump.contains("3000"));
        }
        let service_dump = format!("{:?}", config.services[0]);
        assert!(!service_dump.contains("private-service-token"));
        assert!(!service_dump.contains("private-database-url"));
        assert_eq!(config.api_key, "private-api-key");
        assert_eq!(
            config.services[0].env.as_ref().unwrap()["API_TOKEN"],
            "private-service-token"
        );
    }

    #[test]
    fn validates_api_url_without_exposing_its_contents() {
        for api_url in [
            "",
            "invalid",
            "ftp://backend.example",
            "https://secret@backend.example",
            "https://backend.example/path",
            "https://backend.example?secret",
            "https://backend.example/#secret",
        ] {
            let mut args = test_args();
            args.config.api_url = Some(api_url.into());
            let error = process_args(&mut args).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidApiUrl));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn oauth_requires_https_except_for_localhost() {
        for api_url in [
            "https://backend.example",
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
        ] {
            let config = Config {
                api_url: api_url.into(),
                ..Default::default()
            };
            assert!(config.oauth_backend().is_ok());
        }
        let config = Config {
            api_url: "http://backend.example".into(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
        assert!(matches!(
            config.oauth_backend(),
            Err(ConfigError::InsecureOAuthApiUrl)
        ));
    }

    #[test]
    fn default_values() {
        let mut args = test_args();
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.log_level, "info");
        assert_eq!(config.bind_address, "0.0.0.0");
    }

    #[test]
    fn overwrite_defaults() {
        let mut args = test_args();
        args.config.log_level = Some("debug".to_string());
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.bind_address, "0.0.0.0");
    }

    #[test]
    fn overwrite_file_stuff() {
        let mut args = test_args();
        args.config.project_vcs_path = Some("123445".to_string());
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.project_vcs_path, "123445");
    }

    #[test]
    fn environment_id_from_cli() {
        let mut args = Args::parse_from([
            "kittengrid-agent",
            "--no-global-config",
            "--no-auth-file",
            "--config",
            "kittengrid.test.yml",
            "--environment-id",
            "7ia79uwdfc4j",
        ]);
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.environment_id, "7ia79uwdfc4j");
    }

    #[test]
    fn environment_id_from_yaml() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.yml");
        std::fs::write(
            &path,
            "api_url: https://backend.example\nenvironment_id: 7ia79uwdfc4j\nservices: []\n",
        )
        .unwrap();
        let mut args = Args::parse_from([
            "kittengrid-agent",
            "--no-global-config",
            "--no-auth-file",
            "--config",
            path.to_str().unwrap(),
        ]);
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.environment_id, "7ia79uwdfc4j");
        assert!(config.project_vcs_path.is_empty());
        assert!(config.pull_request_vcs_id.is_empty());
    }

    #[test]
    fn overwrite_test_file_over_defaults() {
        let mut args = test_args();
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.api_url, "http://web:3000");
    }

    #[test]
    fn merges_only_supplied_fields_and_preserves_false_and_zero() {
        let directory = tempfile::tempdir().unwrap();
        let global = directory.path().join("global.yaml");
        let project = directory.path().join("project.yaml");
        std::fs::write(&global, "api_url: https://global.example\nlog_level: warn\nbind_port: 1234\nstart_services: true\n").unwrap();
        std::fs::write(
            &project,
            "api_url: https://project.example\nbind_port: 0\nstart_services: false\nservices: []\n",
        )
        .unwrap();
        let mut args = test_args();
        args.global_config_path = Some(global);
        args.no_global_config = false;
        args.config_path = Some(project);
        args.config.api_url = Some("https://flag.example".into());
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.api_url, "https://flag.example");
        assert_eq!(config.log_level, "warn");
        assert_eq!(config.bind_port, 0);
        assert!(!config.start_services);
        assert_eq!(config.bind_address, "0.0.0.0");
    }

    #[test]
    fn explicit_false_and_zero_flags_override_project_values() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project.yaml");
        std::fs::write(
            &project,
            "api_url: https://backend.example\nstart_services: true\nbind_port: 1234\nservices: []\n",
        )
        .unwrap();
        let mut args = Args::parse_from([
            "kittengrid-agent",
            "--no-global-config",
            "--no-auth-file",
            "--config",
            project.to_str().unwrap(),
            "--start-services",
            "false",
            "--bind-port",
            "0",
        ]);
        let config = process_args(&mut args).unwrap();
        assert!(!config.start_services);
        assert_eq!(config.bind_port, 0);
    }

    #[test]
    fn rejects_services_in_global_settings() {
        let directory = tempfile::tempdir().unwrap();
        let global = directory.path().join("global.yaml");
        std::fs::write(&global, "services: []\n").unwrap();
        let mut args = test_args();
        args.global_config_path = Some(global);
        args.no_global_config = false;
        assert!(matches!(
            process_args(&mut args),
            Err(ConfigError::GlobalServices(_))
        ));
    }

    #[test]
    fn treats_empty_yaml_as_an_empty_layer() {
        let directory = tempfile::tempdir().unwrap();
        let global = directory.path().join("global.yaml");
        std::fs::write(&global, "# No global overrides yet\n").unwrap();
        let mut args = test_args();
        args.global_config_path = Some(global);
        args.no_global_config = false;
        let config = process_args(&mut args).unwrap();
        assert_eq!(config.api_url, "http://web:3000");
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn reports_missing_explicit_global_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = test_args();
        args.global_config_path = Some(directory.path().join("missing.yaml"));
        args.no_global_config = false;
        assert!(matches!(
            process_args(&mut args),
            Err(ConfigError::Open { .. })
        ));
    }
}
