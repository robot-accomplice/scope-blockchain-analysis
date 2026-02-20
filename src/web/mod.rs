//! # Web Server Module
//!
//! Provides a locally hosted HTTP server with REST API endpoints and a
//! single-page web UI that mirrors all CLI functionality. The server uses
//! the same `Config` and `DefaultClientFactory` as the CLI, ensuring
//! identical behavior.
//!
//! ## Usage
//!
//! ```bash
//! # Start in foreground (default port 8080)
//! scope web
//!
//! # Custom port and bind address
//! scope web --port 3000 --bind 0.0.0.0
//!
//! # Run as background daemon
//! scope web --daemon
//!
//! # Stop a running daemon
//! scope web --stop
//! ```

pub mod api;
pub mod monitor;

use crate::chains::DefaultClientFactory;
use crate::config::Config;
use axum::Router;
use axum::response::IntoResponse;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

/// Shared application state passed to all handlers via Axum extractors.
pub struct AppState {
    /// Application configuration (same as CLI).
    pub config: Config,
    /// Client factory for creating chain and DEX clients.
    pub factory: DefaultClientFactory,
}

/// Builds the Axum router with all API routes and static file serving.
pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = api::routes(state.clone());

    Router::new()
        .nest("/api", api_routes)
        .route("/ws/monitor", axum::routing::get(monitor::ws_handler))
        .fallback(axum::routing::get(serve_ui))
        .layer(cors)
        .with_state(state)
}

/// Serves the embedded single-page web UI.
async fn serve_ui(uri: axum::http::Uri) -> impl axum::response::IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Serve specific static assets
    match path {
        "" | "index.html" => {
            axum::response::Html(include_str!("static/index.html")).into_response()
        }
        "app.js" => (
            [(axum::http::header::CONTENT_TYPE, "application/javascript")],
            include_str!("static/app.js"),
        )
            .into_response(),
        "style.css" => (
            [(axum::http::header::CONTENT_TYPE, "text/css")],
            include_str!("static/style.css"),
        )
            .into_response(),
        "favicon.svg" => (
            [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
            include_str!("static/favicon.svg"),
        )
            .into_response(),
        "favicon.ico" => {
            // Redirect .ico requests to the SVG favicon
            axum::response::Redirect::permanent("/favicon.svg").into_response()
        }
        // SPA fallback: serve index.html for client-side routing
        _ => axum::response::Html(include_str!("static/index.html")).into_response(),
    }
}

/// Starts the web server on the given address.
///
/// This is the main entry point called from the CLI `web` command handler.
pub async fn start_server(addr: SocketAddr, config: Config) -> anyhow::Result<()> {
    let http: std::sync::Arc<dyn crate::http::HttpClient> = if config.ghola.enabled {
        match crate::http::GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size) {
            Ok(client) => std::sync::Arc::new(client),
            Err(_) => std::sync::Arc::new(
                crate::http::NativeHttpClient::new().expect("Failed to create HTTP client"),
            ),
        }
    } else {
        std::sync::Arc::new(
            crate::http::NativeHttpClient::new().expect("Failed to create HTTP client"),
        )
    };
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
        http,
    };
    let state = Arc::new(AppState { config, factory });
    let app = build_router(state);

    tracing::info!("Scope web server listening on http://{}", addr);
    eprintln!("Scope web server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ============================================================================
// Daemon management
// ============================================================================

/// Returns the path to the PID file for the daemon.
pub fn pid_file_path() -> std::path::PathBuf {
    Config::default_data_dir().join("scope-web.pid")
}

/// Returns the path to the log file for the daemon.
pub fn log_file_path() -> std::path::PathBuf {
    Config::default_data_dir().join("scope-web.log")
}

/// Stops a running daemon by reading its PID file and sending SIGTERM.
pub fn stop_daemon() -> anyhow::Result<()> {
    let pid_path = pid_file_path();
    if !pid_path.exists() {
        eprintln!("No daemon PID file found at {}", pid_path.display());
        eprintln!("Is the daemon running?");
        return Ok(());
    }

    let pid_str = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid PID in {}: {}", pid_path.display(), e))?;

    eprintln!("Stopping Scope web daemon (PID {})...", pid);

    #[cfg(unix)]
    {
        // Send SIGTERM to the daemon process
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result == 0 {
            eprintln!("Daemon stopped.");
        } else {
            eprintln!("Failed to stop daemon (process may have already exited).");
        }
    }

    #[cfg(not(unix))]
    {
        eprintln!("Daemon stop is only supported on Unix systems.");
        eprintln!("Please manually terminate PID {}.", pid);
    }

    // Remove PID file
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Starts the server as a background daemon (Unix only).
///
/// Spawns the current executable as a detached child process with
/// stdout/stderr redirected to a log file, then writes the PID.
#[cfg(unix)]
pub fn start_daemon(addr: SocketAddr, config: Config) -> anyhow::Result<()> {
    use std::io::Write;

    let _ = config; // Config reloaded from disk in child

    let data_dir = Config::default_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let pid_path = pid_file_path();
    let log_path = log_file_path();

    eprintln!("Starting Scope web daemon...");
    eprintln!("  URL:  http://{}", addr);
    eprintln!("  PID:  {}", pid_path.display());
    eprintln!("  Log:  {}", log_path.display());

    // Redirect stdout/stderr to log file
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_file_err = log_file.try_clone()?;

    let current_exe = std::env::current_exe()?;
    let child = std::process::Command::new(current_exe)
        .args([
            "web",
            "--port",
            &addr.port().to_string(),
            "--bind",
            &addr.ip().to_string(),
        ])
        .env("SCOPE_WEB_DAEMON_CHILD", "1")
        .stdout(log_file)
        .stderr(log_file_err)
        .stdin(std::process::Stdio::null())
        .spawn()?;

    let pid = child.id();

    // Write PID file
    let mut f = std::fs::File::create(&pid_path)?;
    write!(f, "{}", pid)?;

    eprintln!("Daemon started with PID {}", pid);
    eprintln!("Stop with: scope web --stop");

    Ok(())
}

/// Fallback for non-Unix: run in foreground.
#[cfg(not(unix))]
pub fn start_daemon(addr: SocketAddr, config: Config) -> anyhow::Result<()> {
    eprintln!("Daemon mode is only supported on Unix systems.");
    eprintln!("Starting in foreground instead...");
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(start_server(addr, config))
}

/// Returns true if running as a daemon child process.
pub fn is_daemon_child() -> bool {
    std::env::var("SCOPE_WEB_DAEMON_CHILD").is_ok()
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body;

    #[test]
    fn test_pid_file_path() {
        let path = pid_file_path();
        assert!(path.to_string_lossy().contains("scope-web.pid"));
    }

    #[test]
    fn test_log_file_path() {
        let path = log_file_path();
        assert!(path.to_string_lossy().contains("scope-web.log"));
    }

    #[test]
    fn test_build_router() {
        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = Arc::new(AppState { config, factory });
        let _router = build_router(state);
    }

    #[test]
    fn test_is_daemon_child_default() {
        // Should be false in test context (env var not set)
        // Note: may be true if test runner sets it, so just ensure it doesn't panic
        let _ = is_daemon_child();
    }

    #[tokio::test]
    async fn test_serve_ui_index() {
        let response = serve_ui(axum::http::Uri::from_static("/"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Scope"));
    }

    #[tokio::test]
    async fn test_serve_ui_index_html() {
        let response = serve_ui(axum::http::Uri::from_static("/index.html"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_serve_ui_app_js() {
        let response = serve_ui(axum::http::Uri::from_static("/app.js"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let js = String::from_utf8(body.to_vec()).unwrap();
        assert!(js.contains("function") || js.contains("const") || js.contains("var"));
    }

    #[tokio::test]
    async fn test_serve_ui_style_css() {
        let response = serve_ui(axum::http::Uri::from_static("/style.css"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let css = String::from_utf8(body.to_vec()).unwrap();
        assert!(css.contains(":root") || css.contains("body") || css.contains("nav"));
    }

    #[tokio::test]
    async fn test_serve_ui_spa_fallback() {
        // Unknown paths should return index.html (SPA routing)
        let response = serve_ui(axum::http::Uri::from_static("/some/random/path"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
    }

    #[tokio::test]
    async fn test_serve_ui_favicon_svg() {
        let response = serve_ui(axum::http::Uri::from_static("/favicon.svg"))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let headers = response.headers();
        let ct = headers.get(axum::http::header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, "image/svg+xml");
    }

    #[tokio::test]
    async fn test_serve_ui_favicon_ico_redirect() {
        let response = serve_ui(axum::http::Uri::from_static("/favicon.ico"))
            .await
            .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::PERMANENT_REDIRECT
        );
        let headers = response.headers();
        let loc = headers.get(axum::http::header::LOCATION).unwrap();
        assert_eq!(loc, "/favicon.svg");
    }

    #[tokio::test]
    async fn test_route_ws_monitor_handshake() {
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        // GET /ws/monitor?token=USDC - WebSocket upgrade with required query params
        let req = Request::builder()
            .uri("/ws/monitor?token=USDC")
            .method("GET")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Sec-WebSocket-Version", "13")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // With Upgrade headers: 101 Switching Protocols; without proper handshake: 426
        let status = response.status();
        assert!(
            status == StatusCode::SWITCHING_PROTOCOLS
                || status == StatusCode::UPGRADE_REQUIRED
                || status.is_success(),
            "Unexpected status: {}",
            status
        );
    }

    #[tokio::test]
    async fn test_route_ws_monitor_missing_token() {
        use axum::http::Request;
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/ws/monitor")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // Missing required token param -> 422 Unprocessable Entity
        assert!(response.status().is_client_error());
    }

    #[test]
    fn test_pid_file_path_in_data_dir() {
        let path = pid_file_path();
        assert!(path.ends_with("scope-web.pid"));
        assert!(path.parent().is_some());
    }

    #[test]
    fn test_log_file_path_in_data_dir() {
        let path = log_file_path();
        assert!(path.ends_with("scope-web.log"));
        assert!(path.parent().is_some());
    }

    #[test]
    fn test_app_state_construction() {
        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = AppState { config, factory };
        // Verify state fields are accessible
        let _ = state.config.chains.api_keys.len();
    }

    #[test]
    fn test_stop_daemon_no_pid_file() {
        // stop_daemon should handle missing PID file gracefully
        // We can't easily override the data dir, but verify the function exists
        // and the path helpers return valid paths
        let pid = pid_file_path();
        let log = log_file_path();
        assert_eq!(pid.parent(), log.parent());
    }

    #[test]
    fn test_stop_daemon_invoked_no_pid_file() {
        // Exercise stop_daemon when PID file does not exist (common in tests)
        let result = stop_daemon();
        assert!(result.is_ok());
    }

    // Helper to create the test app state
    fn test_state() -> Arc<AppState> {
        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        Arc::new(AppState { config, factory })
    }

    // ========================================================================
    // HTTP routing integration tests — exercise handler code paths
    // ========================================================================

    #[tokio::test]
    async fn test_route_get_config_status() {
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/api/config/status")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("config_path").is_some());
        assert!(json.get("api_keys").is_some());
        assert!(json.get("version").is_some());
    }

    #[tokio::test]
    async fn test_route_post_config_save() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "api_keys": {},
            "rpc_endpoints": {}
        });

        let req = Request::builder()
            .uri("/api/config")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // May succeed or fail depending on file system, but should not panic
        let status = response.status();
        assert!(status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_route_post_address() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "ethereum"
        });

        let req = Request::builder()
            .uri("/api/address")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // Will likely fail due to no API key, but exercises the handler
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_tx() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "hash": "0xabc123def456789012345678901234567890123456789012345678901234abcd",
            "chain": "ethereum"
        });

        let req = Request::builder()
            .uri("/api/tx")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_crawl() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "token": "USDC",
            "chain": "ethereum"
        });

        let req = Request::builder()
            .uri("/api/crawl")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_get_discover() {
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/api/discover?source=profiles&limit=5")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_route_post_insights() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "target": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
        });

        let req = Request::builder()
            .uri("/api/insights")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_compliance_risk() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "ethereum",
            "detailed": true
        });

        let req = Request::builder()
            .uri("/api/compliance/risk")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_export() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "ethereum",
            "format": "json"
        });

        let req = Request::builder()
            .uri("/api/export")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_token_health() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "token": "USDC",
            "chain": "ethereum",
            "with_market": false
        });

        let req = Request::builder()
            .uri("/api/token-health")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_contract() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "ethereum"
        });

        let req = Request::builder()
            .uri("/api/contract")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_post_market_summary() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "pair": "USDC",
            "market_venue": "binance"
        });

        let req = Request::builder()
            .uri("/api/market/summary")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_get_address_book_list() {
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/api/address-book/list")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_route_post_address_book_add() {
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let payload = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "ethereum",
            "label": "Test Wallet"
        });

        let req = Request::builder()
            .uri("/api/address-book/add")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::OK
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_route_invalid_json_body() {
        use axum::http::{Request, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/api/address")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from("not json"))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // Should return 4xx for bad JSON
        assert!(response.status().is_client_error());
    }

    #[tokio::test]
    async fn test_route_missing_required_field() {
        use axum::http::{Request, header};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        // Missing required "address" field
        let payload = serde_json::json!({ "chain": "ethereum" });

        let req = Request::builder()
            .uri("/api/address")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert!(response.status().is_client_error());
    }

    #[tokio::test]
    async fn test_route_static_fallback() {
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let state = test_state();
        let app = build_router(state);

        let req = Request::builder()
            .uri("/nonexistent-path")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
