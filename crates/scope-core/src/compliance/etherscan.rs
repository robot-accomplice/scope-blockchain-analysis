//! # Etherscan access for compliance
//!
//! The one place that knows where the Etherscan API key comes from, and
//! the one way to build a risk engine with or without Etherscan data.

use super::datasource::BlockchainDataClient;
use super::risk::RiskEngine;

/// The environment variable that holds the Etherscan API key.
pub const ETHERSCAN_KEY_ENV: &str = "ETHERSCAN_API_KEY";

/// The Etherscan API key from [`ETHERSCAN_KEY_ENV`], when it is set.
pub fn etherscan_key() -> Option<String> {
    std::env::var(ETHERSCAN_KEY_ENV).ok()
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
    pub fn from_env() -> Self {
        Self::with_optional_client(BlockchainDataClient::from_env_opt())
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
        let client = BlockchainDataClient::new(DataSources::new("k".to_string()));
        let _rich = RiskEngine::with_optional_client(Some(client));
        assert_eq!(ETHERSCAN_KEY_ENV, "ETHERSCAN_API_KEY");
    }
}
