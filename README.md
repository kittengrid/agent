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

# KittenGrid Agent Configuration

The KittenGrid agent uses a YAML configuration file to define services that should be managed. By default, the agent looks for `kittengrid.yml` in the current directory.

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
A standalone agent to execute docker-compose workload remotely
