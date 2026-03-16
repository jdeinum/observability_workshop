use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{MatchedPath, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use opentelemetry::trace::TraceContextExt;
use reqwest_middleware::ClientBuilder;
use reqwest_tracing::TracingMiddleware;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    handlers,
    metrics::{HttpLabels, Metrics, TraceExemplar},
    settings::Settings,
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: reqwest_middleware::ClientWithMiddleware,
    pub service_c_url: String,
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

pub async fn build(settings: Settings, metrics: Arc<Metrics>) -> anyhow::Result<Router> {
    // Postgres database
    let db = PgPoolOptions::new()
        .max_connections(settings.database.pool_max_connections)
        .min_connections(settings.database.pool_min_connections)
        .acquire_timeout(Duration::from_secs(settings.database.pool_acquire_timeout_secs))
        .connect(&settings.database.connection_url())
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&db).await?;

    let http_client = ClientBuilder::new(reqwest::Client::new())
        .with(TracingMiddleware::default())
        .build();

    let state = AppState {
        db,
        http_client,
        service_c_url: settings.services.service_c_url,
        metrics,
    };

    let app = Router::new()
        .route("/api/analytics/events", post(handlers::process_events))
        .route("/api/analytics/count", get(handlers::get_event_count))
        .layer(OtelInResponseLayer::default())
        .layer(middleware::from_fn_with_state(state.clone(), track_http_request))
        .layer(OtelAxumLayer::default())
        .route("/health", get(handlers::health))
        .with_state(state);

    Ok(app)
}
