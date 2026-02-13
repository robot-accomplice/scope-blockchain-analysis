//! Token discovery API handler.

use crate::chains::DexClient;
use crate::web::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Query parameters for token discovery.
#[derive(Debug, Deserialize)]
pub struct DiscoverQuery {
    /// Source: "profiles", "boosts", "top-boosts".
    #[serde(default = "default_source")]
    pub source: String,
    /// Filter by chain (optional).
    pub chain: Option<String>,
    /// Max results (default: 15).
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_source() -> String {
    "profiles".to_string()
}

fn default_limit() -> u32 {
    15
}

/// GET /api/discover — Browse trending/boosted tokens.
pub async fn handle(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<DiscoverQuery>,
) -> impl IntoResponse {
    let client = DexClient::new();

    let tokens = match params.source.as_str() {
        "boosts" => client.get_token_boosts().await,
        "top-boosts" => client.get_token_boosts_top().await,
        _ => client.get_token_profiles().await,
    };

    match tokens {
        Ok(tokens) => {
            let filtered: Vec<_> = if let Some(ref chain) = params.chain {
                let c = chain.to_lowercase();
                tokens
                    .into_iter()
                    .filter(|t| t.chain_id.to_lowercase() == c)
                    .take(params.limit as usize)
                    .collect()
            } else {
                tokens.into_iter().take(params.limit as usize).collect()
            };
            Json(serde_json::json!(filtered)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
