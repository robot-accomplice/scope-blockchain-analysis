//! # Transaction Analysis Command
//!
//! This module implements the `bca tx` command for analyzing
//! blockchain transactions. It decodes transaction data, traces
//! execution, and displays detailed transaction information.
//!
//! ## Usage
//!
//! ```bash
//! # Basic transaction analysis
//! bca tx 0xabc123...
//!
//! # Specify chain
//! bca tx 0xabc123... --chain polygon
//!
//! # Include internal transactions
//! bca tx 0xabc123... --trace
//! ```

use crate::chains::{
    EthereumClient, SolanaClient, TronClient, validate_solana_signature, validate_tron_tx_hash,
};
use crate::config::{Config, OutputFormat};
use crate::error::{BccError, Result};
use clap::Args;

/// Arguments for the transaction analysis command.
#[derive(Debug, Clone, Args)]
pub struct TxArgs {
    /// The transaction hash to analyze.
    ///
    /// Must be a valid transaction hash for the target chain
    /// (e.g., 0x-prefixed 64-character hex for Ethereum).
    #[arg(value_name = "HASH")]
    pub hash: String,

    /// Target blockchain network.
    ///
    /// EVM chains: ethereum, polygon, arbitrum, optimism, base, bsc, aegis
    /// Non-EVM chains: solana, tron
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Override output format for this command.
    #[arg(short, long, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,

    /// Include internal transactions (trace).
    #[arg(long)]
    pub trace: bool,

    /// Decode transaction input data.
    #[arg(long)]
    pub decode: bool,
}

/// Result of a transaction analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionReport {
    /// The analyzed transaction hash.
    pub hash: String,

    /// The blockchain network.
    pub chain: String,

    /// Block information.
    pub block: BlockInfo,

    /// Transaction details.
    pub transaction: TransactionDetails,

    /// Gas information.
    pub gas: GasInfo,

    /// Decoded input data (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_input: Option<DecodedInput>,

    /// Internal transactions (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_transactions: Option<Vec<InternalTransaction>>,
}

/// Block information for a transaction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockInfo {
    /// Block number.
    pub number: u64,

    /// Block timestamp (Unix epoch).
    pub timestamp: u64,

    /// Block hash.
    pub hash: String,
}

/// Detailed transaction information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionDetails {
    /// Sender address.
    pub from: String,

    /// Recipient address (None for contract creation).
    pub to: Option<String>,

    /// Value transferred in native token.
    pub value: String,

    /// Transaction nonce.
    pub nonce: u64,

    /// Transaction index in block.
    pub transaction_index: u64,

    /// Transaction status (success/failure).
    pub status: bool,

    /// Raw input data.
    pub input: String,
}

/// Gas usage information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GasInfo {
    /// Gas limit set for transaction.
    pub gas_limit: u64,

    /// Actual gas used.
    pub gas_used: u64,

    /// Gas price in wei.
    pub gas_price: String,

    /// Total transaction fee.
    pub transaction_fee: String,

    /// Effective gas price (for EIP-1559).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_gas_price: Option<String>,
}

/// Decoded transaction input.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecodedInput {
    /// Function signature (e.g., "transfer(address,uint256)").
    pub function_signature: String,

    /// Function name.
    pub function_name: String,

    /// Decoded parameters.
    pub parameters: Vec<DecodedParameter>,
}

/// A decoded function parameter.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecodedParameter {
    /// Parameter name.
    pub name: String,

    /// Parameter type.
    pub param_type: String,

    /// Parameter value.
    pub value: String,
}

/// An internal transaction (trace result).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InternalTransaction {
    /// Call type (call, delegatecall, staticcall, create).
    pub call_type: String,

    /// From address.
    pub from: String,

    /// To address.
    pub to: String,

    /// Value transferred.
    pub value: String,

    /// Gas provided.
    pub gas: u64,

    /// Input data.
    pub input: String,

    /// Output data.
    pub output: String,
}

/// Executes the transaction analysis command.
///
/// # Arguments
///
/// * `args` - The parsed command arguments
/// * `config` - Application configuration
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if the analysis fails.
///
/// # Errors
///
/// Returns [`BccError::InvalidHash`] if the transaction hash is invalid.
/// Returns [`BccError::Request`] if API calls fail.
pub async fn run(mut args: TxArgs, config: &Config) -> Result<()> {
    // Auto-infer chain if using default and hash format is recognizable
    if args.chain == "ethereum"
        && let Some(inferred) = crate::chains::infer_chain_from_hash(&args.hash)
        && inferred != "ethereum"
    {
        tracing::info!("Auto-detected chain: {}", inferred);
        println!("Auto-detected chain: {}", inferred);
        args.chain = inferred.to_string();
    }

    tracing::info!(
        hash = %args.hash,
        chain = %args.chain,
        "Starting transaction analysis"
    );

    // Validate transaction hash
    validate_tx_hash(&args.hash, &args.chain)?;

    println!("Analyzing transaction on {}...", args.chain);

    let chain_lower = args.chain.to_lowercase();
    let tx = match chain_lower.as_str() {
        "solana" | "sol" => {
            let client = SolanaClient::new(&config.chains)?;
            client.get_transaction(&args.hash).await?
        }
        "tron" | "trx" => {
            let client = TronClient::new(&config.chains)?;
            client.get_transaction(&args.hash).await?
        }
        _ => {
            // EVM chains
            let client = EthereumClient::for_chain(&args.chain, &config.chains)?;
            client.get_transaction(&args.hash).await?
        }
    };

    // Calculate transaction fee
    let gas_price_val: u128 = tx.gas_price.parse().unwrap_or(0);
    let gas_used_val = tx.gas_used.unwrap_or(0) as u128;
    let fee_wei = gas_price_val * gas_used_val;
    let fee_str = if chain_lower == "solana" || chain_lower == "sol" {
        // For Solana, gas_price already contains the fee in lamports
        let fee_sol = tx.gas_price.parse::<f64>().unwrap_or(0.0) / 1_000_000_000.0;
        format!("{:.9}", fee_sol)
    } else {
        fee_wei.to_string()
    };

    let report = TransactionReport {
        hash: tx.hash.clone(),
        chain: args.chain.clone(),
        block: BlockInfo {
            number: tx.block_number.unwrap_or(0),
            timestamp: tx.timestamp.unwrap_or(0),
            hash: String::new(), // Block hash not available from tx data
        },
        transaction: TransactionDetails {
            from: tx.from.clone(),
            to: tx.to.clone(),
            value: tx.value.clone(),
            nonce: tx.nonce,
            transaction_index: 0,
            status: tx.status.unwrap_or(true),
            input: tx.input.clone(),
        },
        gas: GasInfo {
            gas_limit: tx.gas_limit,
            gas_used: tx.gas_used.unwrap_or(0),
            gas_price: tx.gas_price.clone(),
            transaction_fee: fee_str,
            effective_gas_price: None,
        },
        decoded_input: if args.decode && !tx.input.is_empty() && tx.input != "0x" {
            // Basic decode: show function selector (first 4 bytes)
            let selector = if tx.input.len() >= 10 {
                &tx.input[..10]
            } else {
                &tx.input
            };
            Some(DecodedInput {
                function_signature: format!("{}(...)", selector),
                function_name: selector.to_string(),
                parameters: vec![],
            })
        } else if args.decode {
            Some(DecodedInput {
                function_signature: "transfer()".to_string(),
                function_name: "Native Transfer".to_string(),
                parameters: vec![],
            })
        } else {
            None
        },
        internal_transactions: if args.trace { Some(vec![]) } else { None },
    };

    // Output based on format
    let format = args.format.unwrap_or(config.output.format);
    output_report(&report, format)?;

    Ok(())
}

/// Validates a transaction hash format for the given chain.
fn validate_tx_hash(hash: &str, chain: &str) -> Result<()> {
    match chain {
        // EVM-compatible chains use 0x-prefixed 64-char hex hashes
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "aegis" => {
            if !hash.starts_with("0x") {
                return Err(BccError::InvalidHash(format!(
                    "Transaction hash must start with '0x': {}",
                    hash
                )));
            }
            if hash.len() != 66 {
                return Err(BccError::InvalidHash(format!(
                    "Transaction hash must be 66 characters (0x + 64 hex): {}",
                    hash
                )));
            }
            if !hash[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(BccError::InvalidHash(format!(
                    "Transaction hash contains invalid hex characters: {}",
                    hash
                )));
            }
        }
        // Solana uses base58-encoded 64-byte signatures
        "solana" => {
            validate_solana_signature(hash)?;
        }
        // Tron uses 64-char hex hashes (no 0x prefix)
        "tron" => {
            validate_tron_tx_hash(hash)?;
        }
        _ => {
            return Err(BccError::Chain(format!(
                "Unsupported chain: {}. Supported: ethereum, polygon, arbitrum, optimism, base, bsc, aegis, solana, tron",
                chain
            )));
        }
    }
    Ok(())
}

/// Outputs the transaction report in the specified format.
fn output_report(report: &TransactionReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(report)?;
            println!("{}", json);
        }
        OutputFormat::Csv => {
            println!("hash,chain,block,from,to,value,status,gas_used,fee");
            println!(
                "{},{},{},{},{},{},{},{},{}",
                report.hash,
                report.chain,
                report.block.number,
                report.transaction.from,
                report.transaction.to.as_deref().unwrap_or(""),
                report.transaction.value,
                report.transaction.status,
                report.gas.gas_used,
                report.gas.transaction_fee
            );
        }
        OutputFormat::Table => {
            println!("Transaction Analysis Report");
            println!("===========================");
            println!("Hash:         {}", report.hash);
            println!("Chain:        {}", report.chain);
            println!("Block:        {}", report.block.number);
            println!(
                "Status:       {}",
                if report.transaction.status {
                    "Success"
                } else {
                    "Failed"
                }
            );
            println!();
            println!("From:         {}", report.transaction.from);
            println!(
                "To:           {}",
                report
                    .transaction
                    .to
                    .as_deref()
                    .unwrap_or("Contract Creation")
            );
            println!("Value:        {}", report.transaction.value);
            println!();
            println!("Gas Limit:    {}", report.gas.gas_limit);
            println!("Gas Used:     {}", report.gas.gas_used);
            println!("Gas Price:    {}", report.gas.gas_price);
            println!("Fee:          {}", report.gas.transaction_fee);

            if let Some(ref decoded) = report.decoded_input {
                println!();
                println!("Function:     {}", decoded.function_name);
                println!("Signature:    {}", decoded.function_signature);
                if !decoded.parameters.is_empty() {
                    println!("Parameters:");
                    for param in &decoded.parameters {
                        println!("  {} ({}): {}", param.name, param.param_type, param.value);
                    }
                }
            }

            if let Some(ref traces) = report.internal_transactions
                && !traces.is_empty()
            {
                println!();
                println!("Internal Transactions: {}", traces.len());
                for (i, trace) in traces.iter().enumerate() {
                    println!(
                        "  [{}] {} {} -> {}",
                        i, trace.call_type, trace.from, trace.to
                    );
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TX_HASH: &str =
        "0xabc123def456789012345678901234567890123456789012345678901234abcd";

    #[test]
    fn test_validate_tx_hash_valid() {
        let result = validate_tx_hash(VALID_TX_HASH, "ethereum");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_valid_lowercase() {
        let hash = "0xabc123def456789012345678901234567890123456789012345678901234abcd";
        let result = validate_tx_hash(hash, "ethereum");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_valid_polygon() {
        let result = validate_tx_hash(VALID_TX_HASH, "polygon");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_missing_prefix() {
        let hash = "abc123def456789012345678901234567890123456789012345678901234abcd";
        let result = validate_tx_hash(hash, "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("0x"));
    }

    #[test]
    fn test_validate_tx_hash_too_short() {
        let hash = "0xabc123";
        let result = validate_tx_hash(hash, "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("66 characters"));
    }

    #[test]
    fn test_validate_tx_hash_too_long() {
        let hash = "0xabc123def456789012345678901234567890123456789012345678901234abcde";
        let result = validate_tx_hash(hash, "ethereum");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tx_hash_invalid_hex() {
        let hash = "0xabc123def456789012345678901234567890123456789012345678901234GHIJ";
        let result = validate_tx_hash(hash, "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hex"));
    }

    #[test]
    fn test_validate_tx_hash_unsupported_chain() {
        let result = validate_tx_hash(VALID_TX_HASH, "bitcoin");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported chain")
        );
    }

    #[test]
    fn test_validate_tx_hash_valid_bsc() {
        let result = validate_tx_hash(VALID_TX_HASH, "bsc");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_valid_aegis() {
        let result = validate_tx_hash(VALID_TX_HASH, "aegis");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_valid_solana() {
        // Solana signature (base58 encoded, ~88 chars)
        let sig = "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW";
        let result = validate_tx_hash(sig, "solana");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_invalid_solana() {
        // EVM hash should fail for Solana
        let result = validate_tx_hash(VALID_TX_HASH, "solana");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tx_hash_valid_tron() {
        // Tron uses 64-char hex without 0x prefix
        let hash = "abc123def456789012345678901234567890123456789012345678901234abcd";
        let result = validate_tx_hash(hash, "tron");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_hash_invalid_tron() {
        // 0x-prefixed hash should fail for Tron
        let result = validate_tx_hash(VALID_TX_HASH, "tron");
        assert!(result.is_err());
    }

    #[test]
    fn test_tx_args_default_values() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: TxArgs,
        }

        let cli = TestCli::try_parse_from(["test", VALID_TX_HASH]).unwrap();

        assert_eq!(cli.args.chain, "ethereum");
        assert!(!cli.args.trace);
        assert!(!cli.args.decode);
        assert!(cli.args.format.is_none());
    }

    #[test]
    fn test_tx_args_with_options() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: TxArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            VALID_TX_HASH,
            "--chain",
            "polygon",
            "--trace",
            "--decode",
            "--format",
            "json",
        ])
        .unwrap();

        assert_eq!(cli.args.chain, "polygon");
        assert!(cli.args.trace);
        assert!(cli.args.decode);
        assert_eq!(cli.args.format, Some(OutputFormat::Json));
    }

    #[test]
    fn test_transaction_report_serialization() {
        let report = TransactionReport {
            hash: VALID_TX_HASH.to_string(),
            chain: "ethereum".to_string(),
            block: BlockInfo {
                number: 12345678,
                timestamp: 1700000000,
                hash: "0xblock".to_string(),
            },
            transaction: TransactionDetails {
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "1.0".to_string(),
                nonce: 42,
                transaction_index: 5,
                status: true,
                input: "0x".to_string(),
            },
            gas: GasInfo {
                gas_limit: 100000,
                gas_used: 21000,
                gas_price: "20000000000".to_string(),
                transaction_fee: "0.00042".to_string(),
                effective_gas_price: None,
            },
            decoded_input: None,
            internal_transactions: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(VALID_TX_HASH));
        assert!(json.contains("12345678"));
        assert!(json.contains("21000"));
        assert!(!json.contains("decoded_input"));
        assert!(!json.contains("internal_transactions"));
    }

    #[test]
    fn test_block_info_serialization() {
        let block = BlockInfo {
            number: 12345678,
            timestamp: 1700000000,
            hash: "0xblockhash".to_string(),
        };

        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("12345678"));
        assert!(json.contains("1700000000"));
        assert!(json.contains("0xblockhash"));
    }

    #[test]
    fn test_gas_info_serialization() {
        let gas = GasInfo {
            gas_limit: 100000,
            gas_used: 50000,
            gas_price: "20000000000".to_string(),
            transaction_fee: "0.001".to_string(),
            effective_gas_price: Some("25000000000".to_string()),
        };

        let json = serde_json::to_string(&gas).unwrap();
        assert!(json.contains("100000"));
        assert!(json.contains("50000"));
        assert!(json.contains("effective_gas_price"));
    }

    #[test]
    fn test_decoded_input_serialization() {
        let decoded = DecodedInput {
            function_signature: "transfer(address,uint256)".to_string(),
            function_name: "transfer".to_string(),
            parameters: vec![
                DecodedParameter {
                    name: "to".to_string(),
                    param_type: "address".to_string(),
                    value: "0xrecipient".to_string(),
                },
                DecodedParameter {
                    name: "amount".to_string(),
                    param_type: "uint256".to_string(),
                    value: "1000000".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&decoded).unwrap();
        assert!(json.contains("transfer(address,uint256)"));
        assert!(json.contains("0xrecipient"));
        assert!(json.contains("1000000"));
    }

    #[test]
    fn test_internal_transaction_serialization() {
        let internal = InternalTransaction {
            call_type: "call".to_string(),
            from: "0xfrom".to_string(),
            to: "0xto".to_string(),
            value: "1.0".to_string(),
            gas: 50000,
            input: "0x".to_string(),
            output: "0x".to_string(),
        };

        let json = serde_json::to_string(&internal).unwrap();
        assert!(json.contains("call"));
        assert!(json.contains("0xfrom"));
        assert!(json.contains("50000"));
    }
}
