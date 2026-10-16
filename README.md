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

The environment must already exist and belong to the API key's organization. The agent sends `environment_id` during registration and on subsequent tunnel, service, and runtime-status requests. When configured, it replaces the VCS/PR identity in API requests, even if legacy VCS settings remain in the configuration. Without it, the existing CI registration path is unchanged.

Development environments do not need a repository, PR number, workflow run ID, or commit SHA. Empty SHA values are omitted when publishing services. For a PR preview registered by environment ID, still supply `KITTENGRID_WORKFLOW_RUN_ID` for CI startup authorization and `KITTENGRID_LAST_COMMIT_SHA` for preview-link publication. Runtime updates use `/api/agents/environment` in environment mode; legacy CI mode retains `/api/agents/pull_request`.

The API's startup instructions are honored as before: development registration requests service startup and no terminal by default. Explicit local startup flags continue to work. This change adds environment identity, not Docker orchestration or expose-only service mode; configured services are still managed processes.

## Registration declined for closed or merged requests

If the registration API returns HTTP `409 Conflict`, the PR/MR is no longer open. The agent logs this as an expected outcome and exits successfully without publishing services, configuring tunnels, or starting services or a terminal. Explicit startup flags do not override this rejection. Other registration failures still exit with code `1`.

# KittenGrid Agent Configuration

The KittenGrid agent uses a YAML configuration file to define services that should be managed. By default, the agent looks for `kittengrid.yml` or `kittengrid.yaml` in the current directory.

Use `--config path/to/config.yml` to select a file explicitly. If the selected file cannot be opened, the agent prints the path and error and exits with code `1` before registration or service startup; it does not fall back to defaults or another file. When `--config` is omitted and neither default file exists, CLI/environment-only configuration remains supported.

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

## File Location

The configuration file can be specified using:
- Command line: `--config /path/to/config.yml`
- Environment variable: `KITTENGRID_CONFIG_PATH`
- Default: `kittengrid.yml` in the current directory

The agent will also accept `kittengrid.yaml` as an alternative file extension.
