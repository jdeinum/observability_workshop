use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub price: rust_decimal::Decimal,
    pub category: String,
}

// Error handling — thiserror provides Display, manual From impls provide logging.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
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

#[tracing::instrument]
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "service-c-products"
    }))
}

#[tracing::instrument(skip(state))]
pub async fn get_product(
    State(state): State<AppState>,
    Path(product_id): Path<Uuid>,
) -> Result<Json<Product>, AppError> {
    // Connection pool exhaustion happens HERE when Service B hammers this endpoint
    // The pool only has 5 connections, but N+1 queries create 50+ concurrent requests
    let product = sqlx::query_as::<_, Product>(
        "SELECT id, name, price, category FROM products WHERE id = $1",
    )
    .bind(product_id)
    .fetch_one(&state.db)
    .await?;

    tracing::info!(product_id = %product_id, "Product fetched from database");

    Ok(Json(product))
}
