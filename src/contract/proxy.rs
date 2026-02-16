//! # Proxy Pattern Detection
//!
//! Detects common Ethereum proxy patterns by analyzing bytecode,
//! storage slots, and source code metadata.
//!
//! ## Supported Patterns
//!
//! - **EIP-1967** - Transparent Proxy (implementation slot: `0x360894...`)
//! - **EIP-1822** - UUPS (Universal Upgradeable Proxy Standard)
//! - **Transparent Proxy** - OpenZeppelin TransparentUpgradeableProxy
//! - **Beacon Proxy** - EIP-1967 beacon slot
//! - **Minimal Proxy** (EIP-1167) - Clone factory pattern (bytecode detection)
//! - **Diamond** (EIP-2535) - Multi-facet proxy
//!
//! ## Detection Methods
//!
//! 1. **Etherscan metadata** - `Proxy` and `Implementation` fields from getsourcecode
//! 2. **Storage slot reads** - Check well-known EIP-1967 slots
//! 3. **Bytecode patterns** - EIP-1167 minimal proxy prefix/suffix

use crate::contract::source::ContractSource;
use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Well-known EIP-1967 storage slots.
/// Implementation slot: bytes32(uint256(keccak256('eip1967.proxy.implementation')) - 1)
pub const EIP1967_IMPL_SLOT: &str =
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
/// Admin slot: bytes32(uint256(keccak256('eip1967.proxy.admin')) - 1)
pub const EIP1967_ADMIN_SLOT: &str =
    "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";
/// Beacon slot: bytes32(uint256(keccak256('eip1967.proxy.beacon')) - 1)
pub const EIP1967_BEACON_SLOT: &str =
    "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50";

/// EIP-1167 minimal proxy bytecode prefix (first 10 bytes).
const MINIMAL_PROXY_PREFIX: &str = "363d3d373d3d3d363d73";
/// EIP-1167 minimal proxy bytecode suffix (last 15 bytes).
const MINIMAL_PROXY_SUFFIX: &str = "5af43d82803e903d91602b57fd5bf3";

/// Result of proxy pattern detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
    /// Whether the contract is a proxy.
    pub is_proxy: bool,
    /// Type of proxy pattern detected.
    pub proxy_type: String,
    /// Implementation contract address (if resolvable).
    pub implementation_address: Option<String>,
    /// Admin address that can upgrade the proxy (if detectable).
    pub admin_address: Option<String>,
    /// Beacon address (for beacon proxies).
    pub beacon_address: Option<String>,
    /// Detailed findings about the proxy pattern.
    pub details: Vec<String>,
}

impl Default for ProxyInfo {
    fn default() -> Self {
        Self {
            is_proxy: false,
            proxy_type: "None".to_string(),
            implementation_address: None,
            admin_address: None,
            beacon_address: None,
            details: Vec::new(),
        }
    }
}

/// Detect proxy patterns using all available data sources.
///
/// Checks (in order):
/// 1. Etherscan source metadata (Proxy/Implementation fields)
/// 2. Bytecode analysis (EIP-1167 minimal proxy)
/// 3. Storage slot reads (EIP-1967 implementation/admin/beacon)
/// 4. Source code patterns (if verified)
pub async fn detect_proxy(
    address: &str,
    _chain: &str,
    bytecode: &str,
    source: Option<&ContractSource>,
    client: &dyn crate::chains::ChainClient,
    _http_client: &reqwest::Client,
) -> Result<ProxyInfo> {
    let mut info = ProxyInfo::default();

    // 1. Check Etherscan metadata
    if let Some(src) = source
        && src.is_proxy
    {
        info.is_proxy = true;
        info.proxy_type = "Etherscan-detected proxy".to_string();
        info.implementation_address = src.implementation_address.clone();
        info.details
            .push("Etherscan flags this contract as a proxy.".to_string());
    }

    // 2. Check EIP-1167 minimal proxy (bytecode-only detection)
    let code = bytecode.trim_start_matches("0x").to_lowercase();
    if code.starts_with(MINIMAL_PROXY_PREFIX) && code.ends_with(MINIMAL_PROXY_SUFFIX) {
        info.is_proxy = true;
        info.proxy_type = "EIP-1167 Minimal Proxy (Clone)".to_string();
        // Extract implementation address from bytecode (20 bytes after prefix)
        if code.len() >= MINIMAL_PROXY_PREFIX.len() + 40 {
            let impl_addr = &code[MINIMAL_PROXY_PREFIX.len()..MINIMAL_PROXY_PREFIX.len() + 40];
            info.implementation_address = Some(format!("0x{}", impl_addr));
        }
        info.details
            .push("EIP-1167 minimal proxy detected from bytecode pattern.".to_string());
        return Ok(info);
    }

    // 3. Check EIP-1967 storage slots
    if let Ok(impl_slot) = client.get_storage_at(address, EIP1967_IMPL_SLOT).await {
        let impl_addr = extract_address_from_slot(&impl_slot);
        if let Some(addr) = impl_addr {
            info.is_proxy = true;
            info.implementation_address = Some(addr.clone());
            info.details
                .push(format!("EIP-1967 implementation slot points to {}", addr));

            // Determine proxy subtype
            if let Some(src) = source {
                let code_lower = src.source_code.to_lowercase();
                if code_lower.contains("uups") || code_lower.contains("_upgradeto") {
                    info.proxy_type = "UUPS (EIP-1822)".to_string();
                } else {
                    info.proxy_type = "Transparent Proxy (EIP-1967)".to_string();
                }
            } else {
                info.proxy_type = "EIP-1967 Proxy".to_string();
            }
        }
    }

    // Check admin slot
    if let Ok(admin_slot) = client.get_storage_at(address, EIP1967_ADMIN_SLOT).await {
        let admin_addr = extract_address_from_slot(&admin_slot);
        if let Some(addr) = admin_addr {
            info.admin_address = Some(addr.clone());
            info.details
                .push(format!("EIP-1967 admin slot points to {}", addr));
        }
    }

    // Check beacon slot
    if let Ok(beacon_slot) = client.get_storage_at(address, EIP1967_BEACON_SLOT).await {
        let beacon_addr = extract_address_from_slot(&beacon_slot);
        if let Some(addr) = beacon_addr {
            info.is_proxy = true;
            info.proxy_type = "Beacon Proxy (EIP-1967)".to_string();
            info.beacon_address = Some(addr.clone());
            info.details
                .push(format!("EIP-1967 beacon slot points to {}", addr));
        }
    }

    // 4. Source code pattern analysis
    if let Some(src) = source {
        detect_proxy_from_source(src, &mut info);
    }

    // Check bytecode for DELEGATECALL pattern (generic proxy indicator)
    if !info.is_proxy && code.contains("f4") {
        // 0xf4 = DELEGATECALL opcode; not definitive alone but notable
        if let Some(src) = source
            && src.source_code.contains("delegatecall")
        {
            info.details.push(
                "Contract uses delegatecall (may be a proxy or proxy-like pattern).".to_string(),
            );
        }
    }

    Ok(info)
}

/// Extract an Ethereum address from a 32-byte storage slot value.
/// Returns None if the slot is empty (all zeros).
fn extract_address_from_slot(slot_value: &str) -> Option<String> {
    let hex = slot_value.trim_start_matches("0x").to_lowercase();
    if hex.len() < 40 {
        return None;
    }
    // Address is in the last 40 hex chars of the 32-byte slot
    let addr = &hex[hex.len() - 40..];
    // Check if it's not the zero address
    if addr == "0000000000000000000000000000000000000000" {
        return None;
    }
    Some(format!("0x{}", addr))
}

/// Detect proxy patterns from source code analysis.
fn detect_proxy_from_source(source: &ContractSource, info: &mut ProxyInfo) {
    let code = &source.source_code;
    let code_lower = code.to_lowercase();

    // Diamond proxy (EIP-2535)
    if code_lower.contains("diamondcut")
        || code_lower.contains("idiamond")
        || code_lower.contains("facetcut")
    {
        info.is_proxy = true;
        info.proxy_type = "Diamond Proxy (EIP-2535)".to_string();
        info.details
            .push("Diamond/multi-facet proxy pattern detected in source.".to_string());
    }

    // OpenZeppelin upgradeable patterns
    if code_lower.contains("transparentupgradeableproxy") {
        info.is_proxy = true;
        info.proxy_type = "OpenZeppelin TransparentUpgradeableProxy".to_string();
        info.details
            .push("OpenZeppelin TransparentUpgradeableProxy import detected.".to_string());
    }

    if code_lower.contains("uupsupgradeable") {
        info.is_proxy = true;
        info.proxy_type = "UUPS (OpenZeppelin UUPSUpgradeable)".to_string();
        info.details
            .push("OpenZeppelin UUPSUpgradeable import detected.".to_string());
    }

    if code_lower.contains("beaconproxy") || code_lower.contains("upgradeablebeacon") {
        info.is_proxy = true;
        info.proxy_type = "Beacon Proxy (OpenZeppelin)".to_string();
        info.details
            .push("OpenZeppelin BeaconProxy pattern detected in source.".to_string());
    }

    // Generic proxy/upgrade patterns
    if code_lower.contains("_setimplementation") || code_lower.contains("upgradeto(") {
        if !info.is_proxy {
            info.is_proxy = true;
            info.proxy_type = "Upgradeable Proxy (custom)".to_string();
        }
        info.details
            .push("Upgrade mechanism (upgradeTo/_setImplementation) found in source.".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_address_from_slot() {
        let slot = "0x000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7";
        let addr = extract_address_from_slot(slot);
        assert_eq!(
            addr,
            Some("0xdac17f958d2ee523a2206206994597c13d831ec7".to_string())
        );
    }

    #[test]
    fn test_extract_address_from_zero_slot() {
        let slot = "0x0000000000000000000000000000000000000000000000000000000000000000";
        let addr = extract_address_from_slot(slot);
        assert_eq!(addr, None);
    }

    #[test]
    fn test_minimal_proxy_detection() {
        let prefix = MINIMAL_PROXY_PREFIX;
        let suffix = MINIMAL_PROXY_SUFFIX;
        let impl_addr = "bebebebebebebebebebebebebebebebebebebebe";
        let bytecode = format!("0x{}{}{}", prefix, impl_addr, suffix);
        let code = bytecode.trim_start_matches("0x").to_lowercase();
        assert!(code.starts_with(MINIMAL_PROXY_PREFIX));
        assert!(code.ends_with(MINIMAL_PROXY_SUFFIX));
    }

    #[test]
    fn test_detect_proxy_from_source_diamond() {
        let src = ContractSource {
            contract_name: "DiamondProxy".to_string(),
            source_code: "contract DiamondProxy { function diamondCut(...) {} }".to_string(),
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
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("Diamond"));
    }

    #[test]
    fn test_detect_proxy_from_source_uups() {
        let src = ContractSource {
            contract_name: "UUPSToken".to_string(),
            source_code: "import UUPSUpgradeable; contract Token is UUPSUpgradeable {}".to_string(),
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
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("UUPS"));
    }

    fn make_source(code: &str) -> ContractSource {
        ContractSource {
            contract_name: "Test".to_string(),
            source_code: code.to_string(),
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
        }
    }

    #[test]
    fn test_extract_address_short_hex() {
        assert_eq!(extract_address_from_slot("0xabc"), None);
    }

    #[test]
    fn test_extract_address_no_prefix() {
        let slot = "000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7";
        let addr = extract_address_from_slot(slot);
        assert_eq!(addr, Some("0xdac17f958d2ee523a2206206994597c13d831ec7".to_string()));
    }

    #[test]
    fn test_detect_transparent_upgradeable() {
        let src = make_source("import TransparentUpgradeableProxy; contract P is TransparentUpgradeableProxy {}");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("TransparentUpgradeableProxy"));
    }

    #[test]
    fn test_detect_beacon_proxy() {
        let src = make_source("import BeaconProxy; contract P is BeaconProxy { UpgradeableBeacon b; }");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("Beacon"));
    }

    #[test]
    fn test_detect_upgrade_to_pattern() {
        let src = make_source("function upgradeTo(address impl) { _setImplementation(impl); }");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.details.iter().any(|d| d.contains("upgradeTo")));
    }

    #[test]
    fn test_non_proxy_source() {
        let src = make_source("contract Token { function transfer() {} }");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(!info.is_proxy);
    }

    #[test]
    fn test_proxy_info_default() {
        let info = ProxyInfo::default();
        assert!(!info.is_proxy);
        assert_eq!(info.proxy_type, "None");
        assert!(info.implementation_address.is_none());
        assert!(info.admin_address.is_none());
        assert!(info.beacon_address.is_none());
        assert!(info.details.is_empty());
    }

    #[test]
    fn test_detect_proxy_from_source_idiamond() {
        let src = make_source("import IDiamond; contract D is IDiamond { }");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("Diamond"));
    }

    #[test]
    fn test_detect_proxy_from_source_facetcut() {
        let src = make_source("struct FacetCut { address target; } function addFacet(FacetCut[] calldata)");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.proxy_type.contains("Diamond"));
    }

    #[test]
    fn test_detect_proxy_from_source_set_implementation() {
        let src = make_source("function _setImplementation(address impl) internal { _impl = impl; }");
        let mut info = ProxyInfo::default();
        detect_proxy_from_source(&src, &mut info);
        assert!(info.is_proxy);
        assert!(info.details.iter().any(|d| d.contains("_setImplementation")));
    }

    #[test]
    fn test_extract_address_from_slot_uppercase_hex() {
        let slot = "0x000000000000000000000000DAC17F958D2EE523A2206206994597C13D831EC7";
        let addr = extract_address_from_slot(slot);
        assert_eq!(
            addr,
            Some("0xdac17f958d2ee523a2206206994597c13d831ec7".to_string())
        );
    }

    #[test]
    fn test_proxy_info_serialization() {
        let info = ProxyInfo {
            is_proxy: true,
            proxy_type: "EIP-1967".to_string(),
            implementation_address: Some("0x1234567890123456789012345678901234567890".to_string()),
            admin_address: Some("0xadmin".to_string()),
            beacon_address: None,
            details: vec!["Detail 1".to_string()],
        };
        let json = serde_json::to_string(&info).unwrap();
        let restored: ProxyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.is_proxy, info.is_proxy);
        assert_eq!(restored.implementation_address, info.implementation_address);
        assert_eq!(restored.details.len(), 1);
    }

    #[tokio::test]
    async fn test_detect_proxy_minimal_eip1167() {
        let prefix = MINIMAL_PROXY_PREFIX;
        let suffix = MINIMAL_PROXY_SUFFIX;
        let impl_addr = "bebebebebebebebebebebebebebebebebebebebe";
        let bytecode = format!("0x{}{}{}", prefix, impl_addr, suffix);
        let client = crate::chains::mocks::MockChainClient::new("ethereum", "ETH");
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            &bytecode,
            None,
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.is_proxy);
        assert!(result.proxy_type.contains("EIP-1167"));
        assert_eq!(
            result.implementation_address,
            Some("0xbebebebebebebebebebebebebebebebebebebebe".to_string())
        );
    }

    #[tokio::test]
    async fn test_detect_proxy_etherscan_metadata() {
        let src = ContractSource {
            contract_name: "Proxy".to_string(),
            source_code: "contract Proxy {}".to_string(),
            abi: "[]".to_string(),
            compiler_version: "v0.8.19".to_string(),
            optimization_used: true,
            optimization_runs: 200,
            evm_version: "paris".to_string(),
            license_type: "MIT".to_string(),
            is_proxy: true,
            implementation_address: Some("0x1234567890123456789012345678901234567890".to_string()),
            constructor_arguments: String::new(),
            library: String::new(),
            swarm_source: String::new(),
            parsed_abi: vec![],
        };
        let bytecode = "0x6080604052348015600f57600080fd5b50"; // Non-minimal-proxy bytecode
        let client = crate::chains::mocks::MockChainClient::new("ethereum", "ETH");
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            bytecode,
            Some(&src),
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.is_proxy);
        assert!(result.proxy_type.contains("Etherscan"));
        assert_eq!(
            result.implementation_address,
            Some("0x1234567890123456789012345678901234567890".to_string())
        );
    }

    #[test]
    fn test_extract_address_from_slot_exactly_40_chars() {
        let slot_str = "0x".to_string() + &"0".repeat(24) + "dac17f958d2ee523a2206206994597c13d831ec7";
        let addr = extract_address_from_slot(&slot_str);
        assert_eq!(
            addr,
            Some("0xdac17f958d2ee523a2206206994597c13d831ec7".to_string())
        );
    }

    /// Mock that returns configurable storage slot data for EIP-1967 proxy tests.
    struct StorageMockClient {
        inner: crate::chains::mocks::MockChainClient,
        impl_slot_value: Option<String>,
        admin_slot_value: Option<String>,
        beacon_slot_value: Option<String>,
    }

    impl StorageMockClient {
        fn with_impl_slot(addr: &str) -> Self {
            let padded = format!("0x{}{}", "0".repeat(24), addr.trim_start_matches("0x"));
            Self {
                inner: crate::chains::mocks::MockChainClient::new("ethereum", "ETH"),
                impl_slot_value: Some(padded),
                admin_slot_value: None,
                beacon_slot_value: None,
            }
        }

        fn with_all_slots(impl_addr: &str, admin_addr: &str, beacon_addr: &str) -> Self {
            let pad = |a: &str| format!("0x{}{}", "0".repeat(24), a.trim_start_matches("0x"));
            Self {
                inner: crate::chains::mocks::MockChainClient::new("ethereum", "ETH"),
                impl_slot_value: Some(pad(impl_addr)),
                admin_slot_value: Some(pad(admin_addr)),
                beacon_slot_value: Some(pad(beacon_addr)),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::chains::ChainClient for StorageMockClient {
        fn chain_name(&self) -> &str {
            self.inner.chain_name()
        }

        fn native_token_symbol(&self) -> &str {
            self.inner.native_token_symbol()
        }

        async fn get_balance(&self, a: &str) -> crate::error::Result<crate::chains::Balance> {
            self.inner.get_balance(a).await
        }

        async fn enrich_balance_usd(&self, b: &mut crate::chains::Balance) {
            self.inner.enrich_balance_usd(b).await
        }

        async fn get_transaction(&self, h: &str) -> crate::error::Result<crate::chains::Transaction> {
            self.inner.get_transaction(h).await
        }

        async fn get_transactions(&self, a: &str, limit: u32) -> crate::error::Result<Vec<crate::chains::Transaction>> {
            self.inner.get_transactions(a, limit).await
        }

        async fn get_block_number(&self) -> crate::error::Result<u64> {
            self.inner.get_block_number().await
        }

        async fn get_token_balances(&self, a: &str) -> crate::error::Result<Vec<crate::chains::TokenBalance>> {
            self.inner.get_token_balances(a).await
        }

        async fn get_storage_at(&self, _address: &str, slot: &str) -> crate::error::Result<String> {
            if slot == EIP1967_IMPL_SLOT {
                if let Some(ref v) = self.impl_slot_value {
                    return Ok(v.clone());
                }
            }
            if slot == EIP1967_ADMIN_SLOT {
                if let Some(ref v) = self.admin_slot_value {
                    return Ok(v.clone());
                }
            }
            if slot == EIP1967_BEACON_SLOT {
                if let Some(ref v) = self.beacon_slot_value {
                    return Ok(v.clone());
                }
            }
            Err(crate::error::ScopeError::Chain("No storage mock".to_string()))
        }
    }

    #[tokio::test]
    async fn test_detect_proxy_eip1967_transparent() {
        let client = StorageMockClient::with_impl_slot(
            "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        );
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            "0x6080604052",
            None,
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.is_proxy);
        assert!(result.proxy_type.contains("EIP-1967"));
        assert_eq!(
            result.implementation_address,
            Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string())
        );
    }

    #[tokio::test]
    async fn test_detect_proxy_eip1967_uups_with_source() {
        let src = ContractSource {
            contract_name: "UUPSProxy".to_string(),
            source_code: "contract UUPSProxy { function _upgradeTo(address) {} }".to_string(),
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
        let client = StorageMockClient::with_impl_slot(
            "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        );
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            "0x6080604052",
            Some(&src),
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.is_proxy);
        assert!(result.proxy_type.contains("UUPS"));
    }

    #[tokio::test]
    async fn test_detect_proxy_eip1967_admin_and_beacon_slots() {
        let client = StorageMockClient::with_all_slots(
            "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "b0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "c0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        );
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            "0x6080604052",
            None,
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.is_proxy);
        assert!(result.admin_address.is_some());
        assert!(result.beacon_address.is_some());
        assert!(result.proxy_type.contains("Beacon"));
    }

    #[tokio::test]
    async fn test_detect_proxy_delegatecall_in_source() {
        let src = ContractSource {
            contract_name: "DelegateProxy".to_string(),
            source_code: "contract D { function run() { delegatecall(...); } }".to_string(),
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
        let client = crate::chains::mocks::MockChainClient::new("ethereum", "ETH");
        let http = reqwest::Client::new();
        let result = detect_proxy(
            "0xproxy",
            "ethereum",
            "0x6080f4604052",
            Some(&src),
            &client,
            &http,
        )
        .await
        .unwrap();
        assert!(result.details.iter().any(|d| d.contains("delegatecall")));
    }
}
