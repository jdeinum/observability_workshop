use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{FromRef, MatchedPath, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use opentelemetry::trace::TraceContextExt;
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
    pub metrics: Arc<Metrics>,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> PgPool {
        state.db.clone()
    }
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
    // INTENTIONAL BUG: Pool size from config (5 in dev mode - way too small!)
    // This causes connection pool exhaustion when Service B's N+1 queries
    // and Service D's lookups hit this service simultaneously
    let db = PgPoolOptions::new()
        .max_connections(settings.database.pool_max_connections)
        .min_connections(settings.database.pool_min_connections)
        .acquire_timeout(Duration::from_secs(settings.database.pool_acquire_timeout_secs))
        .connect(&settings.database.connection_url())
        .await?;

    tracing::warn!(
        max_connections = settings.database.pool_max_connections,
        "Database pool initialized (5 in dev mode - intentional bug!)"
    );

    // Run migrations
    sqlx::migrate!("./migrations").run(&db).await?;

    let state = AppState { db, metrics };

    let app = Router::new()
        .route("/api/products/{id}", get(handlers::get_product))
        // NOTE: Batch endpoint is intentionally MISSING
        // This forces Service B to make N individual HTTP calls (N+1 bug)
        .layer(OtelInResponseLayer::default())
        .layer(middleware::from_fn_with_state(state.clone(), track_http_request))
        .layer(OtelAxumLayer::default())
        .route("/health", get(handlers::health))
        .with_state(state);

    Ok(app)
}
