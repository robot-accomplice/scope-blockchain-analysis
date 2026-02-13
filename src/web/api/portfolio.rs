//! Portfolio management API handlers.

use crate::cli::portfolio::{Portfolio, WatchedAddress};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/portfolio/list — List portfolio addresses.
pub async fn handle_list(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    match Portfolio::load(&data_dir) {
        Ok(portfolio) => {
            Json(serde_json::json!({
                "addresses": portfolio.addresses,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Request body for adding a portfolio address.
#[derive(Debug, Deserialize)]
pub struct AddPortfolioRequest {
    /// Blockchain address.
    pub address: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Optional label.
    pub label: Option<String>,
    /// Optional tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// POST /api/portfolio/add — Add an address to the portfolio.
pub async fn handle_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddPortfolioRequest>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    let mut portfolio = match Portfolio::load(&data_dir) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let watched = WatchedAddress {
        address: req.address.clone(),
        label: req.label,
        chain: req.chain,
        tags: req.tags,
        added_at: chrono::Utc::now().timestamp() as u64,
    };

    match portfolio.add_address(watched) {
        Ok(_) => {
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = portfolio.save(&data_dir_buf) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            Json(serde_json::json!({ "status": "added", "address": req.address })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
