//! Config status API handler.

use crate::config::Config;
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/config/status — Returns config status (which keys are set).
pub async fn handle(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let config = &state.config;
    let config_path = Config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let config_exists = Config::config_path().map(|p| p.exists()).unwrap_or(false);

    // Report which API keys are configured (without exposing values)
    let api_keys_status: serde_json::Value = serde_json::json!({
        "etherscan": config.chains.api_keys.contains_key("etherscan") ||
            std::env::var("ETHERSCAN_API_KEY").is_ok(),
        "polygonscan": config.chains.api_keys.contains_key("polygonscan"),
        "bscscan": config.chains.api_keys.contains_key("bscscan"),
        "solscan": config.chains.api_keys.contains_key("solscan"),
        "tronscan": config.chains.api_keys.contains_key("tronscan"),
    });

    let rpc_status = serde_json::json!({
        "ethereum_rpc": config.chains.ethereum_rpc.is_some(),
        "bsc_rpc": config.chains.bsc_rpc.is_some(),
        "solana_rpc": config.chains.solana_rpc.is_some(),
        "tron_api": config.chains.tron_api.is_some(),
    });

    Json(serde_json::json!({
        "config_path": config_path,
        "config_exists": config_exists,
        "output_format": format!("{:?}", config.output.format),
        "color_enabled": config.output.color,
        "api_keys": api_keys_status,
        "rpc_endpoints": rpc_status,
        "version": crate::VERSION,
    }))
    .into_response()
}

/// Request body for saving configuration.
#[derive(Debug, Deserialize)]
pub struct SaveConfigRequest {
    /// API keys to set (key name -> value).
    #[serde(default)]
    pub api_keys: std::collections::HashMap<String, String>,
    /// RPC endpoints to set.
    #[serde(default)]
    pub rpc_endpoints: std::collections::HashMap<String, String>,
}

/// POST /api/config — Save API keys and RPC endpoints.
pub async fn handle_save(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<SaveConfigRequest>,
) -> impl IntoResponse {
    // Load existing config or create new
    let mut config = Config::load(None).unwrap_or_default();

    // Update API keys
    for (key, value) in &req.api_keys {
        if !value.is_empty() {
            config.chains.api_keys.insert(key.clone(), value.clone());
        }
    }

    // Update RPC endpoints
    for (key, value) in &req.rpc_endpoints {
        if !value.is_empty() {
            match key.as_str() {
                "ethereum_rpc" => config.chains.ethereum_rpc = Some(value.clone()),
                "bsc_rpc" => config.chains.bsc_rpc = Some(value.clone()),
                "solana_rpc" => config.chains.solana_rpc = Some(value.clone()),
                "tron_api" => config.chains.tron_api = Some(value.clone()),
                _ => {}
            }
        }
    }

    // Save to disk
    let config_path = match Config::config_path() {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Cannot determine config path" })),
            )
                .into_response();
        }
    };

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create config dir: {}", e) })),
            )
                .into_response();
        }
    }

    match serde_yaml::to_string(&config) {
        Ok(yaml) => {
            if let Err(e) = std::fs::write(&config_path, yaml) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to write config: {}", e) })),
                )
                    .into_response();
            }
            Json(serde_json::json!({
                "status": "saved",
                "path": config_path.display().to_string(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to serialize config: {}", e) })),
        )
            .into_response(),
    }
}
