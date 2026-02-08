//! Data sources for compliance analysis
//!
//! Integrates with external APIs to fetch real blockchain data for risk analysis.

use serde::Deserialize;

/// Configuration for external data sources
#[derive(Debug, Clone)]
pub struct DataSources {
    etherscan_api_key: String,
}

impl DataSources {
    /// Create from config (users supply their own API keys)
    pub fn new(etherscan_api_key: String) -> Self {
        Self { etherscan_api_key }
    }

    /// Get Etherscan API key
    pub fn etherscan_key(&self) -> &str {
        &self.etherscan_api_key
    }
}

/// Transaction data from Etherscan
#[derive(Debug, Clone, Deserialize)]
pub struct EtherscanTransaction {
    pub block_number: String,
    pub timestamp: String,
    pub hash: String,
    pub from: String,
    pub to: String,
    pub value: String,
    pub gas: String,
    pub gas_price: String,
    pub is_error: String,
    pub txreceipt_status: String,
    pub input: String,
    pub contract_address: String,
    pub cumulative_gas_used: String,
    pub gas_used: String,
    pub confirmations: String,
}

/// Response from Etherscan API
#[derive(Debug, Clone, Deserialize)]
pub struct EtherscanResponse<T> {
    pub status: String,
    pub message: String,
    pub result: Option<T>,
}

/// Client for fetching blockchain data
pub struct BlockchainDataClient {
    sources: DataSources,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for BlockchainDataClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockchainDataClient")
            .field("sources", &self.sources)
            .field("http_client", &"<reqwest::Client>")
            .finish()
    }
}

impl BlockchainDataClient {
    /// Create new client with data sources
    pub fn new(sources: DataSources) -> Self {
        Self {
            sources,
            http_client: reqwest::Client::new(),
        }
    }

    /// Fetch transaction history for an address
    pub async fn get_transactions(
        &self,
        address: &str,
        chain: &str,
    ) -> anyhow::Result<Vec<EtherscanTransaction>> {
        match chain {
            "ethereum" | "mainnet" => self.get_etherscan_transactions(address).await,
            _ => anyhow::bail!("Chain '{}' not yet supported for transaction fetching", chain),
        }
    }

    /// Fetch from Etherscan API
    async fn get_etherscan_transactions(
        &self,
        address: &str,
    ) -> anyhow::Result<Vec<EtherscanTransaction>> {
        let url = format!(
            "https://api.etherscan.io/api?module=account&action=txlist&address={}&startblock=0&endblock=99999999&sort=asc&apikey={}",
            address,
            self.sources.etherscan_key()
        );

        let response = self.http_client
            .get(&url)
            .send()
            .await?;

        let data: EtherscanResponse<Vec<EtherscanTransaction>> = response.json().await?;

        if data.status != "1" {
            anyhow::bail!("Etherscan API error: {}", data.message);
        }

        Ok(data.result.unwrap_or_default())
    }

    /// Get internal transactions (contract calls)
    pub async fn get_internal_transactions(
        &self,
        address: &str,
    ) -> anyhow::Result<Vec<EtherscanTransaction>> {
        let url = format!(
            "https://api.etherscan.io/api?module=account&action=txlistinternal&address={}&startblock=0&endblock=99999999&sort=asc&apikey={}",
            address,
            self.sources.etherscan_key()
        );

        let response = self.http_client
            .get(&url)
            .send()
            .await?;

        let data: EtherscanResponse<Vec<EtherscanTransaction>> = response.json().await?;

        Ok(data.result.unwrap_or_default())
    }

    /// Get ERC-20 token transfers
    pub async fn get_token_transfers(
        &self,
        address: &str,
    ) -> anyhow::Result<Vec<EtherscanTransaction>> {
        let url = format!(
            "https://api.etherscan.io/api?module=account&action=tokentx&address={}&startblock=0&endblock=99999999&sort=asc&apikey={}",
            address,
            self.sources.etherscan_key()
        );

        let response = self.http_client
            .get(&url)
            .send()
            .await?;

        let data: EtherscanResponse<Vec<EtherscanTransaction>> = response.json().await?;

        Ok(data.result.unwrap_or_default())
    }

    /// Trace transaction by following outputs
    pub async fn trace_transaction(
        &self,
        tx_hash: &str,
        _depth: u32,
    ) -> anyhow::Result<TransactionTrace> {
        let mut trace = TransactionTrace {
            root_hash: tx_hash.to_string(),
            hops: Vec::new(),
        };

        // Get the transaction
        let url = format!(
            "https://api.etherscan.io/api?module=proxy&action=eth_getTransactionByHash&txhash={}&apikey={}",
            tx_hash,
            self.sources.etherscan_key()
        );

        let _response = self.http_client
            .get(&url)
            .send()
            .await?;

        // For now, return basic trace
        // Full implementation would recursively follow outputs
        trace.hops.push(TraceHop {
            depth: 0,
            address: "unknown".to_string(),
            amount: "0".to_string(),
            timestamp: 0,
        });

        Ok(trace)
    }
}

/// Transaction trace result
#[derive(Debug, Clone)]
pub struct TransactionTrace {
    pub root_hash: String,
    pub hops: Vec<TraceHop>,
}

/// Individual hop in transaction trace
#[derive(Debug, Clone)]
pub struct TraceHop {
    pub depth: u32,
    pub address: String,
    pub amount: String,
    pub timestamp: u64,
}

/// Analyze transaction patterns
pub fn analyze_patterns(transactions: &[EtherscanTransaction]) -> PatternAnalysis {
    let mut analysis = PatternAnalysis {
        total_transactions: transactions.len(),
        velocity_score: 0.0,
        structuring_detected: false,
        round_number_pattern: false,
        time_clustering: false,
        unusual_hours: 0,
    };

    if transactions.is_empty() {
        return analysis;
    }

    // Velocity: transactions per day
    let timestamps: Vec<u64> = transactions
        .iter()
        .filter_map(|tx| tx.timestamp.parse::<u64>().ok())
        .collect();
    
    if timestamps.len() >= 2 {
        let min_ts = *timestamps.iter().min().unwrap();
        let max_ts = *timestamps.iter().max().unwrap();
        let days = (max_ts - min_ts) / 86400;
        if days > 0 {
            analysis.velocity_score = transactions.len() as f32 / days as f32;
        }
    }

    // Structuring: amounts just under round numbers
    let amounts: Vec<f64> = transactions
        .iter()
        .filter_map(|tx| {
            let value_eth = tx.value.parse::<f64>().ok()? / 1e18;
            Some(value_eth)
        })
        .collect();

    let structuring_count = amounts.iter().filter(|&&amt| {
        // Check if amount is just under a round number (e.g., 0.99, 1.99, etc.)
        let fractional = amt.fract();
        fractional > 0.9 && fractional < 1.0
    }).count();

    analysis.structuring_detected = structuring_count > amounts.len() / 3;

    // Round numbers (indicates automation)
    let round_count = amounts.iter().filter(|&&amt| {
        amt.fract() == 0.0 || amt.fract() < 0.01
    }).count();

    analysis.round_number_pattern = round_count > amounts.len() / 2;

    // Time analysis (unusual hours)
    let unusual_hour_count = timestamps.iter().filter(|&&ts| {
        let hour = (ts % 86400) / 3600;
        hour < 6 || hour > 23
    }).count();

    analysis.unusual_hours = unusual_hour_count;

    analysis
}

/// Pattern analysis results
#[derive(Debug, Clone)]
pub struct PatternAnalysis {
    pub total_transactions: usize,
    pub velocity_score: f32,
    pub structuring_detected: bool,
    pub round_number_pattern: bool,
    pub time_clustering: bool,
    pub unusual_hours: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_analysis_empty() {
        let analysis = analyze_patterns(&[]);
        assert_eq!(analysis.total_transactions, 0);
        assert!(!analysis.structuring_detected);
    }

    #[test]
    fn test_pattern_analysis_velocity() {
        let txs = vec![
            EtherscanTransaction {
                block_number: "1".to_string(),
                timestamp: "1609459200".to_string(), // 2021-01-01
                hash: "0x1".to_string(),
                from: "0xa".to_string(),
                to: "0xb".to_string(),
                value: "1000000000000000000".to_string(),
                gas: "21000".to_string(),
                gas_price: "20000000000".to_string(),
                is_error: "0".to_string(),
                txreceipt_status: "1".to_string(),
                input: "0x".to_string(),
                contract_address: "".to_string(),
                cumulative_gas_used: "21000".to_string(),
                gas_used: "21000".to_string(),
                confirmations: "100".to_string(),
            },
            EtherscanTransaction {
                block_number: "2".to_string(),
                timestamp: "1609545600".to_string(), // 2021-01-02
                hash: "0x2".to_string(),
                from: "0xb".to_string(),
                to: "0xc".to_string(),
                value: "2000000000000000000".to_string(),
                gas: "21000".to_string(),
                gas_price: "20000000000".to_string(),
                is_error: "0".to_string(),
                txreceipt_status: "1".to_string(),
                input: "0x".to_string(),
                contract_address: "".to_string(),
                cumulative_gas_used: "21000".to_string(),
                gas_used: "21000".to_string(),
                confirmations: "100".to_string(),
            },
        ];

        let analysis = analyze_patterns(&txs);
        assert_eq!(analysis.total_transactions, 2);
        assert_eq!(analysis.velocity_score, 2.0); // 2 transactions over 1 day
    }
}
