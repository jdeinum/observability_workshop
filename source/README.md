# Observability Workshop - Source Code

This directory contains the Rust source code for all four microservices.

## Quick Start

### Prerequisites

- Rust 1.82+
- Docker & Docker Compose
- [just](https://github.com/casey/just): `cargo install just`
- [process-compose](https://github.com/F1bonacc1/process-compose#installation)
- [sqlx-cli](https://github.com/launchbadge/sqlx/tree/main/sqlx-cli): `cargo install sqlx-cli`

### One-Time Setup

```bash
# 1. Start infrastructure and setup databases
just infra-up
just setup-dbs
just migrate-all

# 2. Stop infrastructure (process-compose will manage it)
just infra-down
```

### Daily Development

```bash
# Start EVERYTHING with one command (infrastructure + all 4 services)
just dev-up

# This starts via process-compose:
# - Infrastructure process: Postgres + Redis (via docker compose)
# - Service C (cargo run)
# - Service B (cargo run)
# - Service D (cargo run)
# - Service A (cargo run)

# Or start in background
just dev-up-bg

# Check service health
just dev-status

# View logs
just dev-logs

# Stop everything (services + infrastructure)
just dev-down
```

### Manual Service Control

```bash
# Run individual services
cargo run -p service-c-products
cargo run -p service-b-orders
cargo run -p service-d-notifications
cargo run -p service-a-gateway
```

## Architecture

Four services with intentional bugs:

```
Service A (Gateway) → Service B (Orders) → Service C (Products)
                   ↓                      ↑
                   Service D (Notifications)
```

### Service Ports

| Service | App Port | Metrics Port |
|---------|----------|--------------|
| Service A (Gateway) | 9091 | 19091 |
| Service B (Orders) | 9092 | 19092 |
| Service C (Products) | 9093 | 19093 |
| Service D (Notifications) | 9094 | 19094 |

## Development Commands

### Build & Test

```bash
just build              # Build all services
just test               # Run tests
just check              # Check compilation
just fmt                # Format code
just lint               # Run clippy
just pre-commit         # Run all checks
```

### Infrastructure

```bash
just infra-up           # Start Postgres + Redis
just infra-down         # Stop infrastructure
just infra-logs         # View infrastructure logs
```

### Database Management

```bash
just setup-dbs          # Create databases
just migrate-all        # Run all migrations
just reset-dbs          # Drop and recreate everything

# Connect to databases
just psql-products      # psql to products_db
just psql-orders        # psql to orders_db
just redis-cli          # redis-cli
```

### Service Management

```bash
just dev-up             # Start all services
just dev-down           # Stop all services
just dev-status         # Check health
just dev-logs           # View logs
just clean-dev          # Clean logs and data
just clean-all          # Full cleanup
```

## Project Structure

```
source/
├── Cargo.toml                # Workspace manifest
├── justfile                  # Development commands
├── docker-compose.yaml       # Infrastructure (Postgres + Redis)
├── process-compose.yaml      # Service orchestration
├── configuration/
│   ├── base.yaml            # Shared settings
│   └── local.yaml           # Local overrides
└── services/
    ├── service-a/           # Gateway (cache storm bug)
    ├── service-b/           # Orders (N+1 bug)
    ├── service-c/           # Products (pool exhaustion bug)
    └── service-d/           # Notifications (memory leak bug)
```

## Intentional Bugs

### 1. Service C - Connection Pool Exhaustion
- **Location**: `services/service-c/src/app.rs:29`
- **Bug**: Pool size = 5 (should be 20+)
- **Fix**: Increase `max_connections` to 20

### 2. Service B - N+1 Query Pattern
- **Location**: `services/service-b/src/handlers.rs:133`
- **Bug**: Fetches 50 products individually
- **Fix**: Implement batch endpoint

### 3. Service A - Cache Miss Storm
- **Location**: `services/service-a/src/handlers.rs`
- **Bug**: 30s TTL causes thundering herd
- **Fix**: Implement request coalescing

### 4. Service D - Memory Leak
- **Location**: `services/service-d/src/worker.rs:36`
- **Bug**: Buffer never drains (`should_process = false`)
- **Fix**: Change to `should_process = true`

## Testing Locally

```bash
# Start everything
just dev-setup
just dev-up

# Test endpoints
curl http://localhost:9091/health
curl http://localhost:9092/api/orders
curl http://localhost:9093/api/products
curl http://localhost:9094/api/notifications/count

# Trigger N+1 bug
ORDER_ID=$(curl -s http://localhost:9092/api/orders | jq -r '.orders[0].id')
curl "http://localhost:9092/api/orders/$ORDER_ID"
```

## Troubleshooting

### Services won't start

```bash
# Check infrastructure is running
docker compose ps

# Check database connections
just psql-products
just psql-orders

# Check logs
just dev-logs
```

### Migrations fail

```bash
# Reset everything
just reset-dbs

# Or manually
just infra-down
just infra-up
just setup-dbs
just migrate-all
```

### Port conflicts

```bash
# Check what's using ports
lsof -i :9091
lsof -i :5432
lsof -i :6379

# Kill conflicting processes or change ports in process-compose.yaml
```

## Contributing

1. Make your changes
2. Run pre-commit checks: `just pre-commit`
3. Test locally: `just dev-up && just dev-status`
4. Submit PR

## Links

- [Main Project README](../README.md)
- [Contributing Guide](../CONTRIBUTING.md)
- [Process Compose Docs](https://f1bonacc1.github.io/process-compose/)
