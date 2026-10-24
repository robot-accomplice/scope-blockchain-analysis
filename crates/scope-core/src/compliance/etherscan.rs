//! # Etherscan access for compliance
//!
//! The one place that knows where the Etherscan API key comes from, and
//! the one way to build a risk engine with or without Etherscan data.

use super::datasource::BlockchainDataClient;
use super::risk::RiskEngine;

/// The environment variable that holds the Etherscan API key.
pub const ETHERSCAN_KEY_ENV: &str = "ETHERSCAN_API_KEY";

/// The Etherscan API key: [`ETHERSCAN_KEY_ENV`] when it is set, else
/// `chains.api_keys.etherscan` in the config file. Reading only the
/// variable ignored the key that `scope setup` saves, so compliance ran
/// without data while the rest of scope used the key.
pub fn etherscan_key() -> Option<String> {
    let configured = || {
        crate::config::Config::load(None)
            .ok()
            .and_then(|c| c.chains.api_keys.get("etherscan").cloned())
    };
    key_from(std::env::var(ETHERSCAN_KEY_ENV).ok(), configured)
}

/// The first non-empty key: `env`, then `configured()`.
fn key_from(env: Option<String>, configured: impl FnOnce() -> Option<String>) -> Option<String> {
    env.filter(|k| !k.trim().is_empty())
        .or_else(configured)
        .filter(|k| !k.trim().is_empty())
}

impl RiskEngine {
    /// A risk engine that uses Etherscan data when there is a `client`,
    /// and basic scoring when there is none.
    pub fn with_optional_client(client: Option<BlockchainDataClient>) -> Self {
        match client {
            Some(c) => Self::with_data_client(c),
            None => Self::new(),
        }
    }

    /// A risk engine with Etherscan data when [`ETHERSCAN_KEY_ENV`] is set.
    #[deprecated(
        since = "0.8.0",
        note = "pass the transport: use `RiskEngine::from_env_with_http`"
    )]
    pub fn from_env() -> Self {
        Self::from_env_with_http(crate::http::shared::fallback())
    }

    /// [`RiskEngine::from_env`], sending Etherscan requests through `http`.
    pub fn from_env_with_http(http: std::sync::Arc<dyn crate::http::HttpClient>) -> Self {
        Self::with_optional_client(BlockchainDataClient::from_env_opt_with_http(http))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::datasource::DataSources;

    #[test]
    fn test_optional_client_picks_the_engine() {
        // Both shapes must build; the data client is used only when given.
        let _basic = RiskEngine::with_optional_client(None);
        let client = BlockchainDataClient::new_with_http(
            DataSources::new("k".to_string()),
            crate::http::test_transport(),
        );
        let _rich = RiskEngine::with_optional_client(Some(client));
        assert_eq!(ETHERSCAN_KEY_ENV, "ETHERSCAN_API_KEY");
    }

    #[test]
    fn test_the_variable_wins_and_the_config_key_is_the_fallback() {
        let config = || Some("from-config".to_string());
        assert_eq!(
            key_from(Some("from-env".into()), config).as_deref(),
            Some("from-env")
        );
        // The key `scope setup` saved must be used when the variable is unset.
        assert_eq!(key_from(None, config).as_deref(), Some("from-config"));
        // An empty variable or key is no key.
        assert_eq!(
            key_from(Some("  ".into()), config).as_deref(),
            Some("from-config")
        );
        assert_eq!(key_from(None, || Some(String::new())), None);
    }
}
