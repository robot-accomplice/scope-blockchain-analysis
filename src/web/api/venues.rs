//! Venue listing API handler.
//!
//! GET /api/venues — Returns available exchange venues and their capabilities.

use crate::market::VenueRegistry;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;

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

    let output = serde_json::json!({
        "venues": venues,
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
}
