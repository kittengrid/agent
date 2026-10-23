# Kittengrid Agent

The kittengrid-agent is a small, reliable, and cross-platform build
agent that makes it easy to make your services available for external traffic.

Its primary purpose is to run services defined in a YAML configuration file,
automatically managing their lifecycle, health checks, and network exposure.

This is usually ran automatically using the kittengrid/action GitHub Action.

## Features

- **Service management**: Automatically starts, stops, and restarts services based on configuration.
- **Health checks**: Monitors service health and restarts services if they become unhealthy.
- **Network exposure**: Automatically exposes services to external traffic.
- **Configuration**: Uses a simple YAML file to define services and their options.
- **Environment variables**: Supports setting environment variables for services.
- **Command-line arguments**: Allows passing command-line arguments to services.
- **Automatic shutdown**: Based on a timeout of inactivity, the agent will automatically shut down services after a period of inactivity.

# Authentication

The agent can authenticate with either an organization API key or a user OAuth token.

- **API key:** Use an API key for CI, shared environments, and other unattended or organization-level workloads. Supply it with `--api-key`, `KITTENGRID_API_KEY`, or configuration as described below. Keep API keys in your CI secret store and avoid committing them.
- **OAuth token:** For a local developer environment, run `kittengrid-agent login` to authorize with your user account. The login command obtains and saves an OAuth session so subsequent local agent runs can authenticate without an API key. Login options and saved-credential behavior are described below.

## Registering an existing environment

Set the environment's **public ID** (not its database ID) using one of:

- CLI: `--environment-id 7ia79uwdfc4j`
- Environment variable: `KITTENGRID_ENVIRONMENT_ID=7ia79uwdfc4j`
- YAML: `environment_id: 7ia79uwdfc4j`

For example:

```bash
export KITTENGRID_API_KEY='<organization-api-key>'
kittengrid-agent --environment-id 7ia79uwdfc4j --config kittengrid.yaml
```

The environment must already exist and belong to the API key's organization. The agent sends `environment_id` during registration and on subsequent tunnel, service, and runtime-status requests. When configured, it replaces the VCS/PR identity in API requests, even if legacy VCS settings remain in the configuration. Without it, CI registration continues to use VCS/PR identity.

Development environments do not need a repository, PR number, workflow run ID, or commit SHA. Empty SHA values are omitted when publishing services. For a PR preview registered by environment ID, still supply `KITTENGRID_WORKFLOW_RUN_ID` for CI startup authorization and `KITTENGRID_LAST_COMMIT_SHA` for preview-link publication. Both identity modes use `/api/v1/agents/environment` for runtime updates. All backend requests use the `/api/v1` API with API-key authentication. The configured API URL remains the server origin (without `/api/v1`). This requires a backend exposing the versioned API; there is no fallback to unversioned endpoints.

The API's startup instructions are honored as before: development registration requests service startup and no terminal by default. Explicit local startup flags continue to work. This change adds environment identity, not Docker orchestration or expose-only service mode; configured services are still managed processes.

## Registration declined for closed or merged requests

If the registration API returns HTTP `409 Conflict`, the PR/MR is no longer open. The agent logs this as an expected outcome and exits successfully without publishing services, configuring tunnels, or starting services or a terminal. Explicit startup flags do not override this rejection. Other registration failures still exit with code `1`.

# KittenGrid Agent Configuration

The agent loads optional global settings and a project YAML file, then applies environment variables and command-line flags.

## Settings Precedence

From highest to lowest priority:

```text
command-line flags > environment variables > project YAML > global YAML > built-in defaults
```

Only supplied values override a lower layer. Explicit `false` and `0` values are preserved; missing fields and YAML `null` inherit from lower layers. Global settings do not change the process's working directory or where the project file is discovered. If the resulting `api_key` is empty, the agent additionally checks the separate saved authentication file described below.

## Global Settings

Default location:

| Platform | Global settings file |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/kittengrid-agent/config.yaml`, or `~/.config/kittengrid-agent/config.yaml` when XDG_CONFIG_HOME is unset, empty, or relative |
| macOS | `~/Library/Application Support/kittengrid-agent/config.yaml` |
| Windows | `%APPDATA%\\kittengrid-agent\\config.yaml` (the platform roaming configuration directory) |

Example `~/.config/kittengrid-agent/config.yaml`:

```yaml
api_url: https://app.kittengrid.com
log_level: info
show_services_output: false
```

Use the same top-level setting names as the project file: API, logging, bind/work-directory, and environment/VCS/CI settings all retain their corresponding flags and `KITTENGRID_*` environment variables. **Service definitions belong in the project file**; a global `services` list is rejected rather than accidentally starting processes in every project.

Select another global file with:

```bash
kittengrid-agent --global-config /path/to/global.yaml --config ./kittengrid.yaml
# Or select it through KITTENGRID_GLOBAL_CONFIG=/path/to/global.yaml
```

`--no-global-config` disables global settings entirely, including a file selected through `--global-config` or `KITTENGRID_GLOBAL_CONFIG`. This is useful for isolated CI jobs. A missing automatically discovered global file is normal and ignored; an explicitly selected missing file, unreadable file, or malformed YAML fails before any API requests or service startup. Empty files are empty settings layers. The agent does not create or rewrite this file. Settings are read once at startup; edits apply to subsequent runs.

An existing `api_key` setting can also be supplied globally, but then applies to all projects unless overridden. Treat any file containing secrets as private (`chmod 600` on Unix), never commit it, and take care when projects target different backends. Startup configuration dumps redact `api_key` and every service `env` value, while keeping other settings and environment-variable names visible. Command/argument strings are logged as provided, so use environment variables rather than command arguments for service secrets. Prefer the separate `authn.json` file below for saved API keys. OAuth login and rotating refresh-token persistence are supported as described below.

## Saved Authentication (`authn.json`)

The agent can read a saved API key from `authn.json` in the same **default platform directory** as global settings (on Linux, `$XDG_CONFIG_HOME/kittengrid-agent/authn.json`, falling back to `~/.config/kittengrid-agent/authn.json`). Selecting a different global YAML file does not relocate it.

Create a private file containing:

```json
{
  "api_key": "<organization-api-key>"
}
```

The file stores one active credential (an API key or OAuth login), with no endpoint selection or binding. Saved credentials apply to whichever `api_url` is configured through flags, environment, or YAML. When switching backends for development, supply the appropriate credentials explicitly or use a separate `--auth-file`.

Credential priority is:

```text
--api-key > KITTENGRID_API_KEY > project YAML api_key > global YAML api_key > authn.json api_key > authn.json OAuth
```

A nonempty configured key wins; in that case the saved file is not opened or validated, even if explicitly selected. An empty configured key allows the saved-key fallback.

Use `--auth-file PATH`, `KITTENGRID_AUTH_FILE`, or the YAML `auth_file: /path/to/authn.json` setting to select another file. The path follows the normal settings hierarchy; relative paths are resolved from the process's working directory. `--no-auth-file` disables saved credentials, including an explicitly selected file. `--no-global-config` only disables global YAML, **not** saved authentication; use both flags for complete global-file isolation in CI.

A missing discovered authentication file is optional. When saved credentials are needed, an explicitly selected missing file, unreadable file, invalid JSON, unsupported/missing fields, or invalid key fails before registration. Empty files are invalid, unlike empty YAML settings layers. Errors report the file and location without echoing JSON values or credentials.

On Unix, the file must have no group/other permissions: run `chmod 600 /path/to/authn.json` (read-only `0400` also works). Keep its parent directory private. On Windows, protect the file with your user account's ACLs; the agent does not validate Windows ACLs. This is plaintext secret storage: never commit it. API-key files are read without rewriting them. OAuth login replaces the saved credential with an OAuth session, and refresh updates that session atomically using a private file. Configuration logging never includes saved OAuth secrets; API keys remain redacted.

## User Login

```bash
kittengrid-agent login                  # Open the default OS browser + localhost callback
kittengrid-agent login --no-browser     # Print the URL; still receive a localhost callback
kittengrid-agent login --manual         # Print the URL; paste the browser's JSON handoff
```

`--manual` never opens a browser or starts a localhost server, and can be combined with `--no-browser`. Use it when the browser runs on a different machine from the terminal. `--no-browser` alone still requires the browser to reach the agent's temporary `127.0.0.1` listener. Every mode prints the authorization URL as a fallback, uses PKCE S256 and a random state, and requires explicit browser consent. Only copy the handoff into the login command you started; never paste access/refresh tokens.

The API URL comes from the usual settings hierarchy. OAuth requires HTTPS, except for HTTP localhost development servers. Root settings can appear before or after `login`, for example:

```bash
kittengrid-agent login --manual --api-url https://app.kittengrid.com --auth-file /private/authn.json
```

Operators must register/update the public client on the backend with `oauth:register_agent`. It registers the dynamic-port loopback callback and the backend's `/oauth/manual` handoff page. The manual page only displays the short-lived code and state; PKCE verification still happens at the token endpoint. Existing backend registrations must be updated before using manual login.

The public OAuth client ID is `kittengrid-agent`, and login waits up to 600 seconds for authorization. Timeout, denial, invalid state, or exchange failure leaves existing credentials untouched. Ctrl-C cancels the authorization wait. `login --no-auth-file` is rejected because there would be nowhere to save credentials.

Successful login writes an `oauth` object containing `client_id`, `access_token`, `refresh_token`, and Unix-seconds `expires_at` to `authn.json`, replacing any saved API key (without revoking that API key on the backend). New parent directories are private on Unix and the credential file is `0600`. Explicit API keys in flags/environment/YAML still win; login warns when one is configured. Remove that override to use the OAuth session.

API requests use the saved access token and refresh it when within 60 seconds of expiry, including during long-running agent sessions. Refresh tokens rotate; a private `.lock` sidecar serializes refreshes across concurrent processes, and the credentials are reread after locking. Updates use an atomic private-file replacement. Failed refreshes are not retried blindly and do not overwrite saved credentials; run `login` again if the grant is no longer valid. OAuth requests and authenticated API requests do not follow redirects. No logout command is included yet.

## Project Settings

By default, the agent looks for `kittengrid.yml` and then `kittengrid.yaml` in the current directory, using only the first file found. Use `--config path/to/config.yml` or `KITTENGRID_CONFIG_PATH` to select it explicitly; the flag takes priority over the environment variable. A selected file that cannot be opened or parsed produces a path-specific error and exits with code `1` before registration or service startup, without falling back to another file. When neither project default exists, global/CLI/environment-only configuration remains supported.

## Configuration File Structure

The configuration file has the following top-level structure:

```yaml
services:
  - name: service-name
    # Service configuration options...
```

## Service Configuration Options

Each service in the `services` array supports the following configuration options:

### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Unique identifier for the service. Used as the default command if `cmd` is not specified. |
| `port` | integer | Port number that the service will listen on. |

### Optional Fields

| Field | Type | Description | Default |
|-------|------|-------------|---------|
| `cmd` | string | Command to execute to start the service. | Uses the `name` field value |
| `host` | string | Hostname where the service is reachable from the agent. When non-local, the agent proxies tunneled traffic on `port` to this host and port. | `localhost` |
| `args` | array of strings | Command-line arguments to pass to the service. | Empty array |
| `env` | object | Environment variables to set for the service (key-value pairs). | Empty object |
| `health_check` | object | Health check configuration for the service. | None |

### Health Check Configuration

When specified, the `health_check` object supports the following options:

| Field | Type | Description |
|-------|------|-------------|
| `interval` | integer | Time in seconds between health checks. |
| `timeout` | integer | Maximum time in seconds to wait for a health check response. |
| `retries` | integer | Number of failed health checks before marking service as unhealthy. |
| `path` | string | HTTP path to check for health status (relative to service port). |

## Example Configuration

```yaml
# Basic HTTP server services
services:
  - name: service-a
    cmd: python3
    port: 10000
    args:
      - -m
      - http.server
      - 10000

  - name: service-b
    cmd: python3
    port: 10001
    args:
      - -m
      - http.server
      - 10001
    env:
      DEBUG: "true"
      LOG_LEVEL: "info"

  # Service with health checking
  - name: api-service
    cmd: node
    port: 3000
    args:
      - server.js
    env:
      NODE_ENV: production
    health_check:
      interval: 30
      timeout: 5
      retries: 3
      path: /health

  # A Docker-in-Docker service whose published port belongs to the `docker`
  # service container instead of the agent container.
  - name: prometheus
    cmd: docker
    port: 19090
    host: docker
    args:
      - compose
      - up
      - --attach
      - prometheus
    health_check:
      interval: 5
      timeout: 5
      retries: 60
      path: /-/ready
```

## Configuration Inheritance

- If `cmd` is not specified, the service `name` is used as the command
- If `args` is not specified, no arguments are passed to the command
- If `env` is not specified, the service inherits the agent's environment
- Services without health checks will not be monitored for health status

## File Selection Summary

| Layer | Flag | Environment variable | Default |
| --- | --- | --- | --- |
| Global settings | `--global-config PATH` | `KITTENGRID_GLOBAL_CONFIG` | Platform config directory + `kittengrid-agent/config.yaml` |
| Project settings/services | `--config PATH` / `-c PATH` | `KITTENGRID_CONFIG_PATH` | `./kittengrid.yml`, then `./kittengrid.yaml` |
| Saved authentication | `--auth-file PATH` | `KITTENGRID_AUTH_FILE` | Platform config directory + `kittengrid-agent/authn.json` |

Use `--no-global-config` to opt out of global YAML and `--no-auth-file` to opt out of saved authentication. Settings-file paths are selected through flags/environment, not fields inside another YAML file. The separate JSON authentication file's path can also be configured with the YAML `auth_file` setting.
