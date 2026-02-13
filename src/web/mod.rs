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
use axum::response::IntoResponse;
use axum::Router;
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
        "" | "index.html" => axum::response::Html(include_str!("static/index.html")).into_response(),
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
        // SPA fallback: serve index.html for client-side routing
        _ => axum::response::Html(include_str!("static/index.html")).into_response(),
    }
}

/// Starts the web server on the given address.
///
/// This is the main entry point called from the CLI `web` command handler.
pub async fn start_server(addr: SocketAddr, config: Config) -> anyhow::Result<()> {
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
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
    let pid: u32 = pid_str.trim().parse().map_err(|e| {
        anyhow::anyhow!("Invalid PID in {}: {}", pid_path.display(), e)
    })?;

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
        .args(["web", "--port", &addr.port().to_string(), "--bind", &addr.ip().to_string()])
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
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
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
}
