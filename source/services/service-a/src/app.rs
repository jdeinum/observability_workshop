use std::sync::Arc;

use axum::{
    extract::{MatchedPath, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use fred::prelude::*;
use opentelemetry::trace::TraceContextExt;
use reqwest_middleware::ClientBuilder;
use reqwest_tracing::TracingMiddleware;
use std::time::Instant;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    handlers,
    metrics::{HttpLabels, Metrics, TraceExemplar},
    settings::Settings,
};

#[derive(Clone)]
pub struct AppState {
    pub redis: RedisPool,
    pub http_client: reqwest_middleware::ClientWithMiddleware,
    pub settings: Settings,
    pub metrics: Arc<Metrics>,
}

async fn track_http_request(
    State(state): State<AppState>,
    matched_path: Option<MatchedPath>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = matched_path
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let response = next.run(req).await;

    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let cx = tracing::Span::current().context();
    let span = cx.span();
    let sc = span.span_context();
    let exemplar = sc
        .is_valid()
        .then(|| TraceExemplar { trace_id: sc.trace_id().to_string() });

    state
        .metrics
        .http_duration
        .get_or_create(&HttpLabels { method, path, status })
        .observe(duration, exemplar);

    response
}

pub async fn build(
    settings: Settings,
    metrics: Arc<Metrics>,
) -> anyhow::Result<Router> {
    let config = RedisConfig::from_url(settings.redis_url.as_str())?;
    let redis = RedisPool::new(config, None, None, None, 10)?;
    redis.connect();
    redis.wait_for_connect().await?;

    let http_client = ClientBuilder::new(reqwest::Client::new())
        .with(TracingMiddleware::default())
        .build();

    tracing::warn!(
        cache_ttl_secs = settings.cache.ttl_secs,
        "Gateway initialized with cache TTL from config (15s in dev mode - intentional bug!)"
    );

    let state = AppState {
        redis,
        http_client,
        settings,
        metrics,
    };

    let app = Router::new()
        .route("/api/summary", get(handlers::get_summary))
        .route("/api/orders/{id}", get(handlers::get_order))
        .route("/api/analytics/events", post(handlers::post_analytics_events))
        .layer(OtelInResponseLayer::default())
        .layer(middleware::from_fn_with_state(state.clone(), track_http_request))
        .layer(OtelAxumLayer::default())
        .route("/health", get(handlers::health))
        .with_state(state);

    Ok(app)
}
