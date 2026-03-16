# Observability Workshop

A hands-on workshop for learning distributed tracing, metrics, logging, and profiling using Rust microservices with intentional bugs.

## Getting Started

**Requirements**: [Docker](https://docs.docker.com/get-docker/) with Docker Compose (included in Docker Desktop)

```bash
docker compose up -d
```

Then open **http://localhost:4000** — that's Grafana with all dashboards pre-loaded.

> The first startup takes a minute or two while services initialize and pull images.

---

## Architecture

Four microservices with intentional performance bugs:

```
Traefik (edge proxy)
    ↓
Service A (Gateway) → Service B (Orders) → Service C (Products)
                   ↓
                   Service D (Analytics)
```

### Intentional Bugs

1. **N+1 Query Pattern** (Service B → C)
   - Service B fetches products individually (50 HTTP calls) instead of batch
   - Visible in: Trace waterfall, request latency

2. **Connection Pool Exhaustion** (Service C)
   - Pool size of only 5 connections
   - Overwhelmed by Service B's N+1 queries
   - Visible in: Error logs, metrics, trace errors

3. **Cache Miss Storm** (Service A)
   - 15-second TTL causes thundering herd on expiration
   - Visible in: Latency spikes, cache hit ratio metrics

4. **Regex Compilation in Hot Path** (Service D)
   - `Regex::new(...)` called inside a loop on every event
   - Visible in: Pyroscope flame graphs (hot `Regex::new` frames)

## Quick Start

### Prerequisites

- Docker & Docker Compose
- (Optional) [oha](https://github.com/hatoo/oha) for load testing: `cargo install oha`
- (Optional) [jq](https://jqlang.github.io/jq/) for JSON parsing

### Workshop Deployment (Pre-built Images)

```bash
# Start the full stack with pre-built images
docker compose up -d

# Wait for services to initialize (~30 seconds)
sleep 30

# Check health of all services (all through Traefik)
curl http://localhost/health
```

### Configuration

To use your own registry, create a `.env` file:

```bash
# .env
IMAGE_REGISTRY=ghcr.io/your-org
# or
IMAGE_REGISTRY=docker.io/your-dockerhub-username

# Optional: specify version
IMAGE_TAG=v1.0.0
```

## Deployment Modes

This project supports two deployment modes:

### Docker Mode (Full Stack)

**Use case**: Workshop attendees, demos, or running the complete system with pre-built images

The main `docker-compose.yaml` in the root directory runs the entire observability stack:
- All 4 microservices (pre-built images)
- Infrastructure (Postgres, Redis, Traefik)
- Observability stack (Prometheus, Tempo, Loki, Pyroscope, Grafana, Alloy)

```bash
docker compose up -d              # Start everything
docker compose ps                 # Check status
docker compose logs -f service-a  # View logs
docker compose down -v            # Clean shutdown
```

### Local Development Mode

**Use case**: Service development, debugging, or working on the Rust code

For local development, use the two-part setup in the `source/` directory:

1. **Start infrastructure** (Postgres + Redis only):
   ```bash
   cd source
   docker compose up -d
   ```

2. **Run services locally** via process-compose:
   ```bash
   process-compose up
   # Or run individual services:
   cargo run --bin service-a-gateway
   ```

This mode:
- Uses local Rust builds (hot reload during development)
- Reads configuration from `source/configuration/*.yaml`
- Skips the observability stack (optional, can add if needed)
- Provides faster iteration for code changes

**Important**: The observability stack is NOT included in local dev mode. If you need traces/metrics during development, use Docker mode or manually start the observability services.

### Available Commands

**Workshop Deployment:**
```bash
# Start/stop
docker compose up -d
docker compose down

# View logs
docker compose logs -f
docker compose logs -f service-a

# Check status
docker compose ps
docker stats --no-stream

# Access databases
docker compose exec postgres psql -U postgres -d products_db
docker compose exec redis redis-cli
```

**Rust Development** (source/justfile):

For contributors working on the service code:

```bash
cd source

# Build all Docker images
just build

# Build and push to registry
just publish
```

### Access Points

- **Application**: http://localhost (via Traefik)
- **Grafana**: http://localhost:4000 (admin/workshop)
- **Prometheus**: http://localhost:9090
- **Tempo**: http://localhost:3200
- **Pyroscope**: http://localhost:4040
- **Alloy UI**: http://localhost:12345

### Service Ports

| Service | Application | Metrics |
|---------|-------------|---------|
| Service A (Gateway) | 9091 (also via http://localhost) | 19091 |
| Service B (Orders) | internal only | 19092 |
| Service C (Products) | internal only | 19093 |
| Service D (Analytics) | internal only | 19094 |

## Triggering Bugs

All requests should go through Traefik (`http://localhost`) via Service A, which is the intended traffic path.

### 1. N+1 Query Bug (Service B)

```bash
# Trigger the bug - Service A proxies to B, which makes 50 sequential HTTP calls to C
curl "http://localhost/api/orders/20000000-0000-0000-0000-000000000001" | jq

# Check traces in Grafana
# - Navigate to Grafana → Explore → Tempo
# - Search for traces with service_name="service-b-orders"
# - Look for waterfall with 50 child spans to service-c-products
```

### 2. Connection Pool Exhaustion (Service C)

```bash
# Trigger with concurrent requests (requires oha)
oha -z 60s -c 50 http://localhost/api/orders/20000000-0000-0000-0000-000000000001

# Check for errors
# - Logs: docker compose logs service-c
# - Look for "pool timeout" or "PoolTimedOut" errors
# - Metrics show error rate spike on service-c-products
# - Traces show timeout errors on GET /api/products/{id} spans
```

### 3. Cache Miss Storm (Service A)

```bash
# Warm up the cache
curl -s http://localhost/api/summary > /dev/null

# Wait for 15s TTL to expire
sleep 16

# Trigger thundering herd with concurrent requests (requires oha)
oha -n 200 -c 50 http://localhost/api/summary

# Check metrics
curl http://localhost:19091/metrics | grep cache_

# Observe in Grafana
# - Latency spike at TTL expiration
# - Cache hit ratio drops to 0%
# - Service B/D show simultaneous request spikes
```

### 4. Regex Compilation in Hot Path (Service D)

```bash
oha -z 60s -c 10 -m POST -H 'Content-Type: application/json' -d '{"events":[{"order_id":"20000000-0000-0000-0000-000000000001","log_line":"x"}]}' http://localhost/api/analytics/events

# View in Pyroscope
# - Navigate to http://localhost:4040
# - Select service "service-d-analytics"
# - Look for hot Regex::new frames in the flame graph
# - CPU usage grows with request rate due to repeated regex compilation
```

## Observability Tools

### Grafana Dashboards

1. **Explore → Tempo**: View distributed traces
   - Search by service name
   - Filter by trace ID
   - See span waterfalls
   - Click "Profiles" button on a trace to jump to Pyroscope flame graph

2. **Explore → Loki**: View logs
   - Filter by service: `{service_name="service-a-gateway"}`
   - Correlate with traces via trace_id (click the trace_id link in a log line)
   - JSON structured logs

3. **Explore → Prometheus**: View metrics
   - Cache hit/miss: `cache_hits_total`, `cache_misses_total`
   - HTTP duration: `http_request_duration_seconds_bucket`
   - Exemplars link histogram buckets directly to Tempo traces

4. **Explore → Pyroscope**: View CPU profiles
   - Select service and profile type (`process_cpu`)
   - View flame graphs
   - Compare before/after a load test

### Debugging Workflow

1. **Start with metrics**: Identify anomalies (spikes, errors)
2. **Use exemplars**: Click a histogram data point to jump to the exact trace
3. **Examine traces**: Understand request flow and timing
4. **Correlate with logs**: Click trace_id in a log line to open the trace
5. **Profile with Pyroscope**: From a trace span, click "Profiles" to see CPU flame graph

## Architecture Details

### Service A (Gateway)

- **Purpose**: API gateway, aggregates data from B and D, proxies order requests
- **Dependencies**: Redis (cache), Service B, Service D
- **Bug**: Cache TTL 15s → thundering herd when all concurrent requests miss simultaneously
- **Key files**: `services/service-a/src/handlers.rs`

### Service B (Orders)

- **Purpose**: Order management
- **Dependencies**: Postgres, Redis, Service C
- **Bug**: N+1 query pattern — fetches 50 products individually instead of in batch
- **Key files**: `services/service-b/src/handlers.rs` (see `get_order`, the N+1 loop)

### Service C (Products)

- **Purpose**: Product catalog
- **Dependencies**: Postgres
- **Bug**: Pool size = 5 (too small for the N+1 load from Service B)
- **Key files**: `services/service-c/src/app.rs` (`max_connections` setting)

### Service D (Analytics)

- **Purpose**: Analytics event processing
- **Dependencies**: Postgres
- **Bug**: `Regex::new(...)` compiled on every loop iteration in the hot path
- **Key files**: `services/service-d/src/handlers.rs` (see `process_events`)

## Fixing the Bugs

### Fix 1: N+1 Query (Service B)

**Problem**: Sequential HTTP calls in a loop for each order item

**Solution**: Add a batch endpoint to Service C and call it once
```rust
// In Service C, add:
POST /api/products/batch
// Body: { "ids": ["uuid1", "uuid2", ...] }

// In Service B, replace the loop with:
let product_ids: Vec<Uuid> = items.iter().map(|i| i.product_id).collect();
let products = http_client
    .post(&format!("{}/api/products/batch", service_c_url))
    .json(&ProductBatchRequest { ids: product_ids })
    .send()
    .await?
    .json::<Vec<Product>>()
    .await?;
```

### Fix 2: Pool Exhaustion (Service C)

**Problem**: Pool size = 5 is exhausted by 50 concurrent N+1 requests

**Solution**: Increase pool size in config or code
```rust
// In services/service-c/src/app.rs:
let db = PgPoolOptions::new()
    .max_connections(20)  // Changed from 5
    .acquire_timeout(Duration::from_secs(settings.database.pool_acquire_timeout_secs))
    .connect(&settings.database.connection_url())
    .await?;
```

### Fix 3: Cache Miss Storm (Service A)

**Problem**: All requests miss cache simultaneously when TTL expires

**Solution**: Request coalescing or staggered expiration
```rust
// Option 1: Randomize TTL to spread expirations
let ttl_secs = settings.cache.ttl_secs + (rand::random::<u64>() % 10);

// Option 2: Background refresh before expiration
// Option 3: Single-flight pattern (only one request populates cache)
```

### Fix 4: Regex Compilation in Hot Path (Service D)

**Problem**: `Regex::new(...)` called inside a loop on every event, causing unnecessary CPU burn

**Solution**: Move `Regex::new(...)` above the loop so it compiles once
```rust
// In services/service-d/src/handlers.rs, inside process_events():
let order_id_pattern = Regex::new(r"order_id: ([a-f0-9-]+)")?;

for event in &req.events {
    // use order_id_pattern here — already compiled
}
```

## Cleaning Up

```bash
# Stop all services
docker compose down

# Remove volumes (clears data)
docker compose down -v
```

## Development

### Project Structure

```
observability_workshop/
├── source/                       # Rust source code
│   ├── Cargo.toml               # Workspace manifest
│   ├── justfile                 # Build commands
│   ├── .cargo/config.toml       # Rustflags (force-frame-pointers for Pyroscope)
│   ├── configuration/
│   │   ├── base.yaml            # Shared settings
│   │   ├── dev.yaml             # Local dev overrides
│   │   └── docker.yaml          # Docker overrides
│   └── services/
│       ├── service-a/           # Gateway (cache storm bug)
│       ├── service-b/           # Orders (N+1 bug)
│       ├── service-c/           # Products (pool exhaustion bug)
│       └── service-d/           # Analytics (regex hot path bug)
├── config/                       # Infrastructure configs
│   ├── alloy/                   # Log/trace collection pipeline
│   ├── grafana/                 # Dashboards and datasources
│   ├── tempo/                   # Distributed tracing config
│   ├── loki/                    # Log aggregation config
│   ├── prometheus/              # Metrics scrape config
│   └── postgres/                # DB init script
├── docker-compose.yaml           # Full stack (pre-built images)
└── README.md
```

### Adding Custom Metrics

Services use the `prometheus-client` crate. Metrics are registered at startup and observed via Tower middleware or inline in handlers.

```rust
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;

// Register at startup
let my_counter: Family<MyLabels, Counter> = Family::default();
registry.register("my_counter", "Description", my_counter.clone());

// Observe in handler
my_counter.get_or_create(&MyLabels { label: "value".into() }).inc();
```

### Adding Custom Spans

```rust
#[tracing::instrument(skip(state), fields(user_id = %user_id))]
async fn my_handler(State(state): State<AppState>, user_id: Uuid) {
    tracing::info!("Processing request");
    // ...
}
```

## Learning Objectives

By completing this workshop, you'll learn to:

1. Use distributed tracing to identify N+1 queries
2. Correlate logs with traces using trace_id
3. Detect resource exhaustion via metrics
4. Identify cache stampede patterns
5. Use profiling to find CPU hot paths
6. Navigate between signals using exemplars, derived fields, and trace-to-profile links

## Troubleshooting

### Services won't start
```bash
# Check logs
docker compose logs service-a
docker compose logs postgres

# Ensure ports aren't in use
lsof -i :80 -i :4000 -i :5432 -i :6379 -i :9090
```

### Can't see traces in Tempo
```bash
# Check Alloy is receiving spans
curl http://localhost:12345

# Check Alloy logs
docker compose logs alloy

# Verify OTLP endpoint
docker compose exec service-a env | grep OTLP
```

### Migration errors on startup
```bash
# If you see "migration was previously applied but has been modified":
docker compose down -v
docker compose up -d
```

---

## Related

- [observability_presentation](https://github.com/jdeinum/observability_presentation) — slides and material for the workshop
- [observability_animations](https://github.com/jdeinum/observability_animations) — animations used in the presentation
