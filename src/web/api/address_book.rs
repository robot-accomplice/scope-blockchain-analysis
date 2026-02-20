//! Address book management API handlers.

use crate::cli::address_book::{AddressBook, WatchedAddress};
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/address-book/list — List address book entries.
pub async fn handle_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    match AddressBook::load(&data_dir) {
        Ok(address_book) => Json(serde_json::json!({
            "addresses": address_book.addresses,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Request body for adding an address book entry.
#[derive(Debug, Deserialize)]
pub struct AddAddressBookRequest {
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

/// POST /api/address-book/add — Add an address to the address book.
pub async fn handle_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddAddressBookRequest>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    let mut address_book = match AddressBook::load(&data_dir) {
        Ok(ab) => ab,
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

    match address_book.add_address(watched) {
        Ok(_) => {
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = address_book.save(&data_dir_buf) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            Json(serde_json::json!({
                "status": "added",
                "address": req.address,
                "addresses": address_book.addresses,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Request body for removing an address book entry.
#[derive(Debug, Deserialize)]
pub struct RemoveAddressBookRequest {
    /// Blockchain address to remove.
    pub address: String,
}

/// POST /api/address-book/remove — Remove an address from the address book.
pub async fn handle_remove(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RemoveAddressBookRequest>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    let mut address_book = match AddressBook::load(&data_dir) {
        Ok(ab) => ab,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match address_book.remove_address(&req.address) {
        Ok(true) => {
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = address_book.save(&data_dir_buf) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            Json(serde_json::json!({
                "status": "removed",
                "address": req.address,
                "addresses": address_book.addresses,
            }))
            .into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Address '{}' not found in address book", req.address) })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "chain": "polygon",
            "label": "My Wallet",
            "tags": ["defi", "nft"]
        });
        let req: AddAddressBookRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "polygon");
        assert_eq!(req.label, Some("My Wallet".to_string()));
        assert_eq!(req.tags.len(), 2);
        assert_eq!(req.tags[0], "defi");
        assert_eq!(req.tags[1], "nft");
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: AddAddressBookRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.label, None);
        assert_eq!(req.tags.len(), 0);
    }

    #[test]
    fn test_default_chain() {
        assert_eq!(default_chain(), "ethereum");
    }

    #[test]
    fn test_with_tags() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "tags": ["tag1", "tag2", "tag3"]
        });
        let req: AddAddressBookRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.tags.len(), 3);
        assert_eq!(req.tags[0], "tag1");
        assert_eq!(req.tags[1], "tag2");
        assert_eq!(req.tags[2], "tag3");
    }

    #[test]
    fn test_with_label() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "label": "Test Label"
        });
        let req: AddAddressBookRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.label, Some("Test Label".to_string()));
    }

    #[tokio::test]
    async fn test_handle_address_book_list_direct() {
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
        let response = handle_list(State(state)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_address_book_add_direct() {
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
        let req = AddAddressBookRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            label: Some("Test".to_string()),
            tags: vec!["test".to_string()],
        };
        let response = handle_add(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[test]
    fn test_deserialize_remove_request() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: RemoveAddressBookRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
    }

    #[tokio::test]
    async fn test_handle_address_book_remove_direct() {
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
        let req = RemoveAddressBookRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
        };
        let response = handle_remove(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        // 200 (removed), 404 (not found), or 500 (load/save error)
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::NOT_FOUND
                || status.is_server_error()
        );
    }

    #[tokio::test]
    async fn test_handle_address_book_remove_nonexistent() {
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
        let req = RemoveAddressBookRequest {
            address: "0x000000000000000000000000000000000000dead".to_string(),
        };
        let response = handle_remove(State(state), axum::Json(req))
            .await
            .into_response();
        // Should return 404 (not found) or 500 (load error)
        assert!(
            response.status() == axum::http::StatusCode::NOT_FOUND
                || response.status().is_server_error()
                || response.status().is_success()
        );
    }

    #[tokio::test]
    async fn test_handle_address_book_list_json_structure() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use axum::body;
        use axum::extract::State;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let response = handle_list(State(state)).await.into_response();
        if response.status().is_success() {
            let body_bytes = body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(json.get("addresses").is_some());
        }
    }

    #[tokio::test]
    async fn test_handle_address_book_add_duplicate_returns_bad_request() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let tmp_dir = tempfile::tempdir().unwrap();
        let data_dir = tmp_dir.path().to_path_buf();
        let mut config = Config::default();
        config.address_book.data_dir = Some(data_dir.clone());

        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });

        let addr = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string();
        let req1 = AddAddressBookRequest {
            address: addr.clone(),
            chain: "ethereum".to_string(),
            label: Some("First".to_string()),
            tags: vec![],
        };
        let r1 = handle_add(State(state.clone()), axum::Json(req1))
            .await
            .into_response();
        if !r1.status().is_success() {
            return;
        }

        let req2 = AddAddressBookRequest {
            address: addr,
            chain: "ethereum".to_string(),
            label: Some("Duplicate".to_string()),
            tags: vec![],
        };
        let r2 = handle_add(State(state), axum::Json(req2))
            .await
            .into_response();
        assert_eq!(r2.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(r2.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("already")
        );
    }

    #[tokio::test]
    async fn test_handle_address_book_list_corrupt_file_returns_500() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;

        let tmp_dir = tempfile::tempdir().unwrap();
        let yaml_path = tmp_dir.path().join("address_book.yaml");
        std::fs::write(&yaml_path, "{{{ invalid yaml").unwrap();

        let mut config = Config::default();
        config.address_book.data_dir = Some(tmp_dir.path().to_path_buf());
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let response = handle_list(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
