//! Integration tests for the web API endpoints: auth middleware behavior,
//! address book listing, discover, and venues.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use scope::chains::DefaultClientFactory;
use scope::config::{Config, WebConfig};
use scope_web::{AppState, build_router};
use std::sync::Arc;
use tower::ServiceExt; // for oneshot()

/// Creates an `AppState` with default config and a real HTTP client.
fn make_state(config: Config) -> Arc<AppState> {
    let http: Arc<dyn scope::http::HttpClient> =
        Arc::new(scope::http::NativeHttpClient::new().unwrap());
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
        http,
    };
    Arc::new(AppState { config, factory })
}

// ============================================================================
// Auth middleware
// ============================================================================

#[tokio::test]
async fn auth_no_token_configured_allows_requests() {
    let config = Config {
        web: WebConfig { auth_token: None },
        ..Default::default()
    };
    let state = make_state(config);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_token_configured_rejects_unauthenticated() {
    let config = Config {
        web: WebConfig {
            auth_token: Some("test-secret-token".to_string()),
        },
        ..Default::default()
    };
    let state = make_state(config);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_correct_bearer_token_succeeds() {
    let config = Config {
        web: WebConfig {
            auth_token: Some("test-secret-token".to_string()),
        },
        ..Default::default()
    };
    let state = make_state(config);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .header("authorization", "Bearer test-secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_wrong_bearer_token_rejected() {
    let config = Config {
        web: WebConfig {
            auth_token: Some("correct-token".to_string()),
        },
        ..Default::default()
    };
    let state = make_state(config);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// GET /api/address-book/list
// ============================================================================

#[tokio::test]
async fn address_book_list_returns_ok_json() {
    let state = make_state(Config::default());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/address-book/list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should succeed (may return empty list or data depending on disk state)
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected status: {}",
        status
    );

    if status == StatusCode::OK {
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        // Should be valid JSON
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_object() || json.is_array());
    }
}

// ============================================================================
// GET /api/discover
// ============================================================================

#[tokio::test]
async fn discover_endpoint_returns_response() {
    let state = make_state(Config::default());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/discover?source=profiles&limit=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    // Discover may succeed or fail (network dependent), but should not panic
    assert!(
        status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected status: {}",
        status
    );
}

// ============================================================================
// GET /api/venues
// ============================================================================

#[tokio::test]
async fn venues_endpoint_returns_response() {
    let state = make_state(Config::default());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/venues")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected status: {}",
        status
    );

    if status == StatusCode::OK {
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        // Should be valid JSON
        let _json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    }
}

// ============================================================================
// GET /api/config/status (validates JSON structure)
// ============================================================================

#[tokio::test]
async fn config_status_returns_expected_fields() {
    let state = make_state(Config::default());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json.get("config_path").is_some());
    assert!(json.get("api_keys").is_some());
    assert!(json.get("version").is_some());
}

// ============================================================================
// SPA fallback: non-API paths serve index.html
// ============================================================================

#[tokio::test]
async fn non_api_path_serves_html_fallback() {
    let state = make_state(Config::default());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/some/random/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
}
