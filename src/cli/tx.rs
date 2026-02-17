//! # Transaction Analysis Command
//!
//! This module implements the `scope tx` command for analyzing
//! blockchain transactions. It decodes transaction data, traces
//! execution, and displays detailed transaction information.
//!
//! ## Usage
//!
//! ```bash
//! # Basic transaction analysis
//! scope tx 0xabc123...
//!
//! # Specify chain
//! scope tx 0xabc123... --chain polygon
//!
//! # Include internal transactions
//! scope tx 0xabc123... --trace
//! ```

use crate::chains::{ChainClientFactory, validate_solana_signature, validate_tron_tx_hash};
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScopeError};
use clap::Args;

/// Arguments for the transaction analysis command.
#[derive(Debug, Clone, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope tx 0xabc123def456...
  scope tx 0xabc123... --chain polygon --trace
  scope tx 0xabc123... --decode --format json

\x1b[2mTip: All address/token inputs accept @label shortcuts from the address book.\x1b[0m")]
pub struct TxArgs {
    /// The transaction hash to analyze.
    ///
    /// Must be a valid transaction hash for the target chain
    /// (e.g., 0x-prefixed 64-character hex for Ethereum).
    #[arg(value_name = "HASH")]
    pub hash: String,

    /// Target blockchain network.
    ///
    /// EVM chains: ethereum, polygon, arbitrum, optimism, base, bsc
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
/// Returns [`ScopeError::InvalidHash`] if the transaction hash is invalid.
/// Returns [`ScopeError::Request`] if API calls fail.
pub async fn run(
    mut args: TxArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
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

    let sp =
        crate::cli::progress::Spinner::new(&format!("Analyzing transaction on {}...", args.chain));

    let report =
        fetch_transaction_report(&args.hash, &args.chain, args.decode, args.trace, clients).await?;

    sp.finish("Transaction loaded.");

    // Output based on format
    let format = args.format.unwrap_or(config.output.format);
    output_report(&report, format)?;

    Ok(())
}

/// Fetches and builds a transaction report for programmatic use.
///
/// Used by the insights command and batch reporting.
pub async fn fetch_transaction_report(
    hash: &str,
    chain: &str,
    decode: bool,
    trace: bool,
    clients: &dyn ChainClientFactory,
) -> Result<TransactionReport> {
    validate_tx_hash(hash, chain)?;
    let client = clients.create_chain_client(chain)?;
    let tx = client.get_transaction(hash).await?;

    let gas_price_val: u128 = tx.gas_price.parse().unwrap_or(0);
    let gas_used_val = tx.gas_used.unwrap_or(0) as u128;
    let fee_wei = gas_price_val * gas_used_val;
    let chain_lower = chain.to_lowercase();
    let fee_str = if chain_lower == "solana" || chain_lower == "sol" {
        let fee_sol = tx.gas_price.parse::<f64>().unwrap_or(0.0) / 1_000_000_000.0;
        format!("{:.9}", fee_sol)
    } else {
        fee_wei.to_string()
    };

    let report = TransactionReport {
        hash: tx.hash.clone(),
        chain: chain.to_string(),
        block: BlockInfo {
            number: tx.block_number.unwrap_or(0),
            timestamp: tx.timestamp.unwrap_or(0),
            hash: String::new(),
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
        decoded_input: if decode && !tx.input.is_empty() && tx.input != "0x" {
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
        } else if decode {
            Some(DecodedInput {
                function_signature: "transfer()".to_string(),
                function_name: "Native Transfer".to_string(),
                parameters: vec![],
            })
        } else {
            None
        },
        internal_transactions: if trace { Some(vec![]) } else { None },
    };
    Ok(report)
}

/// Validates a transaction hash format for the given chain.
fn validate_tx_hash(hash: &str, chain: &str) -> Result<()> {
    match chain {
        // EVM-compatible chains use 0x-prefixed 64-char hex hashes
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "aegis" => {
            if !hash.starts_with("0x") {
                return Err(ScopeError::InvalidHash(format!(
                    "Transaction hash must start with '0x': {}",
                    hash
                )));
            }
            if hash.len() != 66 {
                return Err(ScopeError::InvalidHash(format!(
                    "Transaction hash must be 66 characters (0x + 64 hex): {}",
                    hash
                )));
            }
            if !hash[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ScopeError::InvalidHash(format!(
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
            return Err(ScopeError::Chain(format!(
                "Unsupported chain: {}. Supported: ethereum, polygon, arbitrum, optimism, base, bsc, solana, tron",
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
        OutputFormat::Markdown => {
            let md = format_tx_markdown(report);
            println!("{}", md);
        }
    }
    Ok(())
}

/// Formats a transaction report as markdown for agent consumption.
/// Exposed for use by insights and report generation.
pub fn format_tx_markdown(report: &TransactionReport) -> String {
    let mut md = String::new();
    md.push_str("# Transaction Analysis\n\n");
    md.push_str("| Field | Value |\n|-------|-------|\n");
    md.push_str(&format!("| Hash | `{}` |\n", report.hash));
    md.push_str(&format!("| Chain | {} |\n", report.chain));
    md.push_str(&format!("| Block | {} |\n", report.block.number));
    md.push_str(&format!(
        "| Status | {} |\n",
        if report.transaction.status {
            "Success"
        } else {
            "Failed"
        }
    ));
    md.push_str(&format!("| From | `{}` |\n", report.transaction.from));
    md.push_str(&format!(
        "| To | `{}` |\n",
        report
            .transaction
            .to
            .as_deref()
            .unwrap_or("Contract Creation")
    ));
    md.push_str(&format!("| Value | {} |\n", report.transaction.value));
    md.push_str(&format!("| Gas Used | {} |\n", report.gas.gas_used));
    md.push_str(&format!("| Fee | {} |\n", report.gas.transaction_fee));
    if let Some(ref decoded) = report.decoded_input {
        md.push_str("\n## Decoded Input\n\n");
        md.push_str(&format!("- **Function:** {}\n", decoded.function_name));
        md.push_str(&format!(
            "- **Signature:** `{}`\n",
            decoded.function_signature
        ));
        if !decoded.parameters.is_empty() {
            md.push_str("\n| Parameter | Type | Value |\n|-----------|------|-------|\n");
            for param in &decoded.parameters {
                md.push_str(&format!(
                    "| {} | {} | {} |\n",
                    param.name, param.param_type, param.value
                ));
            }
        }
    }
    if let Some(ref traces) = report.internal_transactions
        && !traces.is_empty()
    {
        md.push_str("\n## Internal Transactions\n\n");
        md.push_str("| # | Type | From | To |\n|---|---|---|---|\n");
        for (i, trace) in traces.iter().enumerate() {
            md.push_str(&format!(
                "| {} | {} | `{}` | `{}` |\n",
                i + 1,
                trace.call_type,
                trace.from,
                trace.to
            ));
        }
    }
    md
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
    fn test_validate_tx_hash_invalid_hex_cli() {
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

    // ========================================================================
    // Output formatting tests
    // ========================================================================

    fn make_test_tx_report() -> TransactionReport {
        TransactionReport {
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
                input: "0xa9059cbb0000000000".to_string(),
            },
            gas: GasInfo {
                gas_limit: 100000,
                gas_used: 21000,
                gas_price: "20000000000".to_string(),
                transaction_fee: "0.00042".to_string(),
                effective_gas_price: None,
            },
            decoded_input: Some(DecodedInput {
                function_signature: "transfer(address,uint256)".to_string(),
                function_name: "transfer".to_string(),
                parameters: vec![DecodedParameter {
                    name: "to".to_string(),
                    param_type: "address".to_string(),
                    value: "0xrecipient".to_string(),
                }],
            }),
            internal_transactions: Some(vec![InternalTransaction {
                call_type: "call".to_string(),
                from: "0xfrom".to_string(),
                to: "0xto".to_string(),
                value: "0.5".to_string(),
                gas: 30000,
                input: "0x".to_string(),
                output: "0x".to_string(),
            }]),
        }
    }

    #[test]
    fn test_output_report_json() {
        let report = make_test_tx_report();
        let result = output_report(&report, OutputFormat::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_csv() {
        let report = make_test_tx_report();
        let result = output_report(&report, OutputFormat::Csv);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table() {
        let report = make_test_tx_report();
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_no_decoded() {
        let mut report = make_test_tx_report();
        report.decoded_input = None;
        report.internal_transactions = None;
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_failed_tx() {
        let mut report = make_test_tx_report();
        report.transaction.status = false;
        report.transaction.to = None; // Contract creation
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_empty_traces() {
        let mut report = make_test_tx_report();
        report.internal_transactions = Some(vec![]);
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_csv_no_to() {
        let mut report = make_test_tx_report();
        report.transaction.to = None;
        let result = output_report(&report, OutputFormat::Csv);
        assert!(result.is_ok());
    }

    // ========================================================================
    // Mock-based tests for fetch_transaction_report
    // ========================================================================

    use crate::chains::{
        Balance as ChainBalance, ChainClient, ChainClientFactory, DexDataSource,
        TokenBalance as ChainTokenBalance, Transaction as ChainTransaction,
    };
    use async_trait::async_trait;

    struct MockTxClient;

    #[async_trait]
    impl ChainClient for MockTxClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _a: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "0".into(),
                formatted: "0 ETH".into(),
                decimals: 18,
                symbol: "ETH".into(),
                usd_value: None,
            })
        }
        async fn enrich_balance_usd(&self, _b: &mut ChainBalance) {}
        async fn get_transaction(&self, _h: &str) -> crate::error::Result<ChainTransaction> {
            Ok(ChainTransaction {
                hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd".into(),
                block_number: Some(12345678),
                timestamp: Some(1700000000),
                from: "0xfrom".into(),
                to: Some("0xto".into()),
                value: "1000000000000000000".into(),
                gas_limit: 21000,
                gas_used: Some(21000),
                gas_price: "20000000000".into(),
                nonce: 42,
                input: "0xa9059cbb0000000000000000000000001234".into(),
                status: Some(true),
            })
        }
        async fn get_transactions(
            &self,
            _a: &str,
            _l: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _a: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _addr: &str) -> crate::error::Result<String> {
            Ok("0x".into())
        }
    }

    struct MockTxFactory;
    impl ChainClientFactory for MockTxFactory {
        fn create_chain_client(&self, _chain: &str) -> crate::error::Result<Box<dyn ChainClient>> {
            Ok(Box::new(MockTxClient))
        }
        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            crate::chains::DefaultClientFactory {
                chains_config: Default::default(),
            }
            .create_dex_client()
        }
    }

    #[tokio::test]
    async fn test_fetch_transaction_report_mock() {
        let factory = MockTxFactory;
        let result = fetch_transaction_report(
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
            "ethereum",
            false,
            false,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        let report = result.unwrap();
        assert_eq!(report.transaction.from, "0xfrom");
        assert!(report.transaction.status);
    }

    #[tokio::test]
    async fn test_fetch_transaction_report_with_decode() {
        let factory = MockTxFactory;
        let result = fetch_transaction_report(
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
            "ethereum",
            true,
            false,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // End-to-end tests using MockClientFactory
    // ========================================================================

    use crate::chains::mocks::MockClientFactory;

    fn mock_factory() -> MockClientFactory {
        MockClientFactory::new()
    }

    #[tokio::test]
    async fn test_run_ethereum_tx() {
        let config = Config::default();
        let factory = mock_factory();
        let args = TxArgs {
            hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Json),
            trace: false,
            decode: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_tx_with_decode() {
        let config = Config::default();
        let mut factory = mock_factory();
        factory.mock_client.transaction.input = "0xa9059cbb000000000000000000000000".to_string();
        let args = TxArgs {
            hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Table),
            trace: false,
            decode: true,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_tx_with_trace() {
        let config = Config::default();
        let factory = mock_factory();
        let args = TxArgs {
            hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Csv),
            trace: true,
            decode: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_tx_invalid_hash() {
        let config = Config::default();
        let factory = mock_factory();
        let args = TxArgs {
            hash: "invalid".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Json),
            trace: false,
            decode: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_tx_auto_detect_tron() {
        let config = Config::default();
        let factory = mock_factory();
        let args = TxArgs {
            // 64 hex chars = tron
            hash: "abc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: "ethereum".to_string(), // Will be auto-detected to tron
            format: Some(OutputFormat::Json),
            trace: false,
            decode: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Markdown formatting tests
    // ========================================================================

    #[test]
    fn test_format_tx_markdown_basic() {
        let report = make_test_tx_report();
        let md = format_tx_markdown(&report);
        assert!(md.contains("# Transaction Analysis"));
        assert!(md.contains(&report.hash));
        assert!(md.contains(&report.chain));
        assert!(md.contains("Success"));
        assert!(md.contains(&report.transaction.from));
    }

    #[test]
    fn test_format_tx_markdown_contract_creation() {
        let mut report = make_test_tx_report();
        report.transaction.to = None;
        let md = format_tx_markdown(&report);
        assert!(md.contains("Contract Creation"));
    }

    #[test]
    fn test_format_tx_markdown_failed_tx() {
        let mut report = make_test_tx_report();
        report.transaction.status = false;
        let md = format_tx_markdown(&report);
        assert!(md.contains("Failed"));
    }

    #[test]
    fn test_format_tx_markdown_with_decoded_input() {
        let report = make_test_tx_report();
        let md = format_tx_markdown(&report);
        assert!(md.contains("## Decoded Input"));
        assert!(md.contains("transfer"));
        assert!(md.contains("transfer(address,uint256)"));
    }

    #[test]
    fn test_format_tx_markdown_with_internal_transactions() {
        let report = make_test_tx_report();
        let md = format_tx_markdown(&report);
        assert!(md.contains("## Internal Transactions"));
        assert!(md.contains("call"));
    }

    #[test]
    fn test_format_tx_markdown_no_decoded_input() {
        let mut report = make_test_tx_report();
        report.decoded_input = None;
        let md = format_tx_markdown(&report);
        assert!(!md.contains("## Decoded Input"));
    }

    #[test]
    fn test_format_tx_markdown_no_internal_transactions() {
        let mut report = make_test_tx_report();
        report.internal_transactions = None;
        let md = format_tx_markdown(&report);
        assert!(!md.contains("## Internal Transactions"));
    }

    #[test]
    fn test_format_tx_markdown_empty_internal_transactions() {
        let mut report = make_test_tx_report();
        report.internal_transactions = Some(vec![]);
        let md = format_tx_markdown(&report);
        assert!(!md.contains("## Internal Transactions"));
    }
}
