//! Unified insights API handler.

use crate::chains::ChainClientFactory;
use crate::cli::insights::{self, InsightsArgs};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for insights analysis.
#[derive(Debug, Deserialize)]
pub struct InsightsRequest {
    /// Target: address, tx hash, or token symbol/name.
    pub target: String,
    /// Override detected chain.
    pub chain: Option<String>,
    /// Decode tx input (for tx targets).
    #[serde(default)]
    pub decode: bool,
    /// Include internal trace (for tx targets).
    #[serde(default)]
    pub trace: bool,
}

/// POST /api/insights — Unified insights for any target.
///
/// Returns the insights markdown as JSON `{ "markdown": "..." }` along
/// with structured metadata about the detected target type.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InsightsRequest>,
) -> impl IntoResponse {
    let target = insights::infer_target(&req.target, req.chain.as_deref());

    let target_type = match &target {
        insights::InferredTarget::Address { chain } => {
            serde_json::json!({ "type": "address", "chain": chain })
        }
        insights::InferredTarget::Transaction { chain } => {
            serde_json::json!({ "type": "transaction", "chain": chain })
        }
        insights::InferredTarget::Token { chain } => {
            serde_json::json!({ "type": "token", "chain": chain })
        }
    };

    // Run the insights command which builds markdown output
    // We capture it by running the underlying functions directly
    let args = InsightsArgs {
        target: req.target.clone(),
        chain: req.chain,
        decode: req.decode,
        trace: req.trace,
    };

    // Run insights - it prints to stdout so we need to capture
    // For the web API, we reconstruct the data using the inferred target
    match &target {
        insights::InferredTarget::Address { chain } => {
            let addr_args = crate::cli::address::AddressArgs {
                address: req.target,
                chain: chain.clone(),
                format: None,
                include_txs: false,
                include_tokens: true,
                limit: 10,
                report: None,
                dossier: false,
            };
            let client: Box<dyn crate::chains::ChainClient> = match state.factory.create_chain_client(chain) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response();
                }
            };
            match crate::cli::address::analyze_address(&addr_args, client.as_ref()).await {
                Ok(report) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": report,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        insights::InferredTarget::Transaction { chain } => {
            match crate::cli::tx::fetch_transaction_report(
                &req.target,
                chain,
                args.decode,
                args.trace,
                &state.factory,
            )
            .await
            {
                Ok(report) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": report,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        insights::InferredTarget::Token { chain } => {
            match crate::cli::crawl::fetch_analytics_for_input(
                &req.target,
                chain,
                crate::cli::crawl::Period::Hour24,
                10,
                &state.factory,
            )
            .await
            {
                Ok(analytics) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": analytics,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
    }
}
