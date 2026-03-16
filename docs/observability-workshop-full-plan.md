# Observability Workshop: From Zero to Production-Ready
## Rust/Axum Edition

---

## Part 1: Workshop Plan

### Stack Overview

**Application Layer (Rust/Axum)**

| Signal | Crate | Purpose |
|--------|-------|---------|
| Traces | `tracing`, `tracing-opentelemetry`, `opentelemetry-otlp`, `axum-tracing-opentelemetry` | Structured spans, OTel export via OTLP to Alloy |
| Metrics | `axum-prometheus` (uses `metrics` + `metrics-exporter-prometheus`) | RED metrics on `/metrics` endpoint, Prometheus-compatible |
| Logs | `tracing-subscriber` (with `json` feature) | Structured JSON logs to stdout, collected by Alloy |
| Profiling | None (eBPF via Alloy) or `pyroscope`, `pyroscope-pprofrs` | CPU profiling, zero-code via eBPF or SDK push |

**Infrastructure Layer (Docker Compose)**

| Component | Image | Role |
|-----------|-------|------|
| Traefik | `traefik:v3` | Reverse proxy / edge router, root trace spans |
| Grafana | `grafana/grafana:latest` | Dashboards, alerting, exploration |
| Loki | `grafana/loki:latest` | Log storage |
| Tempo | `grafana/tempo:latest` | Trace storage |
| Prometheus | `prom/prometheus:latest` | Metrics scraping and storage |
| Pyroscope | `grafana/pyroscope:latest` | Profile storage |
| Alloy | `grafana/alloy:latest` | Unified collector (logs, traces, profiles) |
| Postgres | `postgres:16-alpine` | Separate databases for Orders (B) and Products (C) |
| Redis | `redis:7-alpine` | Shared caching instance, per-service key prefixes |

Note: Using Prometheus instead of Mimir for workshop simplicity. Mimir adds operational complexity that isn't needed for learning.

### Project Directory Structure

```
observability-workshop/
├── Cargo.toml              (Workspace root)
├── Cargo.lock              (Shared lockfile)
├── docker-compose.yml
├── config/
│   ├── traefik/traefik.yml
│   ├── alloy/config.alloy
│   ├── grafana/
│   │   ├── provisioning/
│   │   │   └── datasources/datasources.yml
│   │   └── dashboards/
│   │       ├── red-overview.json
│   │       └── service-detail.json
│   ├── tempo/tempo-config.yaml
│   ├── loki/loki-config.yaml
│   ├── prometheus/prometheus.yml
│   └── postgres/init.sql
├── services/
│   ├── service-a/          (Gateway / Aggregator)
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
│   │   └── src/main.rs
│   ├── service-b/          (Orders)
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
│   │   └── src/main.rs
│   ├── service-c/          (Products)
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
│   │   └── src/main.rs
│   └── service-d/          (Notifications)
│       ├── Cargo.toml
│       ├── Dockerfile
│       └── src/main.rs
├── slides/                  (Slidev presentation)
│   ├── package.json
│   └── slides.md
├── animations/              (Motion Canvas)
│   ├── package.json
│   └── src/scenes/
└── loadtest/
    └── k6-script.js         (or hey/wrk script for generating traffic)
```

### Demo Application Architecture

Traefik sits at the edge as a reverse proxy, routing traffic to four Axum microservices. Each service owns its data store. This mirrors a production Kubernetes setup where an ingress controller fronts your services.

```
                    ┌──────────────────────────┐
   HTTP Request ──▶ │  Traefik (Edge Router)   │
                    │  Port 80                 │
                    │  Root trace span (OTel)  │
                    │  Metrics on :8080        │
                    └──────────┬───────────────┘
                               │ routes by path prefix
                               ▼
                    ┌───────────────────────┐
                    │  Service A (Gateway)  │
                    │  Axum · Port 3000     │
                    │  Redis (response cache)│
                    └───┬───────────┬───────┘
                        │           │
              ┌─────────┘           └──────────┐
              ▼                                ▼
┌──────────────────────┐       ┌──────────────────────┐
│  Service B (Orders)  │       │  Service D (Notifs)  │
│  Axum · Port 3000    │       │  Axum · Port 3000    │
│  Postgres (orders_db)│       │  SQLite (notifs.db)  │
│  Redis (order cache) │       │  In-memory queue     │
└──────────┬───────────┘       └──────────┬───────────┘
           │ HTTP: fetch product data      │ HTTP: look up product
           ▼                               ▼
┌──────────────────────┐                   │
│  Service C (Products)│◀──────────────────┘
│  Axum · Port 3000    │
│  Postgres (products_db)│
│  Redis (product cache)│
└──────────────────────┘

┌──────────────────────┐    ┌──────────────────────┐
│  Postgres            │    │  Redis (shared)      │
│  Port 5432           │    │  Port 6379           │
│  Databases:          │    │  Key prefixes:       │
│   orders_db (Svc B)  │    │   gateway:* orders:* │
│   products_db (Svc C)│    │   products:*         │
└──────────────────────┘    └──────────────────────┘
```

**Trace flow:** Traefik creates the root span (edge latency, routing, middleware) → propagates `traceparent` header → Service A picks it up as a child span → propagates to B/C/D. In Tempo, you see the full picture from the moment the request hits the proxy to the database query that served it.

**Infrastructure backing services:**

| Service | Database | Redis | Notes |
|---------|----------|-------|-------|
| Traefik | — | — | Edge proxy; emits own metrics and trace spans |
| A (Gateway) | — | `gateway:*` prefix | Aggregates responses from B/D, caches results |
| B (Orders) | Postgres `orders_db` | `orders:*` prefix | Stores orders, caches hot order data |
| C (Products) | Postgres `products_db` | `products:*` prefix | Stores product catalog, caches product lookups |
| D (Notifications) | SQLite `notifs.db` | — | Lightweight async worker, in-memory work queue |

Single Postgres container with separate databases per service. Single Redis with key prefixes. Service D uses SQLite because it's a lightweight async worker — this is a realistic pattern for services that don't need a full RDBMS.

**Baked-in bugs (mapped to discovery method):**

| # | Bug | Service | Discovered via | Module |
|---|-----|---------|---------------|--------|
| 1 | **N+1 query pattern** — fetches an order, then calls Service C individually for each product line item (50+ sequential HTTP calls instead of a batch endpoint) | B → C | Metrics (high latency), then Traces (50 sequential child spans in waterfall) | Modules 1 + 3 |
| 2 | **Connection pool exhaustion** — Postgres pool of 5 connections, overwhelmed when B's N+1 and D's lookups spike concurrently. Real TCP pool exhaustion, not SQLite write locks. | C | Metrics (error rate spike), then Logs (pool timeout errors with trace_id), then Traces (error spans correlating to pool waits) | Modules 1 + 2 + 3 |
| 3 | **Cache miss storm** — Redis TTL expires, all requests suddenly bypass cache and hit Postgres/downstream services simultaneously | A | Metrics (latency spike + cache hit ratio drop), Logs (cache MISS entries), Traces (spans show downstream calls that were previously cached) | Modules 1 + 2 |
| 4 | **Memory leak** — buffers notification payloads in a Vec without draining; grows unbounded under load | D | Profiling only (flame graph shows growing allocation in the buffer code path; metrics/logs/traces won't surface this) | Module 5 |
| 5 | **Slow middleware** (bonus) — visible in Traefik's edge span; the gap between Traefik's root span start and Service A's span start reveals proxy overhead. Not a bug per se, but teaches attendees to read trace waterfalls and identify where time is spent outside application code. | Traefik → A | Traces (gap in waterfall between proxy and app spans) | Module 3 |

### Cargo Workspace Configuration

The project uses a Cargo workspace to share dependencies and build artifacts across all services. This reduces build time and ensures consistent dependency versions.

#### Workspace Root Cargo.toml

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    "services/service-a",
    "services/service-b",
    "services/service-c",
    "services/service-d",
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Your Name <you@example.com>"]

[workspace.dependencies]
# Web framework
axum = { version = "0.8", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Tracing + OpenTelemetry (traces + logs)
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tracing-opentelemetry = "0.28"
opentelemetry = { version = "0.27", features = ["trace"] }
opentelemetry_sdk = { version = "0.27", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.27", features = ["grpc-tonic"] }
axum-tracing-opentelemetry = "0.24"
init-tracing-opentelemetry = { version = "0.24", features = ["otlp"] }

# Metrics (Prometheus)
axum-prometheus = "0.8"

# HTTP client (inter-service calls)
reqwest = { version = "0.12", features = ["json"] }

# Database drivers
sqlx = { version = "0.8" }

# Caching
redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }

# Utilities
uuid = { version = "1", features = ["v4", "serde"] }

[profile.release]
debug = true                 # keep debug symbols for profiling
force-frame-pointers = true  # critical for eBPF profiling
```

#### Individual Service Cargo.toml Files

```toml
# services/service-a/Cargo.toml (Gateway)
[package]
name = "service-a-gateway"
version.workspace = true
edition.workspace = true

[[bin]]
name = "service-a-gateway"
path = "src/main.rs"

[dependencies]
# Web framework
axum.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true

# Serialization
serde.workspace = true
serde_json.workspace = true

# Tracing + OpenTelemetry
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-opentelemetry.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
axum-tracing-opentelemetry.workspace = true
init-tracing-opentelemetry.workspace = true

# Metrics
axum-prometheus.workspace = true

# HTTP client
reqwest.workspace = true

# Caching
redis.workspace = true
```

```toml
# services/service-b/Cargo.toml (Orders)
[package]
name = "service-b-orders"
version.workspace = true
edition.workspace = true

[[bin]]
name = "service-b-orders"
path = "src/main.rs"

[dependencies]
# Web framework
axum.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true

# Serialization
serde.workspace = true
serde_json.workspace = true

# Tracing + OpenTelemetry
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-opentelemetry.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
axum-tracing-opentelemetry.workspace = true
init-tracing-opentelemetry.workspace = true

# Metrics
axum-prometheus.workspace = true

# HTTP client
reqwest.workspace = true

# Database
sqlx = { workspace = true, features = ["runtime-tokio", "postgres", "uuid", "chrono"] }

# Caching
redis.workspace = true

# Utilities
uuid.workspace = true
```

```toml
# services/service-c/Cargo.toml (Products)
[package]
name = "service-c-products"
version.workspace = true
edition.workspace = true

[[bin]]
name = "service-c-products"
path = "src/main.rs"

[dependencies]
# Web framework
axum.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true

# Serialization
serde.workspace = true
serde_json.workspace = true

# Tracing + OpenTelemetry
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-opentelemetry.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
axum-tracing-opentelemetry.workspace = true
init-tracing-opentelemetry.workspace = true

# Metrics
axum-prometheus.workspace = true

# HTTP client
reqwest.workspace = true

# Database
sqlx = { workspace = true, features = ["runtime-tokio", "postgres", "uuid", "chrono"] }

# Caching
redis.workspace = true

# Utilities
uuid.workspace = true
```

```toml
# services/service-d/Cargo.toml (Notifications)
[package]
name = "service-d-notifications"
version.workspace = true
edition.workspace = true

[[bin]]
name = "service-d-notifications"
path = "src/main.rs"

[dependencies]
# Web framework
axum.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true

# Serialization
serde.workspace = true
serde_json.workspace = true

# Tracing + OpenTelemetry
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-opentelemetry.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
axum-tracing-opentelemetry.workspace = true
init-tracing-opentelemetry.workspace = true

# Metrics
axum-prometheus.workspace = true

# HTTP client
reqwest.workspace = true

# Database (SQLite for lightweight worker)
sqlx = { workspace = true, features = ["runtime-tokio", "sqlite", "chrono"] }

# Note: No redis — uses in-memory VecDeque as work queue (this is the leak)
```

**Benefits of the workspace approach:**
- **Shared dependencies:** All services use the same version of `axum`, `tokio`, `opentelemetry`, etc., reducing build time and binary size
- **Single `Cargo.lock`:** Ensures reproducible builds across all services
- **Incremental compilation:** Changes to one service don't require rebuilding dependencies for others
- **Consistent tooling:** `cargo build --workspace` builds all services at once

**Note on crate versions:** The OpenTelemetry Rust ecosystem is evolving fast. Pin your versions and check compatibility between `tracing-opentelemetry`, `opentelemetry`, and `opentelemetry-otlp` — they must be from the same release cycle. Check https://github.com/open-telemetry/opentelemetry-rust for the latest compatible set.

### Dockerfile (workspace-aware build)

Multi-stage build using Debian slim for the runtime image. The Dockerfile works with the Cargo workspace structure, building from the workspace root and selecting specific binaries.

```dockerfile
# services/service-a/Dockerfile
# Build from workspace root context: docker build -f services/service-a/Dockerfile -t service-a .

# ---- Build stage ----
FROM rust:1.82-bookworm AS builder
WORKDIR /workspace

# Copy workspace manifests for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY services/service-a/Cargo.toml ./services/service-a/
COPY services/service-b/Cargo.toml ./services/service-b/
COPY services/service-c/Cargo.toml ./services/service-c/
COPY services/service-d/Cargo.toml ./services/service-d/

# Create dummy source files for all workspace members to cache dependencies
RUN mkdir -p services/service-a/src && echo "fn main() {}" > services/service-a/src/main.rs && \
    mkdir -p services/service-b/src && echo "fn main() {}" > services/service-b/src/main.rs && \
    mkdir -p services/service-c/src && echo "fn main() {}" > services/service-c/src/main.rs && \
    mkdir -p services/service-d/src && echo "fn main() {}" > services/service-d/src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release -p service-a-gateway

# Remove dummy sources
RUN rm -rf services/*/src

# Copy real source code
COPY services/ ./services/

# Build only this service's binary
RUN cargo build --release -p service-a-gateway

# ---- Runtime stage ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /workspace/target/release/service-a-gateway /usr/local/bin/service

EXPOSE 3000 9090
CMD ["service"]
```

**Service-specific Dockerfiles:**

For each service, the Dockerfile is nearly identical — only the package name (`-p` flag) and binary name change:

```dockerfile
# services/service-b/Dockerfile
# ... same as above, but change these lines:
RUN cargo build --release -p service-b-orders
COPY --from=builder /workspace/target/release/service-b-orders /usr/local/bin/service
```

```dockerfile
# services/service-c/Dockerfile
# ... same as above, but change these lines:
RUN cargo build --release -p service-c-products
COPY --from=builder /workspace/target/release/service-c-products /usr/local/bin/service
```

```dockerfile
# services/service-d/Dockerfile
# ... same as above, but change these lines:
RUN cargo build --release -p service-d-notifications
COPY --from=builder /workspace/target/release/service-d-notifications /usr/local/bin/service
```

**Key details:**
- **Workspace-aware caching:** Copies all workspace `Cargo.toml` files first, then builds dependencies for all services. This ensures Docker's layer cache is invalidated only when dependencies change, not when source code changes.
- **Build context:** All Dockerfiles must be built from the workspace root: `docker build -f services/service-a/Dockerfile -t service-a .`
- **Selective build:** Using `cargo build -p <package-name>` builds only the specified service, but leverages workspace-wide dependency resolution.
- **`ca-certificates`:** Required for HTTPS calls between services (and to Alloy's OTLP endpoint if you ever enable TLS).
- **`force-frame-pointers = true`** in the release profile (from workspace Cargo.toml) ensures binaries are profilable by eBPF. Debug symbols (`debug = true`) are included in the build but don't ship in the runtime image since they stay in the builder layer.
- **Image size:** ~80MB runtime image (Debian slim + binary). Alpine would be ~15MB but requires musl cross-compilation and TLS backend changes that aren't worth the friction for a workshop.

**Docker Compose build context update:**

Since we're building from the workspace root, update `docker-compose.yml` build contexts:

```yaml
services:
  service-a:
    build:
      context: .
      dockerfile: services/service-a/Dockerfile
    # ... rest of config

  service-b:
    build:
      context: .
      dockerfile: services/service-b/Dockerfile
    # ... rest of config
```

### Docker Compose

```yaml
version: "3.9"

services:
  # ============================================================
  # EDGE PROXY
  # ============================================================
  traefik:
    image: traefik:v3
    ports:
      - "80:80"       # HTTP entrypoint (all traffic enters here)
      - "8080:8080"   # Traefik dashboard + Prometheus metrics
    volumes:
      - ./config/traefik/traefik.yml:/etc/traefik/traefik.yml:ro
      - /var/run/docker.sock:/var/run/docker.sock:ro
    depends_on:
      - service-a

  # ============================================================
  # BACKING SERVICES
  # ============================================================
  postgres:
    image: postgres:16-alpine
    ports: ["5432:5432"]
    environment:
      POSTGRES_USER: workshop
      POSTGRES_PASSWORD: workshop
    volumes:
      - ./config/postgres/init.sql:/docker-entrypoint-initdb.d/init.sql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U workshop"]
      interval: 5s
      timeout: 3s
      retries: 5

  redis:
    image: redis:7-alpine
    ports: ["6379:6379"]
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 3s
      retries: 5

  # ============================================================
  # DEMO APPLICATION (Cargo workspace services)
  # ============================================================
  service-a:
    build:
      context: .
      dockerfile: services/service-a/Dockerfile
    ports:
      - "9091:9090"   # metrics (app port not exposed — Traefik routes to it)
    environment:
      RUST_LOG: "info,tower_http=debug"
      OTEL_EXPORTER_OTLP_ENDPOINT: "http://alloy:4317"
      OTEL_SERVICE_NAME: "gateway"
      SERVICE_B_URL: "http://service-b:3000"
      SERVICE_D_URL: "http://service-d:3000"
      REDIS_URL: "redis://redis:6379"
      REDIS_PREFIX: "gateway"
      CACHE_TTL_SECS: "30"           # short TTL to demo cache miss storms
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.gateway.rule=PathPrefix(`/`)"
      - "traefik.http.services.gateway.loadbalancer.server.port=3000"
    depends_on:
      redis: { condition: service_healthy }

  service-b:
    build:
      context: .
      dockerfile: services/service-b/Dockerfile
    ports:
      - "9092:9090"   # metrics
    environment:
      RUST_LOG: "info"
      OTEL_EXPORTER_OTLP_ENDPOINT: "http://alloy:4317"
      OTEL_SERVICE_NAME: "orders"
      SERVICE_C_URL: "http://service-c:3000"
      DATABASE_URL: "postgres://workshop:workshop@postgres:5432/orders_db"
      DB_POOL_SIZE: "10"
      REDIS_URL: "redis://redis:6379"
      REDIS_PREFIX: "orders"
    depends_on:
      postgres: { condition: service_healthy }
      redis: { condition: service_healthy }

  service-c:
    build:
      context: .
      dockerfile: services/service-c/Dockerfile
    ports:
      - "9093:9090"   # metrics
    environment:
      RUST_LOG: "info"
      OTEL_EXPORTER_OTLP_ENDPOINT: "http://alloy:4317"
      OTEL_SERVICE_NAME: "products"
      DATABASE_URL: "postgres://workshop:workshop@postgres:5432/products_db"
      DB_POOL_SIZE: "5"              # intentionally small — pool exhaustion bug
      REDIS_URL: "redis://redis:6379"
      REDIS_PREFIX: "products"
    depends_on:
      postgres: { condition: service_healthy }
      redis: { condition: service_healthy }

  service-d:
    build:
      context: .
      dockerfile: services/service-d/Dockerfile
    ports:
      - "9094:9090"   # metrics
    environment:
      RUST_LOG: "info"
      OTEL_EXPORTER_OTLP_ENDPOINT: "http://alloy:4317"
      OTEL_SERVICE_NAME: "notifications"
      SERVICE_C_URL: "http://service-c:3000"
      DATABASE_URL: "sqlite:///data/notifs.db?mode=rwc"
    volumes:
      - service-d-data:/data

  # ============================================================
  # GRAFANA OBSERVABILITY STACK
  # ============================================================
  grafana:
    image: grafana/grafana:latest
    ports: ["4000:3000"]
    environment:
      GF_SECURITY_ADMIN_PASSWORD: "workshop"
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: "Admin"
    volumes:
      - ./config/grafana/provisioning:/etc/grafana/provisioning
      - ./config/grafana/dashboards:/var/lib/grafana/dashboards

  loki:
    image: grafana/loki:latest
    ports: ["3100:3100"]
    command: -config.file=/etc/loki/local-config.yaml
    volumes:
      - ./config/loki/loki-config.yaml:/etc/loki/local-config.yaml

  tempo:
    image: grafana/tempo:latest
    ports: ["3200:3200"]
    command: -config.file=/etc/tempo/tempo-config.yaml
    volumes:
      - ./config/tempo/tempo-config.yaml:/etc/tempo/tempo-config.yaml

  prometheus:
    image: prom/prometheus:latest
    ports: ["9090:9090"]
    volumes:
      - ./config/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml

  pyroscope:
    image: grafana/pyroscope:latest
    ports: ["4040:4040"]

  alloy:
    image: grafana/alloy:latest
    ports:
      - "4317:4317"   # OTLP gRPC (traces from apps)
      - "4318:4318"   # OTLP HTTP
      - "12345:12345" # Alloy UI
    volumes:
      - ./config/alloy/config.alloy:/etc/alloy/config.alloy
      - /var/run/docker.sock:/var/run/docker.sock  # for container log discovery
      - /proc:/host/proc:ro                         # for eBPF profiling
      - /sys:/host/sys:ro
    privileged: true  # required for eBPF
    pid: host         # required for eBPF to see host processes
    command: run --config.file=/etc/alloy/config.alloy

volumes:
  service-d-data:
```

### Traefik Configuration (config/traefik/traefik.yml)

```yaml
# API / Dashboard
api:
  dashboard: true
  insecure: true  # dashboard on :8080 without auth (workshop only)

# Entrypoints
entryPoints:
  web:
    address: ":80"

# Provider: Docker labels for service discovery
providers:
  docker:
    exposedByDefault: false
    network: default

# Metrics: Prometheus endpoint on :8080/metrics
metrics:
  prometheus:
    entryPoint: traefik
    addEntryPointsLabels: true
    addRoutersLabels: true
    addServicesLabels: true

# Tracing: OpenTelemetry export to Alloy
tracing:
  otlp:
    grpc:
      endpoint: "alloy:4317"
      insecure: true

# Access log (Alloy will collect this from container stdout)
accessLog:
  format: json
  fields:
    headers:
      names:
        traceparent: keep
```

### Database Seeding

**Postgres (Services B and C)** — seeded via init.sql that runs on first container start:

```sql
-- config/postgres/init.sql

-- Create separate databases for each service
CREATE DATABASE orders_db;
CREATE DATABASE products_db;

-- Seed products_db
\c products_db;
CREATE TABLE products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    price NUMERIC(10,2) NOT NULL,
    category TEXT NOT NULL
);
-- Seed 50 products so the N+1 bug produces a visible trace waterfall
INSERT INTO products (name, price, category)
SELECT
    'Product ' || i,
    (random() * 100 + 10)::numeric(10,2),
    CASE WHEN i % 3 = 0 THEN 'widgets'
         WHEN i % 3 = 1 THEN 'gadgets'
         ELSE 'accessories' END
FROM generate_series(1, 50) AS i;

-- Seed orders_db
\c orders_db;
CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_name TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);
CREATE TABLE order_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id UUID REFERENCES orders(id),
    product_id UUID NOT NULL,  -- references products service, fetched via HTTP
    quantity INT NOT NULL DEFAULT 1
);
-- Seed a demo order with 50 line items (triggers N+1 in Service B)
DO $$
DECLARE
    demo_order_id UUID := 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11';
BEGIN
    INSERT INTO orders (id, customer_name) VALUES (demo_order_id, 'Demo Customer');
    FOR i IN 1..50 LOOP
        INSERT INTO order_items (order_id, product_id, quantity)
        VALUES (demo_order_id, gen_random_uuid(), (random() * 5 + 1)::int);
    END LOOP;
END $$;
```

**SQLite (Service D only)** — creates its own database on startup in Rust:

```rust
async fn init_db(pool: &SqlitePool) {
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS notifications (
            id TEXT PRIMARY KEY,
            order_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            payload TEXT,
            created_at TEXT DEFAULT (datetime('now'))
        )
    "#).execute(pool).await.unwrap();
}
```

Service D is intentionally lightweight — a SQLite-backed async worker that buffers notification payloads in memory before writing them. The in-memory VecDeque that never drains is the memory leak.

### Alloy Configuration (config/alloy/config.alloy)

```hcl
// ============================================================
// TRACES: Receive OTLP from Axum apps → forward to Tempo
// ============================================================
otelcol.receiver.otlp "default" {
  grpc { endpoint = "0.0.0.0:4317" }
  http { endpoint = "0.0.0.0:4318" }
  output {
    traces = [otelcol.processor.batch.default.input]
  }
}

otelcol.processor.batch "default" {
  output {
    traces = [otelcol.exporter.otlp.tempo.input]
  }
}

otelcol.exporter.otlp "tempo" {
  client {
    endpoint = "tempo:4317"
    tls { insecure = true }
  }
}

// ============================================================
// LOGS: Collect container stdout → forward to Loki
// ============================================================
discovery.docker "containers" {
  host = "unix:///var/run/docker.sock"
}

discovery.relabel "service_logs" {
  targets = discovery.docker.containers.targets
  rule {
    source_labels = ["__meta_docker_container_name"]
    regex         = "/(service-.+)"
    target_label  = "service"
  }
  rule {
    source_labels = ["__meta_docker_container_name"]
    regex         = "/.+"
    action        = "keep"
  }
}

loki.source.docker "default" {
  host       = "unix:///var/run/docker.sock"
  targets    = discovery.relabel.service_logs.output
  forward_to = [loki.process.json_parse.receiver]
}

loki.process "json_parse" {
  stage.json {
    expressions = {
      level    = "level",
      trace_id = "trace_id",
      span_id  = "span_id",
    }
  }
  stage.labels {
    values = { level = "" }
  }
  forward_to = [loki.write.default.receiver]
}

loki.write "default" {
  endpoint {
    url = "http://loki:3100/loki/api/v1/push"
  }
}

// ============================================================
// PROFILES: eBPF CPU profiling → forward to Pyroscope
// ============================================================
discovery.process "all" {
  refresh_interval = "10s"
  discover_config {
    cgroup_path = "/"
  }
}

discovery.relabel "axum_services" {
  targets = discovery.process.all.targets
  rule {
    source_labels = ["__meta_process_exe"]
    regex         = ".*(service-.+)"
    target_label  = "service_name"
  }
  rule {
    source_labels = ["service_name"]
    regex         = ".+"
    action        = "keep"
  }
}

pyroscope.ebpf "default" {
  targets    = discovery.relabel.axum_services.output
  forward_to = [pyroscope.write.default.receiver]
}

pyroscope.write "default" {
  endpoint {
    url = "http://pyroscope:4040"
  }
}
```

### Prometheus Configuration (config/prometheus/prometheus.yml)

```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: "traefik"
    static_configs:
      - targets: ["traefik:8080"]
  - job_name: "gateway"
    static_configs:
      - targets: ["service-a:9090"]
  - job_name: "orders"
    static_configs:
      - targets: ["service-b:9090"]
  - job_name: "products"
    static_configs:
      - targets: ["service-c:9090"]
  - job_name: "notifications"
    static_configs:
      - targets: ["service-d:9090"]
```

### Workshop Timing (Full Day)

| Time | Block | Content |
|------|-------|---------|
| 9:00 | Opening (15 min) | The data: MTTR trends, cost stats, knowledge gap |
| 9:15 | Module 1 (75 min) | Metrics: PromQL, RED method, axum-prometheus, build dashboard |
| 10:30 | Break (15 min) | |
| 10:45 | Module 2 (75 min) | Structured logging: tracing crate, JSON output, LogQL in Loki |
| 12:00 | Lunch (60 min) | |
| 1:00 | Module 3 (75 min) | Distributed tracing: tracing-opentelemetry, Tempo, trace propagation |
| 2:15 | Break (15 min) | |
| 2:30 | Module 4 (50 min) | Alerting and SLOs: Grafana alerting, error budgets, burn rates |
| 3:20 | Module 5 (50 min) | Continuous profiling: eBPF via Alloy, Pyroscope flame graphs |
| 4:10 | Break (10 min) | |
| 4:20 | Module 6 (45 min) | Incident simulation: trigger cascading failure, debug with full stack |
| 5:05 | Closing (15 min) | Monday morning checklist, maturity model, resources |

---

## Part 2: Sources and Statistics

### MTTR and Incident Cost

| Stat | Source | Year | Citation |
|------|--------|------|----------|
| MTTR >1hr for 82% of orgs, up from 47% in 2021 | Logz.io Observability Pulse Survey | 2024 | https://logz.io/observability-pulse-2024/ |
| Average enterprise: 9 brownouts/month, ~12hr each, costing $13.7M/year | SolarWinds Survey (via DevOps.com) | 2023 | https://devops.com/survey-surfaces-major-observability-challenges/ |
| Only 10% practice full observability; 60% who increase focus report improved troubleshooting | Logz.io Observability Pulse Survey | 2024 | https://logz.io/observability-pulse-2024/ |
| Highest-performing DORA teams have MTTR under 1 day | DORA / Accelerate State of DevOps | 2024 | https://dora.dev/research/ |
| 70% of consumers say page speed impacts willingness to buy | DevPro Journal (citing research) | 2025 | https://www.devprojournal.com/technology-trends/observability/modern-apps-broke-observability-heres-how-we-fix-it/ |

### Knowledge Gap and Adoption

| Stat | Source | Year | Citation |
|------|--------|------|----------|
| #1 challenge: lack of knowledge among team (48%, up from 30%) | Logz.io Observability Pulse | 2024 | https://logz.io/observability-pulse-2024/ |
| 76% say OpenTelemetry adoption is at least "somewhat important" | Logz.io Observability Pulse | 2024 | https://logz.io/observability-pulse-2024/ |
| Top obstacles: pace of tech change (72%), blind spots (58%), app complexity (58%) | SolarWinds Survey | 2023 | https://devops.com/survey-surfaces-major-observability-challenges/ |
| 33% of orgs say C-suite considers observability business-critical | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |
| Alert fatigue is top operational pain point | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |
| 50% of orgs building POC for OpenTelemetry | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |

### Cost and Complexity

| Stat | Source | Year | Citation |
|------|--------|------|----------|
| Cost is the #1 concern about observability (>50%) | Grafana Observability Survey | 2024 | https://grafana.com/observability-survey/2024/ |
| Observability spend averages 17% of total compute infrastructure cost | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |
| 76% use open source for observability in some capacity | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |
| Orgs with 5000+ employees average 24 data sources | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |
| 61% of developers cite ease of use as top buying criteria | Grafana Observability Survey | 2025 | https://grafana.com/observability-survey/2025/ |

### DORA Metrics and Performance

| Stat | Source | Year | Citation |
|------|--------|------|----------|
| Teams with high-quality documentation 2x more likely to meet reliability targets | DORA Accelerate State of DevOps | 2024 | https://dora.dev/research/ |
| 25% increase in AI adoption = 7.5% documentation quality boost but 1.5% delivery performance drop | DORA Report | 2024 | https://dora.dev/research/ |
| Improving MTTR requires improving observability as the primary lever | DORA / Code Climate | 2024 | https://codeclimate.com/blog/dora-metrics |
| 4 DORA metrics: deployment frequency, lead time, change failure rate, failed deployment recovery time | DORA | 2024 | https://dora.dev/guides/dora-metrics-four-keys/ |

### Key Books and Reports (for deeper reference)

| Resource | Authors | Notes |
|----------|---------|-------|
| Accelerate: The Science of Lean Software and DevOps | Forsgren, Humble, Kim | The foundational DORA research book |
| Site Reliability Engineering (Google SRE Book) | Beyer, Jones, Petoff, Murphy | Free online; chapters on monitoring, alerting, SLOs |
| Observability Engineering | Majors, Fong-Jones, Miranda | O'Reilly; the modern "three pillars" reference |
| DORA Accelerate State of DevOps Report 2024 | Google Cloud DORA team | Annual report with latest benchmarks |
| Grafana Observability Survey 2025 | Grafana Labs | 1,255 respondents; largest community observability survey |

---

## Part 3: Presentation Tooling

### Slidev (slides)

**What it is:** Markdown-powered slides built on Vue.js and Vite. You write slides in Markdown with code blocks, and get syntax highlighting, animations, presenter mode, and export to PDF.

**Why it works for this:**
- First-class code syntax highlighting (Shiki) — Rust code will look great
- Built-in Mermaid diagram support for architecture diagrams
- LaTeX support if you want to show formulas (error budget math)
- Dark mode by default (good for tech talks)
- Presenter mode with notes and timer
- Export to PDF for handouts
- Vue components can be embedded for interactive elements

**Setup:**
```bash
npm init slidev@latest observability-workshop
cd observability-workshop
npm run dev
```

**Slide structure suggestion:**

```
slides.md
├── 01-title.md
├── 02-the-problem (MTTR stats, knowledge gap)
├── 03-mental-model (three pillars + profiling)
├── 04-metrics (RED, USE, PromQL, axum-prometheus)
├── 05-logging (structured vs unstructured, tracing crate, LogQL)
├── 06-tracing (spans, propagation, Tempo)
├── 07-alerting (SLOs, error budgets, burn rates)
├── 08-profiling (eBPF, flame graphs, Pyroscope)
├── 09-putting-it-together (incident simulation walkthrough)
└── 10-next-steps (Monday morning checklist)
```

**Embedding code with highlighting:**
```markdown
# axum-prometheus Integration

​```rust {3,7|10-12} 
// Lines 3,7 highlight first, then 10-12 on click
let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

let app = Router::new()
    .route("/api/users", get(get_users))
    .route("/metrics", get(|| async move { metric_handle.render() }))
    .layer(prometheus_layer);
​```
```

**Embedding Mermaid diagrams:**
```markdown
​```mermaid
graph LR
    A[Service A] -->|HTTP| B[Service B]
    B -->|HTTP x50| C[Service C]
    C -->|SQL| D[(Database)]
​```
```

### Motion Canvas (animations)

**What it is:** A TypeScript-based library for creating programmatic animations, rendered as video. Think "After Effects but in code." Each animation is a generator function with tweening and timeline control.

**Why it works for this:**
- Code-driven (version-controllable, reproducible)
- Perfect for animating data flow (packets moving through a pipeline, spans appearing in a trace waterfall)
- Can render to MP4 or play in the browser
- Good community and documentation

**Setup:**
```bash
npm init @motion-canvas@latest observability-animations
cd observability-animations
npm run serve
```

**Suggested animations (prioritized by impact):**

1. **The Request Flow** (highest priority)
   - Animate an HTTP request hitting Traefik (root span created)
   - Traefik injects `traceparent` header, routes to Service A
   - Service A fans out to B and D
   - Service B calls Service C 50 times (N+1 visual)
   - Show spans appearing in a trace waterfall as the request moves
   - Show the gap between Traefik's span and Service A's span (proxy overhead)

2. **The Three Pillars + Profiling**
   - Start with a "black box" service
   - Add metrics (gauges/counters appearing on the outside)
   - Add logs (text streaming out)
   - Add traces (connecting lines between services)
   - Add profiles (flame graph appearing inside the box, revealing the code)

3. **eBPF Profiling**
   - Show user space (your Axum binary) and kernel space
   - Animate the eBPF program attaching to a hook point
   - Show stack traces being sampled at 97Hz
   - Show them aggregating into a flame graph

4. **The Debugging Flow**
   - Alert fires (metric spike)
   - Dashboard shows which service
   - Drill into logs, find trace_id
   - Open trace, see the waterfall
   - Click span, see the flame graph
   - Root cause identified

**Example Motion Canvas scene (simplified):**

```typescript
import { makeScene2D, Txt, Rect, Line } from "@motion-canvas/2d";
import { createRef, waitFor, all } from "@motion-canvas/core";

export default makeScene2D(function* (view) {
  const serviceA = createRef<Rect>();
  const serviceB = createRef<Rect>();
  const arrow = createRef<Line>();

  view.add(
    <>
      <Rect ref={serviceA} x={-300} width={200} height={80}
            fill="#1a1a2e" stroke="#0ea5e9" lineWidth={2} radius={8}>
        <Txt fill="white" fontSize={20}>Service A</Txt>
      </Rect>
      <Rect ref={serviceB} x={300} width={200} height={80}
            fill="#1a1a2e" stroke="#0ea5e9" lineWidth={2} radius={8}>
        <Txt fill="white" fontSize={20}>Service B</Txt>
      </Rect>
    </>
  );

  // Animate request flowing between services
  yield* serviceA().scale(1.1, 0.3);
  yield* serviceA().scale(1, 0.2);
  // ... animate arrow, span creation, etc.
});
```

### Integration Strategy

**Workflow:**
1. Build slides in Slidev (the main presentation)
2. Build 3-4 key animations in Motion Canvas, render as MP4 or GIF
3. Embed the videos/GIFs in Slidev slides using standard HTML/Markdown:

```markdown
---
layout: center
---

# How a Request Flows Through the Stack

<video src="/animations/request-flow.mp4" autoplay loop muted />
```

Alternatively, if presenting from a browser, you can iframe the Motion Canvas player directly into Slidev for live-rendered animations (fancier, but more fragile for live talks).

**Time estimate for the presentation assets:**
- Slidev slides: 1-2 days (mostly content you already have)
- Motion Canvas animations: 3-5 days for 3-4 polished animations (this is the time sink)
- Docker Compose + demo app: 2-3 days to build and test the intentional bugs
- Testing the full workshop flow end-to-end: 1 day

---

## Part 4: Workshop Pre-Requisites Handout

### For Attendees

**Required software:**
- Docker Desktop (or Podman) with Docker Compose
- A terminal (any OS)
- A code editor (VS Code recommended for Rust syntax highlighting)
- A web browser

**Optional but recommended:**
- Rust toolchain (rustup) if you want to modify the demo app
- `curl` or `httpie` for making test requests

**Pre-workshop setup:**
```bash
git clone https://github.com/<your-org>/observability-workshop
cd observability-workshop
docker compose up -d
# Wait ~90 seconds for Postgres + all services to start
#
# Application:      http://localhost        (via Traefik)
# Traefik Dashboard: http://localhost:8080
# Grafana:          http://localhost:4000   (user: admin, pass: workshop)
# Alloy UI:         http://localhost:12345
# Prometheus:       http://localhost:9090
```

**No prior observability experience required.** Familiarity with HTTP APIs and basic terminal usage is sufficient.
