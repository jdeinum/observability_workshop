use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid,
    pub customer_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct OrderItem {
    pub id: Uuid,
    pub order_id: Uuid,
    pub product_id: Uuid,
    pub quantity: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub price: rust_decimal::Decimal,
    pub category: String,
}

#[derive(Debug, Serialize)]
pub struct EnrichedOrderItem {
    pub product_id: Uuid,
    pub product_name: String,
    pub quantity: i32,
    pub price: rust_decimal::Decimal,
}

#[derive(Debug, Serialize)]
pub struct OrderResponse {
    pub id: Uuid,
    pub customer_name: String,
    pub items: Vec<EnrichedOrderItem>,
    pub created_at: DateTime<Utc>,
    pub total_items: usize,
}

#[derive(Debug, Serialize)]
pub struct OrderListResponse {
    pub orders: Vec<OrderSummary>,
    pub count: usize,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct OrderSummary {
    pub id: Uuid,
    pub customer_name: String,
    pub item_count: i64,
    pub created_at: DateTime<Utc>,
}

// Error handling — thiserror provides Display, manual From impls provide logging.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(sqlx::Error),
    #[error("Redis error: {0}")]
    Redis(fred::error::RedisError),
    #[error("Upstream service error: {0}")]
    Http(reqwest_middleware::Error),
    #[error("Serialization error: {0}")]
    Serialization(serde_json::Error),
    #[error("Order not found")]
    NotFound,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Database(_) | AppError::Redis(_) | AppError::Serialization(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::Http(_) => StatusCode::BAD_GATEWAY,
            AppError::NotFound => StatusCode::NOT_FOUND,
        };

        (status, self.to_string()).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound,
            _ => {
                tracing::error!(error = ?e, "Database error: {e:?}");
                AppError::Database(e)
            }
        }
    }
}

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

#[tracing::instrument]
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "service-b-orders"
    }))
}

#[tracing::instrument(skip(state))]
pub async fn list_orders(
    State(state): State<AppState>,
) -> Result<Json<OrderListResponse>, AppError> {
    let orders = sqlx::query_as::<_, OrderSummary>(
        r#"
        SELECT
            o.id,
            o.customer_name,
            COUNT(oi.id) as item_count,
            o.created_at
        FROM orders o
        CROSS JOIN (SELECT pg_sleep(0.2)) AS _sleep
        LEFT JOIN order_items oi ON o.id = oi.order_id
        GROUP BY o.id, o.customer_name, o.created_at
        ORDER BY o.created_at DESC
        LIMIT 50
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let count = orders.len();

    tracing::info!(count, "Listed orders");

    Ok(Json(OrderListResponse { orders, count }))
}

#[tracing::instrument(skip(state), fields(order_id = %order_id))]
pub async fn get_order(
    State(state): State<AppState>,
    Path(order_id): Path<Uuid>,
) -> Result<Json<OrderResponse>, AppError> {
    tracing::info!("Fetching order details");

    // Fetch order from database
    let order = sqlx::query_as::<_, Order>(
        "SELECT id, customer_name, created_at FROM orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_one(&state.db)
    .await?;

    // Fetch order items
    let items = sqlx::query_as::<_, OrderItem>(
        "SELECT id, order_id, product_id, quantity FROM order_items WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_all(&state.db)
    .await?;

    tracing::info!(
        item_count = items.len(),
        "Fetched order items, now fetching product details"
    );

    // BUG: N+1 Query Pattern
    let mut enriched_items = Vec::new();
    for item in items.iter() {
        // Each iteration makes a separate HTTP call - this is the N+1 bug!
        let product_url = format!("{}/api/products/{}", state.service_c_url, item.product_id);

        let product = state
            .http_client
            .get(&product_url)
            .send()
            .await?
            .error_for_status()?
            .json::<Product>()
            .await?;

        enriched_items.push(EnrichedOrderItem {
            product_id: item.product_id,
            product_name: product.name,
            quantity: item.quantity,
            price: product.price,
        });
    }

    tracing::info!(
        enriched_items = enriched_items.len(),
        "Completed fetching all product details (via N+1 calls)"
    );

    Ok(Json(OrderResponse {
        id: order.id,
        customer_name: order.customer_name,
        items: enriched_items,
        created_at: order.created_at,
        total_items: items.len(),
    }))
}
