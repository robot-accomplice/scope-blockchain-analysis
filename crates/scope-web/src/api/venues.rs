//! Venue listing API handler.
//!
//! GET /api/venues — Returns the CEX venues with their capabilities and the
//! DEX venues (`dex_venues`), so the web UI hard-codes no venue.

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use scope::market::{DexVenue, VenueRegistry};

/// GET /api/venues — List available exchange venues.
pub async fn handle() -> impl IntoResponse {
    let registry = match VenueRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to load venue registry: {e}") })),
            )
                .into_response();
        }
    };

    let venues: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .filter_map(|id| {
            registry.get(id).map(|desc| {
                serde_json::json!({
                    "id": desc.id,
                    "name": desc.name,
                    "base_url": desc.base_url,
                    "capabilities": desc.capability_names(),
                })
            })
        })
        .collect();

    let dex_venues: Vec<serde_json::Value> = DexVenue::ALL
        .iter()
        .map(|d| serde_json::json!({ "id": d.id(), "name": d.name(), "chain": d.chain() }))
        .collect();

    let output = serde_json::json!({
        "venues": venues,
        "dex_venues": dex_venues,
        "total": registry.len(),
        "user_venues_dir": VenueRegistry::user_venues_dir().display().to_string(),
    });

    Json(output).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn test_handle_venues() {
        let response = handle().await.into_response();
        let status = response.status();
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn test_handle_venues_returns_all_built_in() {
        let response = handle().await.into_response();
        let status = response.status();
        assert!(status.is_success());

        // Extract body and verify structure
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].as_u64().unwrap() >= 11);
        assert!(json["venues"].is_array());
        let venues = json["venues"].as_array().unwrap();
        assert!(venues.iter().any(|v| v["id"] == "binance"));
        assert!(venues.iter().any(|v| v["id"] == "kraken"));
        // Verify each venue has expected fields
        for venue in venues {
            assert!(venue["id"].is_string());
            assert!(venue["name"].is_string());
            assert!(venue["base_url"].is_string());
            assert!(venue["capabilities"].is_array());
        }
    }

    #[tokio::test]
    async fn test_handle_venues_response_structure() {
        let response = handle().await.into_response();
        assert!(response.status().is_success());
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total"].as_u64().unwrap() >= 11);
        let venues = json["venues"].as_array().unwrap();
        for v in venues {
            assert!(v["id"].is_string());
            assert!(v["name"].is_string());
            assert!(v["capabilities"].is_array());
        }
        assert!(json["user_venues_dir"].is_string());
    }

    /// The UI reads DEX venues from here instead of hard-coding them.
    #[tokio::test]
    async fn test_handle_venues_lists_dex_venues() {
        let response = handle().await.into_response();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids: Vec<&str> = json["dex_venues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["ethereum", "solana"]);
        assert_eq!(json["dex_venues"][1]["name"], "Solana DEX");
    }
}
