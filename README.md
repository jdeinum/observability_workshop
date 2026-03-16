# Observability Workshop

A hands-on workshop for learning distributed tracing, metrics, logging, and profiling using Rust microservices with intentional bugs.

## Requirements

- [Docker](https://docs.docker.com/get-docker/) with Docker Compose (included in Docker Desktop)
- [oha](https://github.com/hatoo/oha) for load testing: `cargo install oha`

## Getting Started

```bash
docker compose up -d
```

Then open **http://localhost:4000** — Grafana with all dashboards pre-loaded.

> The first startup takes a minute or two while images are pulled and services initialize.

## Related

- [observability_presentation](https://github.com/jdeinum/observability_presentation) — slides and material for the workshop
- [observability_animations](https://github.com/jdeinum/observability_animations) — animations used in the presentation
