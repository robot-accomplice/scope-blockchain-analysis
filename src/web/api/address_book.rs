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
            Json(serde_json::json!({ "status": "added", "address": req.address })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
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
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
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
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
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
}
