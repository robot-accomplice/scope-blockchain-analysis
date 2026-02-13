//! Token crawl API handler.

use crate::cli::crawl::{self, Period};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for token crawl.
#[derive(Debug, Deserialize)]
pub struct CrawlRequest {
    /// Token address or symbol.
    pub token: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Time period: "1h", "24h", "7d", "30d".
    #[serde(default)]
    pub period: Option<String>,
    /// Max holders to include.
    #[serde(default = "default_holders_limit")]
    pub holders_limit: u32,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

fn default_holders_limit() -> u32 {
    10
}

/// POST /api/crawl — Token analytics.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CrawlRequest>,
) -> impl IntoResponse {
    let period = match req.period.as_deref() {
        Some("1h") => Period::Hour1,
        Some("7d") => Period::Day7,
        Some("30d") => Period::Day30,
        _ => Period::Hour24,
    };

    match crawl::fetch_analytics_for_input(
        &req.token,
        &req.chain,
        period,
        req.holders_limit,
        &state.factory,
    )
    .await
    {
        Ok(analytics) => Json(serde_json::json!(analytics)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
