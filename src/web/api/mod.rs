//! # Web API Handlers
//!
//! REST API endpoints mirroring CLI commands. Each handler accepts JSON
//! request bodies matching CLI argument structures and returns JSON responses.

pub mod address;
pub mod compliance;
pub mod config_status;
pub mod crawl;
pub mod discover;
pub mod export;
pub mod insights;
pub mod market;
pub mod portfolio;
pub mod token_health;
pub mod tx;

use crate::web::AppState;
use axum::Router;
use std::sync::Arc;

/// Registers all API routes under the `/api` prefix.
pub fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/address", axum::routing::post(address::handle))
        .route("/tx", axum::routing::post(tx::handle))
        .route("/insights", axum::routing::post(insights::handle))
        .route("/crawl", axum::routing::post(crawl::handle))
        .route("/discover", axum::routing::get(discover::handle))
        .route("/token-health", axum::routing::post(token_health::handle))
        .route("/market/summary", axum::routing::post(market::handle))
        .route("/portfolio/list", axum::routing::get(portfolio::handle_list))
        .route("/portfolio/add", axum::routing::post(portfolio::handle_add))
        .route("/export", axum::routing::post(export::handle))
        .route("/compliance/risk", axum::routing::post(compliance::handle_risk))
        .route("/config/status", axum::routing::get(config_status::handle))
        .route("/config", axum::routing::post(config_status::handle_save))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::chains::DefaultClientFactory;

    #[test]
    fn test_routes_construction() {
        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = Arc::new(AppState { config, factory });
        let _router = routes(state);
        // If this doesn't panic, routes are properly constructed
    }
}
