//! Compliance risk assessment API handler.

use crate::compliance::datasource::{BlockchainDataClient, DataSources};
use crate::compliance::risk::RiskEngine;
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for compliance risk analysis.
#[derive(Debug, Deserialize)]
pub struct ComplianceRiskRequest {
    /// Address to assess.
    pub address: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Include detailed breakdown.
    #[serde(default)]
    pub detailed: bool,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// POST /api/compliance/risk — Risk assessment for an address.
pub async fn handle_risk(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<ComplianceRiskRequest>,
) -> impl IntoResponse {
    // Build risk engine (with Etherscan key if available)
    let engine = if let Ok(key) = std::env::var("ETHERSCAN_API_KEY") {
        let sources = DataSources::new(key);
        let client = BlockchainDataClient::new(sources);
        RiskEngine::with_data_client(client)
    } else {
        RiskEngine::new()
    };

    match engine.assess_address(&req.address, &req.chain).await {
        Ok(assessment) => Json(serde_json::json!({
            "address": assessment.address,
            "chain": assessment.chain,
            "overall_score": assessment.overall_score,
            "risk_level": format!("{:?}", assessment.risk_level),
            "factors": assessment.factors.iter().map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "weight": f.weight,
                    "score": f.score,
                    "description": f.description,
                })
            }).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
