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
            _ => anyhow::bail!(
                "Chain '{}' not yet supported for transaction fetching",
                chain
            ),
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

        let response = self.http_client.get(&url).send().await?;

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

        let response = self.http_client.get(&url).send().await?;

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

        let response = self.http_client.get(&url).send().await?;

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

        let _response = self.http_client.get(&url).send().await?;

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

    let structuring_count = amounts
        .iter()
        .filter(|&&amt| {
            // Check if amount is just under a round number (e.g., 0.99, 1.99, etc.)
            let fractional = amt.fract();
            fractional > 0.9 && fractional < 1.0
        })
        .count();

    analysis.structuring_detected = structuring_count > amounts.len() / 3;

    // Round numbers (indicates automation)
    let round_count = amounts
        .iter()
        .filter(|&&amt| amt.fract() == 0.0 || amt.fract() < 0.01)
        .count();

    analysis.round_number_pattern = round_count > amounts.len() / 2;

    // Time analysis (unusual hours)
    let unusual_hour_count = timestamps
        .iter()
        .filter(|&&ts| {
            let hour = (ts % 86400) / 3600;
            !(6..=23).contains(&hour)
        })
        .count();

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

    #[test]
    fn test_pattern_analysis_empty() {
        let analysis = analyze_patterns(&[]);
        assert_eq!(analysis.total_transactions, 0);
        assert_eq!(analysis.velocity_score, 0.0);
        assert!(!analysis.structuring_detected);
        assert!(!analysis.round_number_pattern);
        assert!(!analysis.time_clustering);
        assert_eq!(analysis.unusual_hours, 0);
    }

    #[test]
    fn test_pattern_analysis_structuring_detection() {
        // Transactions just under 1 ETH (structuring pattern)
        let txs = vec![
            create_test_tx("1609459200", "0.99"),
            create_test_tx("1609459300", "0.95"),
            create_test_tx("1609459400", "0.98"),
        ];

        let analysis = analyze_patterns(&txs);
        assert!(analysis.structuring_detected);
    }

    #[test]
    fn test_pattern_analysis_round_numbers() {
        // Round number transactions
        let txs = vec![
            create_test_tx("1609459200", "1.0"),
            create_test_tx("1609459300", "2.0"),
            create_test_tx("1609459400", "5.0"),
        ];

        let analysis = analyze_patterns(&txs);
        assert!(analysis.round_number_pattern);
    }

    #[test]
    fn test_pattern_analysis_unusual_hours() {
        // Transaction at 3 AM (unusual hour)
        // 1609459200 = 2021-01-01 00:00:00 UTC
        // 1609470000 = 2021-01-01 03:00:00 UTC
        let txs = vec![EtherscanTransaction {
            block_number: "1".to_string(),
            timestamp: "1609470000".to_string(),
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
        }];

        let analysis = analyze_patterns(&txs);
        assert!(analysis.unusual_hours > 0);
    }

    #[test]
    fn test_data_sources_creation() {
        let sources = DataSources::new("test_api_key".to_string());
        assert_eq!(sources.etherscan_key(), "test_api_key");
    }

    #[test]
    fn test_pattern_analysis_high_velocity() {
        // Many transactions spread over multiple days
        let mut txs = Vec::new();
        let base_time = 1609459200u64; // 2021-01-01 00:00:00 UTC

        for i in 0..100 {
            // Spread over 2 days (every ~30 minutes)
            txs.push(EtherscanTransaction {
                block_number: i.to_string(),
                timestamp: (base_time + i * 1800).to_string(),
                hash: format!("0x{}", i),
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
            });
        }

        let analysis = analyze_patterns(&txs);
        // 100 transactions over 2 days = 50 tx/day
        assert!(analysis.velocity_score > 40.0); // High velocity
        assert_eq!(analysis.total_transactions, 100);
    }

    #[test]
    fn test_pattern_analysis_failed_transactions() {
        let txs = vec![EtherscanTransaction {
            block_number: "1".to_string(),
            timestamp: "1609459200".to_string(),
            hash: "0x1".to_string(),
            from: "0xa".to_string(),
            to: "0xb".to_string(),
            value: "1000000000000000000".to_string(),
            gas: "21000".to_string(),
            gas_price: "20000000000".to_string(),
            is_error: "1".to_string(), // Failed transaction
            txreceipt_status: "0".to_string(),
            input: "0x".to_string(),
            contract_address: "".to_string(),
            cumulative_gas_used: "21000".to_string(),
            gas_used: "21000".to_string(),
            confirmations: "100".to_string(),
        }];

        let analysis = analyze_patterns(&txs);
        assert_eq!(analysis.total_transactions, 1);
    }

    // Helper function to create test transactions
    fn create_test_tx(timestamp: &str, value_eth: &str) -> EtherscanTransaction {
        let value_wei = (value_eth.parse::<f64>().unwrap() * 1e18) as u64;
        EtherscanTransaction {
            block_number: "1".to_string(),
            timestamp: timestamp.to_string(),
            hash: "0x1".to_string(),
            from: "0xa".to_string(),
            to: "0xb".to_string(),
            value: value_wei.to_string(),
            gas: "21000".to_string(),
            gas_price: "20000000000".to_string(),
            is_error: "0".to_string(),
            txreceipt_status: "1".to_string(),
            input: "0x".to_string(),
            contract_address: "".to_string(),
            cumulative_gas_used: "21000".to_string(),
            gas_used: "21000".to_string(),
            confirmations: "100".to_string(),
        }
    }

    #[test]
    fn test_data_sources_clone() {
        let sources = DataSources::new("key1".to_string());
        let cloned = sources.clone();
        assert_eq!(cloned.etherscan_key(), "key1");
    }

    #[test]
    fn test_data_sources_debug() {
        let sources = DataSources::new("secret_key".to_string());
        let debug = format!("{:?}", sources);
        assert!(debug.contains("DataSources"));
    }

    #[test]
    fn test_blockchain_data_client_creation() {
        let sources = DataSources::new("test_key".to_string());
        let client = BlockchainDataClient::new(sources);
        let debug = format!("{:?}", client);
        assert!(debug.contains("BlockchainDataClient"));
        assert!(debug.contains("<reqwest::Client>"));
    }

    #[test]
    fn test_etherscan_response_deserialization() {
        let json = r#"{"status":"1","message":"OK","result":null}"#;
        let response: EtherscanResponse<Vec<EtherscanTransaction>> =
            serde_json::from_str(json).unwrap();
        assert_eq!(response.status, "1");
        assert_eq!(response.message, "OK");
        assert!(response.result.is_none());
    }

    #[test]
    fn test_etherscan_response_with_result() {
        let json = r#"{"status":"1","message":"OK","result":[]}"#;
        let response: EtherscanResponse<Vec<EtherscanTransaction>> =
            serde_json::from_str(json).unwrap();
        assert_eq!(response.status, "1");
        assert!(response.result.is_some());
        assert!(response.result.unwrap().is_empty());
    }

    #[test]
    fn test_transaction_trace_creation() {
        let trace = TransactionTrace {
            root_hash: "0xabc".to_string(),
            hops: vec![
                TraceHop {
                    depth: 0,
                    address: "0x111".to_string(),
                    amount: "1.5".to_string(),
                    timestamp: 1700000000,
                },
                TraceHop {
                    depth: 1,
                    address: "0x222".to_string(),
                    amount: "0.8".to_string(),
                    timestamp: 1700003600,
                },
            ],
        };
        assert_eq!(trace.root_hash, "0xabc");
        assert_eq!(trace.hops.len(), 2);
        assert_eq!(trace.hops[0].depth, 0);
        assert_eq!(trace.hops[1].address, "0x222");
    }

    #[test]
    fn test_trace_hop_debug_and_clone() {
        let hop = TraceHop {
            depth: 2,
            address: "0x333".to_string(),
            amount: "3.14".to_string(),
            timestamp: 1700000000,
        };
        let cloned = hop.clone();
        assert_eq!(cloned.depth, 2);
        let debug = format!("{:?}", hop);
        assert!(debug.contains("TraceHop"));
    }

    #[test]
    fn test_pattern_analysis_debug_and_clone() {
        let analysis = analyze_patterns(&[]);
        let cloned = analysis.clone();
        assert_eq!(cloned.total_transactions, 0);
        let debug = format!("{:?}", analysis);
        assert!(debug.contains("PatternAnalysis"));
    }

    #[test]
    fn test_pattern_analysis_mixed_patterns() {
        // Mix of round numbers and non-round: 2 out of 5 rounds = not over half
        let txs = vec![
            create_test_tx("1609459200", "1.0"),   // round
            create_test_tx("1609459300", "2.345"),  // not round
            create_test_tx("1609459400", "5.0"),    // round
            create_test_tx("1609459500", "0.123"),  // not round
            create_test_tx("1609459600", "7.891"),  // not round
        ];
        let analysis = analyze_patterns(&txs);
        assert!(!analysis.round_number_pattern); // 2 out of 5 is not > half
    }

    #[test]
    fn test_pattern_analysis_all_unusual_hours() {
        // All transactions at 2 AM UTC (timestamp % 86400 / 3600 = 2)
        // 1609459200 = 2021-01-01 00:00:00 UTC
        // + 7200 = 2021-01-01 02:00:00 UTC
        let txs = vec![
            create_test_tx("1609466400", "1.0"), // 02:00 UTC
            create_test_tx("1609470000", "2.0"), // 03:00 UTC
            create_test_tx("1609473600", "3.0"), // 04:00 UTC
        ];
        let analysis = analyze_patterns(&txs);
        assert_eq!(analysis.unusual_hours, 3);
    }

    #[test]
    fn test_etherscan_transaction_clone_and_debug() {
        let tx = create_test_tx("1609459200", "1.0");
        let cloned = tx.clone();
        assert_eq!(cloned.hash, "0x1");
        let debug = format!("{:?}", tx);
        assert!(debug.contains("EtherscanTransaction"));
    }
}
