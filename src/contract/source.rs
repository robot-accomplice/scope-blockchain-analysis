//! # Contract Source Code Retrieval
//!
//! Fetches verified contract source code, ABI, compiler settings, and
//! metadata from block explorer APIs (Etherscan `getsourcecode`).
//!
//! ## Etherscan Free Tier
//!
//! The `getsourcecode` endpoint is available on the free tier (5 calls/sec).
//! It returns: SourceCode, ABI, ContractName, CompilerVersion,
//! OptimizationUsed, Runs, ConstructorArguments, EVMVersion,
//! Library, LicenseType, Proxy, Implementation, SwarmSource.

use crate::error::{Result, ScopeError};
use serde::{Deserialize, Serialize};

/// Etherscan V2 API base URL (same as in ethereum.rs).
const ETHERSCAN_V2_API: &str = "https://api.etherscan.io/v2/api";

/// Full contract source information from a block explorer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractSource {
    /// Contract name as registered on the explorer.
    pub contract_name: String,
    /// Solidity (or Vyper) source code. May be a single file or JSON of multiple files.
    pub source_code: String,
    /// Contract ABI as a JSON string.
    pub abi: String,
    /// Compiler version used (e.g., "v0.8.19+commit.7dd6d404").
    pub compiler_version: String,
    /// Whether optimization was enabled.
    pub optimization_used: bool,
    /// Optimization runs count.
    pub optimization_runs: u32,
    /// EVM version targeted (e.g., "paris", "london").
    pub evm_version: String,
    /// SPDX license identifier.
    pub license_type: String,
    /// Whether Etherscan flagged this as a proxy contract.
    pub is_proxy: bool,
    /// Implementation address if proxy detected by Etherscan.
    pub implementation_address: Option<String>,
    /// Constructor arguments (hex-encoded).
    pub constructor_arguments: String,
    /// Library addresses used.
    pub library: String,
    /// Swarm/IPFS source hash.
    pub swarm_source: String,
    /// Parsed ABI entries for programmatic access.
    pub parsed_abi: Vec<AbiEntry>,
}

/// A single ABI entry (function, event, or constructor).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiEntry {
    /// Entry type: "function", "event", "constructor", "fallback", "receive".
    #[serde(rename = "type")]
    pub entry_type: String,
    /// Function/event name (empty for constructor/fallback/receive).
    #[serde(default)]
    pub name: String,
    /// Input parameters.
    #[serde(default)]
    pub inputs: Vec<AbiParam>,
    /// Output parameters (functions only).
    #[serde(default)]
    pub outputs: Vec<AbiParam>,
    /// State mutability: "pure", "view", "nonpayable", "payable".
    #[serde(default, rename = "stateMutability")]
    pub state_mutability: String,
}

/// An ABI parameter (input or output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiParam {
    /// Parameter name.
    pub name: String,
    /// Solidity type (e.g., "uint256", "address", "bytes32").
    #[serde(rename = "type")]
    pub param_type: String,
    /// Whether this is an indexed event parameter.
    #[serde(default)]
    pub indexed: bool,
    /// Tuple components (for tuple types).
    #[serde(default)]
    pub components: Vec<AbiParam>,
}

impl AbiEntry {
    /// Returns the canonical function signature (e.g., "transfer(address,uint256)").
    pub fn signature(&self) -> String {
        let params: Vec<String> = self.inputs.iter().map(|p| p.canonical_type()).collect();
        format!("{}({})", self.name, params.join(","))
    }

    /// Returns the 4-byte selector hex string for this function.
    pub fn selector(&self) -> String {
        let sig = self.signature();
        let hash = sha2_256(sig.as_bytes());
        format!("0x{}", hex::encode(&hash[..4]))
    }

    /// Whether this is a state-changing function (not view/pure).
    pub fn is_state_changing(&self) -> bool {
        self.entry_type == "function"
            && self.state_mutability != "view"
            && self.state_mutability != "pure"
    }
}

impl AbiParam {
    /// Returns the canonical Solidity type string for ABI encoding.
    fn canonical_type(&self) -> String {
        if self.param_type == "tuple" {
            let components: Vec<String> =
                self.components.iter().map(|c| c.canonical_type()).collect();
            format!("({})", components.join(","))
        } else {
            self.param_type.clone()
        }
    }
}

/// Compute SHA-256 hash (reuse sha2 crate already in deps).
fn sha2_256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Map chain name to Etherscan V2 chain ID parameter.
fn chain_to_etherscan_id(chain: &str) -> Option<&'static str> {
    match chain.to_lowercase().as_str() {
        "ethereum" | "eth" => Some("1"),
        "polygon" | "matic" => Some("137"),
        "arbitrum" | "arb" => Some("42161"),
        "optimism" | "op" => Some("10"),
        "base" => Some("8453"),
        "bsc" | "bnb" => Some("56"),
        _ => None,
    }
}

/// Etherscan getsourcecode response structures.
#[derive(Deserialize)]
struct EtherscanSourceResponse {
    status: String,
    #[allow(dead_code)]
    message: String,
    result: Vec<EtherscanSourceResult>,
}

#[derive(Deserialize)]
struct EtherscanSourceResult {
    #[serde(rename = "SourceCode")]
    source_code: String,
    #[serde(rename = "ABI")]
    abi: String,
    #[serde(rename = "ContractName")]
    contract_name: String,
    #[serde(rename = "CompilerVersion")]
    compiler_version: String,
    #[serde(rename = "OptimizationUsed")]
    optimization_used: String,
    #[serde(rename = "Runs")]
    runs: String,
    #[serde(rename = "ConstructorArguments")]
    constructor_arguments: String,
    #[serde(rename = "EVMVersion")]
    evm_version: String,
    #[serde(rename = "Library")]
    library: String,
    #[serde(rename = "LicenseType")]
    license_type: String,
    #[serde(rename = "Proxy")]
    proxy: String,
    #[serde(rename = "Implementation")]
    implementation: String,
    #[serde(rename = "SwarmSource")]
    swarm_source: String,
}

/// Fetch full contract source code and metadata from Etherscan.
///
/// Uses the `getsourcecode` endpoint which is available on the free tier.
/// Returns `Err` if the contract is not verified or the API call fails.
pub async fn fetch_contract_source(
    address: &str,
    chain: &str,
    http_client: &reqwest::Client,
) -> Result<ContractSource> {
    let chain_id = chain_to_etherscan_id(chain).ok_or_else(|| {
        ScopeError::Chain(format!(
            "Chain '{}' does not have Etherscan source code support",
            chain
        ))
    })?;

    let api_key = std::env::var("ETHERSCAN_API_KEY").unwrap_or_default();
    let url = format!(
        "{}?chainid={}&module=contract&action=getsourcecode&address={}&apikey={}",
        ETHERSCAN_V2_API, chain_id, address, api_key
    );

    let response = http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ScopeError::Api(format!("Etherscan source fetch failed: {}", e)))?;

    let text = response
        .text()
        .await
        .map_err(|e| ScopeError::Api(format!("Failed to read Etherscan response: {}", e)))?;

    let api_response: EtherscanSourceResponse = serde_json::from_str(&text)
        .map_err(|e| ScopeError::Api(format!("Failed to parse Etherscan response: {}", e)))?;

    if api_response.status != "1" || api_response.result.is_empty() {
        return Err(ScopeError::NotFound(format!(
            "Contract source not verified for {} on {}",
            address, chain
        )));
    }

    let result = &api_response.result[0];

    // Check if source is actually present (unverified contracts return empty)
    if result.source_code.is_empty() || result.abi == "Contract source code not verified" {
        return Err(ScopeError::NotFound(format!(
            "Contract {} is not verified on {} Etherscan",
            address, chain
        )));
    }

    // Parse ABI JSON into structured entries
    let parsed_abi: Vec<AbiEntry> = if result.abi.starts_with('[') {
        serde_json::from_str(&result.abi).unwrap_or_default()
    } else {
        Vec::new()
    };

    let optimization_used = result.optimization_used == "1";
    let optimization_runs: u32 = result.runs.parse().unwrap_or(200);

    let implementation_address = if !result.implementation.is_empty() {
        Some(result.implementation.clone())
    } else {
        None
    };

    Ok(ContractSource {
        contract_name: result.contract_name.clone(),
        source_code: result.source_code.clone(),
        abi: result.abi.clone(),
        compiler_version: result.compiler_version.clone(),
        optimization_used,
        optimization_runs,
        evm_version: result.evm_version.clone(),
        license_type: result.license_type.clone(),
        is_proxy: result.proxy == "1",
        implementation_address,
        constructor_arguments: result.constructor_arguments.clone(),
        library: result.library.clone(),
        swarm_source: result.swarm_source.clone(),
        parsed_abi,
    })
}

/// Extract individual source files from Etherscan's source code response.
///
/// Etherscan returns source code in different formats:
/// 1. Single file: plain Solidity source
/// 2. Multi-file: JSON object wrapped in `{{...}}` with "sources" key
/// 3. Standard JSON input: JSON with "language", "sources", "settings"
pub fn extract_source_files(source: &ContractSource) -> Vec<SourceFile> {
    let code = &source.source_code;

    // Multi-file format: starts with {{ and ends with }}
    if code.starts_with("{{") && code.ends_with("}}") {
        // Strip outer braces (Etherscan wraps standard JSON input in extra braces)
        let inner = &code[1..code.len() - 1];
        if let Ok(standard_json) = serde_json::from_str::<serde_json::Value>(inner) {
            return extract_from_standard_json(&standard_json);
        }
    }

    // Single-file JSON (standard input format)
    if code.starts_with('{')
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(code)
    {
        return extract_from_standard_json(&json);
    }

    // Single plain source file
    vec![SourceFile {
        path: format!("{}.sol", source.contract_name),
        content: code.clone(),
    }]
}

/// A source file extracted from multi-file source code.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// File path (e.g., "contracts/Token.sol").
    pub path: String,
    /// File content.
    pub content: String,
}

fn extract_from_standard_json(json: &serde_json::Value) -> Vec<SourceFile> {
    let mut files = Vec::new();

    if let Some(sources) = json.get("sources").and_then(|s| s.as_object()) {
        for (path, source) in sources {
            if let Some(content) = source.get("content").and_then(|c| c.as_str()) {
                files.push(SourceFile {
                    path: path.clone(),
                    content: content.to_string(),
                });
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_to_etherscan_id() {
        assert_eq!(chain_to_etherscan_id("ethereum"), Some("1"));
        assert_eq!(chain_to_etherscan_id("polygon"), Some("137"));
        assert_eq!(chain_to_etherscan_id("bsc"), Some("56"));
        assert_eq!(chain_to_etherscan_id("solana"), None);
    }

    #[test]
    fn test_abi_entry_signature() {
        let entry = AbiEntry {
            entry_type: "function".to_string(),
            name: "transfer".to_string(),
            inputs: vec![
                AbiParam {
                    name: "to".to_string(),
                    param_type: "address".to_string(),
                    indexed: false,
                    components: vec![],
                },
                AbiParam {
                    name: "amount".to_string(),
                    param_type: "uint256".to_string(),
                    indexed: false,
                    components: vec![],
                },
            ],
            outputs: vec![],
            state_mutability: "nonpayable".to_string(),
        };
        assert_eq!(entry.signature(), "transfer(address,uint256)");
    }

    #[test]
    fn test_abi_entry_is_state_changing() {
        let view_fn = AbiEntry {
            entry_type: "function".to_string(),
            name: "balanceOf".to_string(),
            inputs: vec![],
            outputs: vec![],
            state_mutability: "view".to_string(),
        };
        assert!(!view_fn.is_state_changing());

        let write_fn = AbiEntry {
            entry_type: "function".to_string(),
            name: "transfer".to_string(),
            inputs: vec![],
            outputs: vec![],
            state_mutability: "nonpayable".to_string(),
        };
        assert!(write_fn.is_state_changing());
    }

    #[test]
    fn test_extract_single_file_source() {
        let source = ContractSource {
            contract_name: "TestToken".to_string(),
            source_code: "pragma solidity ^0.8.0;\ncontract TestToken {}".to_string(),
            abi: "[]".to_string(),
            compiler_version: "v0.8.19".to_string(),
            optimization_used: true,
            optimization_runs: 200,
            evm_version: "paris".to_string(),
            license_type: "MIT".to_string(),
            is_proxy: false,
            implementation_address: None,
            constructor_arguments: String::new(),
            library: String::new(),
            swarm_source: String::new(),
            parsed_abi: vec![],
        };
        let files = extract_source_files(&source);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "TestToken.sol");
        assert!(files[0].content.contains("pragma solidity"));
    }

    #[test]
    fn test_extract_multi_file_source() {
        let json_source = r#"{"sources":{"contracts/Token.sol":{"content":"pragma solidity ^0.8.0;"},"contracts/Lib.sol":{"content":"library Lib {}"}}}"#;
        let wrapped = format!("{{{}}}", json_source);
        let source = ContractSource {
            contract_name: "Token".to_string(),
            source_code: wrapped,
            abi: "[]".to_string(),
            compiler_version: "v0.8.19".to_string(),
            optimization_used: true,
            optimization_runs: 200,
            evm_version: "paris".to_string(),
            license_type: "MIT".to_string(),
            is_proxy: false,
            implementation_address: None,
            constructor_arguments: String::new(),
            library: String::new(),
            swarm_source: String::new(),
            parsed_abi: vec![],
        };
        let files = extract_source_files(&source);
        assert_eq!(files.len(), 2);
    }
}
