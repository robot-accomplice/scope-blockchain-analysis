//! # ABI Parsing and Function Signature Decoding
//!
//! Provides function selector lookup via 4byte.directory and ABI-based
//! parameter decoding for transaction calldata.
//!
//! ## Function Signature Resolution
//!
//! 1. If contract ABI is available (verified source), decode directly from ABI
//! 2. Otherwise, look up the 4-byte selector on 4byte.directory (free, no API key)
//! 3. Fallback: display raw selector hex
//!
//! ## Calldata Decoding
//!
//! For verified contracts with ABI, decodes full calldata parameters using
//! ABI type information. For unverified contracts, only the function name
//! is resolved from the selector.

use crate::contract::source::{AbiEntry, ContractSource};
use serde::Deserialize;

/// A decoded function call from transaction input data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecodedCall {
    /// 4-byte selector (e.g., "0xa9059cbb").
    pub selector: String,
    /// Function signature (e.g., "transfer(address,uint256)").
    pub signature: String,
    /// Human-readable function name (e.g., "transfer").
    pub function_name: String,
    /// Decoded parameters (if ABI available).
    pub parameters: Vec<DecodedParam>,
    /// Whether the decoding came from verified ABI or 4byte.directory.
    pub source: DecodeSource,
}

/// Source of the function signature resolution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DecodeSource {
    /// Decoded from contract's verified ABI.
    VerifiedAbi,
    /// Looked up from 4byte.directory.
    FourByteDirectory,
    /// Only raw selector available.
    Unknown,
}

/// A decoded parameter from calldata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecodedParam {
    /// Parameter name (from ABI, or "arg0", "arg1" if unknown).
    pub name: String,
    /// Solidity type.
    pub param_type: String,
    /// Decoded value as string.
    pub value: String,
}

/// 4byte.directory API response.
#[derive(Deserialize)]
struct FourByteResponse {
    results: Vec<FourByteResult>,
}

#[derive(Deserialize)]
struct FourByteResult {
    text_signature: String,
}

/// OpenChain (Samczsun) signature lookup response.
#[derive(Deserialize)]
struct OpenChainResponse {
    ok: bool,
    result: Option<OpenChainResult>,
}

#[derive(Deserialize)]
struct OpenChainResult {
    function: Option<std::collections::HashMap<String, Vec<OpenChainEntry>>>,
}

#[derive(Deserialize)]
struct OpenChainEntry {
    name: String,
}

/// Decode transaction input data using available sources.
///
/// Resolution order:
/// 1. ABI from verified source (most accurate)
/// 2. OpenChain/Samczsun signature database
/// 3. 4byte.directory
/// 4. Raw selector fallback
pub async fn decode_calldata(
    input: &str,
    contract_source: Option<&ContractSource>,
    http_client: &reqwest::Client,
) -> DecodedCall {
    if input.is_empty() || input == "0x" {
        return DecodedCall {
            selector: "0x".to_string(),
            signature: "()".to_string(),
            function_name: "Native Transfer".to_string(),
            parameters: vec![],
            source: DecodeSource::Unknown,
        };
    }

    let selector = if input.len() >= 10 {
        &input[..10]
    } else {
        input
    };

    // Try ABI-based decoding first
    if let Some(source) = contract_source
        && let Some(entry) = find_abi_by_selector(selector, &source.parsed_abi)
    {
        let params = decode_params_from_abi(input, entry);
        return DecodedCall {
            selector: selector.to_string(),
            signature: entry.signature(),
            function_name: entry.name.clone(),
            parameters: params,
            source: DecodeSource::VerifiedAbi,
        };
    }

    // Try OpenChain (Samczsun) lookup
    if let Some(sig) = lookup_openchain(selector, http_client).await {
        let fn_name = sig.split('(').next().unwrap_or(&sig).to_string();
        return DecodedCall {
            selector: selector.to_string(),
            signature: sig,
            function_name: fn_name,
            parameters: vec![],
            source: DecodeSource::FourByteDirectory,
        };
    }

    // Try 4byte.directory lookup
    if let Some(sig) = lookup_4byte(selector, http_client).await {
        let fn_name = sig.split('(').next().unwrap_or(&sig).to_string();
        return DecodedCall {
            selector: selector.to_string(),
            signature: sig,
            function_name: fn_name,
            parameters: vec![],
            source: DecodeSource::FourByteDirectory,
        };
    }

    // Fallback: raw selector
    DecodedCall {
        selector: selector.to_string(),
        signature: format!("{}(...)", selector),
        function_name: selector.to_string(),
        parameters: vec![],
        source: DecodeSource::Unknown,
    }
}

/// Look up a function signature on OpenChain (Samczsun's database).
async fn lookup_openchain(selector: &str, http_client: &reqwest::Client) -> Option<String> {
    let url = format!(
        "https://api.openchain.xyz/signature-database/v1/lookup?function={}&filter=true",
        selector
    );

    let response = http_client.get(&url).send().await.ok()?;
    let body: OpenChainResponse = response.json().await.ok()?;

    if !body.ok {
        return None;
    }

    body.result
        .and_then(|r| r.function)
        .and_then(|mut f| f.remove(selector))
        .and_then(|entries| entries.into_iter().next())
        .map(|e| e.name)
}

/// Look up a function signature on 4byte.directory.
async fn lookup_4byte(selector: &str, http_client: &reqwest::Client) -> Option<String> {
    let hex = selector.trim_start_matches("0x");
    let url = format!(
        "https://www.4byte.directory/api/v1/signatures/?hex_signature=0x{}",
        hex
    );

    let response = http_client.get(&url).send().await.ok()?;
    let body: FourByteResponse = response.json().await.ok()?;
    body.results.into_iter().next().map(|r| r.text_signature)
}

/// Find an ABI entry matching a 4-byte selector.
fn find_abi_by_selector<'a>(selector: &str, abi: &'a [AbiEntry]) -> Option<&'a AbiEntry> {
    let selector_lower = selector.to_lowercase();
    abi.iter().find(|entry| {
        entry.entry_type == "function" && entry.selector().to_lowercase() == selector_lower
    })
}

/// Decode calldata parameters using ABI type information.
///
/// This performs basic hex-to-value decoding for common Solidity types.
/// For complex types (dynamic arrays, nested tuples), returns hex representation.
fn decode_params_from_abi(input: &str, entry: &AbiEntry) -> Vec<DecodedParam> {
    let data = if input.len() > 10 { &input[10..] } else { "" };

    entry
        .inputs
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let offset = i * 64;
            let raw_value = if offset + 64 <= data.len() {
                &data[offset..offset + 64]
            } else if offset < data.len() {
                &data[offset..]
            } else {
                ""
            };

            let value = decode_abi_value(raw_value, &param.param_type);

            DecodedParam {
                name: if param.name.is_empty() {
                    format!("arg{}", i)
                } else {
                    param.name.clone()
                },
                param_type: param.param_type.clone(),
                value,
            }
        })
        .collect()
}

/// Decode a single ABI-encoded value from hex.
fn decode_abi_value(hex_value: &str, solidity_type: &str) -> String {
    if hex_value.is_empty() {
        return "(empty)".to_string();
    }

    match solidity_type {
        "address" => {
            // Address is right-padded in 32 bytes, take last 40 chars
            let addr = if hex_value.len() >= 40 {
                &hex_value[hex_value.len() - 40..]
            } else {
                hex_value
            };
            format!("0x{}", addr)
        }
        t if t.starts_with("uint") || t.starts_with("int") => {
            // Parse as big number (show decimal for reasonable values)
            let trimmed = hex_value.trim_start_matches('0');
            if trimmed.is_empty() {
                "0".to_string()
            } else if trimmed.len() <= 16 {
                // Fits in u64
                u64::from_str_radix(trimmed, 16)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| format!("0x{}", hex_value))
            } else {
                format!("0x{}", trimmed)
            }
        }
        "bool" => {
            let last_char = hex_value.chars().last().unwrap_or('0');
            if last_char == '1' {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        t if t.starts_with("bytes") && !t.contains('[') => {
            // Fixed-size bytes
            let n: usize = t.trim_start_matches("bytes").parse().unwrap_or(32);
            let hex_chars = n * 2;
            let bytes_hex = if hex_value.len() >= hex_chars {
                &hex_value[..hex_chars]
            } else {
                hex_value
            };
            format!("0x{}", bytes_hex)
        }
        _ => {
            // Dynamic types (string, bytes, arrays): show truncated hex
            if hex_value.len() > 16 {
                format!("0x{}...", &hex_value[..16])
            } else {
                format!("0x{}", hex_value)
            }
        }
    }
}

/// Build a selector-to-signature map from a contract's parsed ABI.
pub fn build_selector_map(abi: &[AbiEntry]) -> std::collections::HashMap<String, String> {
    abi.iter()
        .filter(|e| e.entry_type == "function")
        .map(|e| (e.selector().to_lowercase(), e.signature()))
        .collect()
}

/// Get all state-changing functions from an ABI.
pub fn get_state_changing_functions(abi: &[AbiEntry]) -> Vec<&AbiEntry> {
    abi.iter().filter(|e| e.is_state_changing()).collect()
}

/// Get all events from an ABI.
pub fn get_events(abi: &[AbiEntry]) -> Vec<&AbiEntry> {
    abi.iter().filter(|e| e.entry_type == "event").collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::source::AbiParam;

    #[test]
    fn test_decode_address() {
        let hex = "000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7";
        let result = decode_abi_value(hex, "address");
        assert_eq!(result, "0xdac17f958d2ee523a2206206994597c13d831ec7");
    }

    #[test]
    fn test_decode_uint256() {
        let hex = "0000000000000000000000000000000000000000000000000000000005f5e100";
        let result = decode_abi_value(hex, "uint256");
        assert_eq!(result, "100000000");
    }

    #[test]
    fn test_decode_uint256_zero() {
        let hex = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = decode_abi_value(hex, "uint256");
        assert_eq!(result, "0");
    }

    #[test]
    fn test_decode_bool_true() {
        let hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let result = decode_abi_value(hex, "bool");
        assert_eq!(result, "true");
    }

    #[test]
    fn test_decode_bool_false() {
        let hex = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = decode_abi_value(hex, "bool");
        assert_eq!(result, "false");
    }

    #[test]
    fn test_decode_empty() {
        let result = decode_abi_value("", "uint256");
        assert_eq!(result, "(empty)");
    }

    #[test]
    fn test_native_transfer_decoding() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let client = reqwest::Client::new();
            decode_calldata("0x", None, &client).await
        });
        assert_eq!(result.function_name, "Native Transfer");
    }

    #[test]
    fn test_build_selector_map() {
        let abi = vec![
            AbiEntry {
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
            },
            AbiEntry {
                entry_type: "event".to_string(),
                name: "Transfer".to_string(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: String::new(),
            },
        ];
        let map = build_selector_map(&abi);
        assert_eq!(map.len(), 1); // Only functions, not events
        assert!(map.values().any(|v| v == "transfer(address,uint256)"));
    }

    #[test]
    fn test_get_state_changing_functions() {
        let abi = vec![
            AbiEntry {
                entry_type: "function".to_string(),
                name: "transfer".to_string(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: "nonpayable".to_string(),
            },
            AbiEntry {
                entry_type: "function".to_string(),
                name: "balanceOf".to_string(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: "view".to_string(),
            },
        ];
        let sc_fns = get_state_changing_functions(&abi);
        assert_eq!(sc_fns.len(), 1);
        assert_eq!(sc_fns[0].name, "transfer");
    }
}
