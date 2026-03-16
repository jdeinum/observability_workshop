FROM lukemathwalker/cargo-chef:latest-rust-bookworm AS chef
WORKDIR /workspace

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p service-b-orders
COPY . .
RUN cargo build --release -p service-b-orders

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /workspace/target/release/service-b-orders /usr/local/bin/service
COPY --from=builder /workspace/configuration /app/configuration
COPY --from=builder /workspace/services/service-b/migrations /app/migrations
WORKDIR /app
ENV RUN_MODE=production
EXPOSE 3000 9090
CMD ["service"]
