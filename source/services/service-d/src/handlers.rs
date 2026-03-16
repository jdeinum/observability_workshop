use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsEvent {
    pub order_id: Uuid,
    pub log_line: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessEventsRequest {
    pub events: Vec<AnalyticsEvent>,
}

#[derive(Debug, Serialize)]
pub struct ProcessEventsResponse {
    pub processed: usize,
    pub extracted_order_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EventCountResponse {
    pub count: i64,
}

// Error handling — thiserror provides Display, manual From impls provide logging.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(sqlx::Error),
    #[error("Upstream service error: {0}")]
    Http(reqwest_middleware::Error),
    #[error("Serialization error: {0}")]
    Serialization(serde_json::Error),
    #[error("Regex error: {0}")]
    Regex(regex::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Database(_) | AppError::Serialization(_) | AppError::Regex(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::Http(_) => StatusCode::BAD_GATEWAY,
        };

        (status, self.to_string()).into_response()
    }
}

// Error logging happens here in the From impls because the ? operator calls
// From::from() inside the handler's #[tracing::instrument] span, where the
// OpenTelemetry trace context is still active. This ensures error log events
// in Loki carry trace_id and span_id as structured metadata.

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = ?e, "Database error: {e:?}");
        AppError::Database(e)
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

impl From<regex::Error> for AppError {
    fn from(e: regex::Error) -> Self {
        tracing::error!(error = ?e, "Regex error: {e:?}");
        AppError::Regex(e)
    }
}

#[tracing::instrument]
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "service-d-analytics"
    }))
}

/// Process analytics events and extract order IDs from log lines
///
/// INTENTIONAL BUG: Regex Compilation in Hot Path
#[tracing::instrument(skip(state))]
pub async fn process_events(
    State(state): State<AppState>,
    Json(req): Json<ProcessEventsRequest>,
) -> Result<Json<ProcessEventsResponse>, AppError> {
    tracing::info!(
        event_count = req.events.len(),
        "Processing analytics events"
    );

    let mut extracted_order_ids = Vec::new();
    let mut processed = 0;

    for event in req.events.iter() {
        // not even sure what we are doing here but its stupid
        for _ in 0..500 {
            let re = Regex::new(r"order_id: ([a-f0-9-]+)")?;
            let _ = re.captures(&event.log_line);
        }

        let order_id_pattern = Regex::new(r"order_id: ([a-f0-9-]+)")?;

        // Extract order_id from log line using the freshly compiled regex
        if let Some(caps) = order_id_pattern.captures(&event.log_line) {
            if let Some(order_id_str) = caps.get(1) {
                if let Ok(order_id) = Uuid::parse_str(order_id_str.as_str()) {
                    extracted_order_ids.push(order_id);

                    // Store in database
                    sqlx::query(
                        "INSERT INTO analytics_events (order_id, event_type, log_line) VALUES ($1, $2, $3)"
                    )
                    .bind(order_id)
                    .bind("order_processed")
                    .bind(&event.log_line)
                    .execute(&state.db)
                    .await?;

                    processed += 1;
                }
            }
        }
    }

    tracing::info!(
        processed,
        extracted_count = extracted_order_ids.len(),
        "Completed processing events (with inefficient regex compilation)"
    );

    Ok(Json(ProcessEventsResponse {
        processed,
        extracted_order_ids,
    }))
}

#[tracing::instrument(skip(state))]
pub async fn get_event_count(
    State(state): State<AppState>,
) -> Result<Json<EventCountResponse>, AppError> {
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM analytics_events CROSS JOIN (SELECT pg_sleep(0.2)) AS _sleep",
    )
    .fetch_one(&state.db)
    .await?;

    tracing::info!(count = count.0, "Fetched analytics event count");

    Ok(Json(EventCountResponse { count: count.0 }))
}
