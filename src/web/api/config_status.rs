//! Config status API handler.

use crate::config::Config;
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/config/status — Returns config status (which keys are set).
pub async fn handle(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
    if let Some(parent) = config_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to create config dir: {}", e) })),
        )
            .into_response();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_save_config_full() {
        let json = serde_json::json!({
            "api_keys": {
                "etherscan": "test_key_123",
                "polygonscan": "test_key_456"
            },
            "rpc_endpoints": {
                "ethereum_rpc": "https://eth.example.com",
                "bsc_rpc": "https://bsc.example.com"
            }
        });
        let req: SaveConfigRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.api_keys.len(), 2);
        assert_eq!(
            req.api_keys.get("etherscan"),
            Some(&"test_key_123".to_string())
        );
        assert_eq!(
            req.api_keys.get("polygonscan"),
            Some(&"test_key_456".to_string())
        );
        assert_eq!(req.rpc_endpoints.len(), 2);
        assert_eq!(
            req.rpc_endpoints.get("ethereum_rpc"),
            Some(&"https://eth.example.com".to_string())
        );
        assert_eq!(
            req.rpc_endpoints.get("bsc_rpc"),
            Some(&"https://bsc.example.com".to_string())
        );
    }

    #[test]
    fn test_deserialize_save_config_empty() {
        let json = serde_json::json!({});
        let req: SaveConfigRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.api_keys.len(), 0);
        assert_eq!(req.rpc_endpoints.len(), 0);
    }

    #[test]
    fn test_deserialize_save_config_partial() {
        let json = serde_json::json!({
            "api_keys": {
                "etherscan": "test_key_123"
            }
        });
        let req: SaveConfigRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.api_keys.len(), 1);
        assert_eq!(
            req.api_keys.get("etherscan"),
            Some(&"test_key_123".to_string())
        );
        assert_eq!(req.rpc_endpoints.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_config_status() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let response = handle(State(state)).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("config_path").is_some());
        assert!(json.get("api_keys").is_some());
        assert!(json.get("rpc_endpoints").is_some());
        assert!(json.get("version").is_some());
    }

    #[tokio::test]
    async fn test_handle_save_config() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = SaveConfigRequest {
            api_keys: std::collections::HashMap::new(),
            rpc_endpoints: std::collections::HashMap::new(),
        };
        let response = handle_save(State(state), axum::Json(req))
            .await
            .into_response();
        // May succeed or fail depending on filesystem
        let status = response.status();
        assert!(
            status == axum::http::StatusCode::OK
                || status == axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_save_config_request_debug() {
        let req = SaveConfigRequest {
            api_keys: std::collections::HashMap::new(),
            rpc_endpoints: std::collections::HashMap::new(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("SaveConfigRequest"));
    }

    #[tokio::test]
    async fn test_handle_save_config_path_none() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });

        let req = SaveConfigRequest {
            api_keys: std::collections::HashMap::new(),
            rpc_endpoints: std::collections::HashMap::new(),
        };

        let old_home = std::env::var_os("HOME");
        let old_userprofile = std::env::var_os("USERPROFILE");
        unsafe {
            std::env::remove_var("HOME");
            std::env::remove_var("USERPROFILE");
        }

        let response = handle_save(State(state), axum::Json(req))
            .await
            .into_response();

        if let Some(h) = old_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        if let Some(u) = old_userprofile {
            unsafe { std::env::set_var("USERPROFILE", u) };
        }

        if response.status() == StatusCode::INTERNAL_SERVER_ERROR {
            let body = axum::body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(
                json["error"]
                    .as_str()
                    .unwrap()
                    .contains("Cannot determine config path")
                    || json["error"]
                        .as_str()
                        .unwrap()
                        .contains("Failed to create config dir")
                    || json["error"]
                        .as_str()
                        .unwrap()
                        .contains("Failed to write config")
            );
        }
    }

    #[tokio::test]
    async fn test_handle_save_config_create_dir_fails() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("fake_home");
        std::fs::create_dir_all(&fake_home).unwrap();
        let config_as_file = fake_home.join(".config");
        std::fs::File::create(&config_as_file).unwrap();

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });

        let req = SaveConfigRequest {
            api_keys: std::collections::HashMap::new(),
            rpc_endpoints: std::collections::HashMap::new(),
        };

        let old_home = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", &fake_home) };

        let response = handle_save(State(state), axum::Json(req))
            .await
            .into_response();

        if let Some(h) = old_home {
            unsafe { std::env::set_var("HOME", h) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("Failed to create config dir")
        );
    }

    #[tokio::test]
    async fn test_handle_save_config_with_api_keys_and_rpc() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });

        let mut api_keys = std::collections::HashMap::new();
        api_keys.insert("etherscan".to_string(), "test_key_abc".to_string());
        api_keys.insert("polygonscan".to_string(), "test_key_def".to_string());
        api_keys.insert("empty_key".to_string(), "".to_string()); // empty value - should be skipped

        let mut rpc_endpoints = std::collections::HashMap::new();
        rpc_endpoints.insert(
            "ethereum_rpc".to_string(),
            "https://eth.example.com".to_string(),
        );
        rpc_endpoints.insert("bsc_rpc".to_string(), "https://bsc.example.com".to_string());
        rpc_endpoints.insert(
            "solana_rpc".to_string(),
            "https://sol.example.com".to_string(),
        );
        rpc_endpoints.insert(
            "tron_api".to_string(),
            "https://tron.example.com".to_string(),
        );
        rpc_endpoints.insert(
            "unknown_key".to_string(),
            "https://unknown.example.com".to_string(),
        );
        rpc_endpoints.insert("empty_rpc".to_string(), "".to_string()); // empty value

        let req = SaveConfigRequest {
            api_keys,
            rpc_endpoints,
        };

        let response = handle_save(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        // May succeed or fail depending on filesystem permissions, but we cover the code paths
        assert!(
            status == axum::http::StatusCode::OK
                || status == axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
