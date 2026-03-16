use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use fred::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderSummary {
    pub id: Uuid,
    pub customer_name: String,
    pub item_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderListResponse {
    pub orders: Vec<OrderSummary>,
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsCountResponse {
    pub count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub orders: OrderListResponse,
    pub analytics_event_count: i64,
    pub cached: bool,
}

// Error handling — thiserror provides Display, manual From impls provide logging.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Redis error: {0}")]
    Redis(fred::error::RedisError),
    #[error("Upstream service error: {0}")]
    Http(reqwest_middleware::Error),
    #[error("Serialization error: {0}")]
    Serialization(serde_json::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Redis(_) | AppError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Http(_) => StatusCode::BAD_GATEWAY,
        };

        (status, self.to_string()).into_response()
    }
}

// Error logging happens here in the From impls because the ? operator calls
// From::from() inside the handler's #[tracing::instrument] span, where the
// OpenTelemetry trace context is still active. This ensures error log events
// in Loki carry trace_id and span_id as structured metadata.

impl From<fred::error::RedisError> for AppError {
    fn from(e: fred::error::RedisError) -> Self {
        tracing::error!(error = ?e, "Redis error: {e:?}");
        AppError::Redis(e)
    }
}

impl From<reqwest_middleware::Error> for AppError {
    fn from(e: reqwest_middleware::Error) -> Self {
        tracing::error!(error = ?e, "HTTP client error: {e:?}");
        AppError::Http(e)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        tracing::error!(error = ?e, "HTTP client error: {e:?}");
        AppError::Http(reqwest_middleware::Error::Reqwest(e))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        tracing::error!(error = ?e, "Serialization error: {e:?}");
        AppError::Serialization(e)
    }
}

#[tracing::instrument(skip(state), fields(order_id = %order_id))]
pub async fn get_order(
    State(state): State<AppState>,
    Path(order_id): Path<Uuid>,
) -> Result<Response, AppError> {
    let url = format!(
        "{}/api/orders/{}",
        state.settings.services.service_b_url, order_id
    );

    let resp = state
        .http_client
        .get(&url)
        .send()
        .await?
        .error_for_status()?;

    let body = resp.bytes().await?;

    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response())
}

#[tracing::instrument(skip(state, body))]
pub async fn post_analytics_events(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let url = format!(
        "{}/api/analytics/events",
        state.settings.services.service_d_url,
    );

    let resp = state
        .http_client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?
        .error_for_status()?;

    let response_body = resp.bytes().await?;

    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        response_body,
    )
        .into_response())
}

#[tracing::instrument]
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "service-a-gateway"
    }))
}

/// Get summary data aggregated from Services B and D
/// INTENTIONAL BUG: Cache Miss Storm
#[tracing::instrument(skip(state))]
pub async fn get_summary(State(state): State<AppState>) -> Result<Json<SummaryResponse>, AppError> {
    let cache_key = "summary:latest";

    // Try cache first
    let cached: Option<String> = state.redis.get(cache_key).await?;

    if let Some(cached_data) = cached {
        tracing::info!("Cache HIT for summary");

        // Record cache hit metric
        state
            .metrics
            .cache_hits
            .get_or_create(&crate::metrics::CacheLabels {
                endpoint: "summary".to_string(),
            })
            .inc();

        let mut response: SummaryResponse = serde_json::from_str(&cached_data)?;
        response.cached = true;

        return Ok(Json(response));
    }

    tracing::warn!("Cache MISS for summary - fetching from downstream services");

    // Record cache miss metric
    state
        .metrics
        .cache_misses
        .get_or_create(&crate::metrics::CacheLabels {
            endpoint: "summary".to_string(),
        })
        .inc();

    // When cache expires, ALL concurrent requests hit this code path simultaneously
    // causing a stampede to Services B and D

    // Fetch from Service B (orders)
    let orders_url = format!("{}/api/orders", state.settings.services.service_b_url);

    let orders = state
        .http_client
        .get(&orders_url)
        .send()
        .await?
        .error_for_status()?
        .json::<OrderListResponse>()
        .await?;

    // Fetch from Service D (analytics event count)
    let analytics_url = format!(
        "{}/api/analytics/count",
        state.settings.services.service_d_url
    );

    let analytics = state
        .http_client
        .get(&analytics_url)
        .send()
        .await?
        .error_for_status()?
        .json::<AnalyticsCountResponse>()
        .await?;

    let summary = SummaryResponse {
        orders,
        analytics_event_count: analytics.count,
        cached: false,
    };

    // Cache with TTL from config (30s in dev - intentionally short!)
    let cached_data = serde_json::to_string(&summary)?;
    let _: Result<(), _> = state
        .redis
        .set(
            cache_key,
            cached_data.as_str(),
            Some(Expiration::EX(state.settings.cache.ttl_secs as i64)),
            None,
            false,
        )
        .await;

    tracing::info!(
        ttl_secs = state.settings.cache.ttl_secs,
        "Summary cached (TTL from config)"
    );

    Ok(Json(summary))
}
