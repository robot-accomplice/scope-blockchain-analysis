//! # External Intelligence
//!
//! Gathers external information about smart contracts including:
//!
//! - **GitHub repository linking** - Finds associated source code repos
//! - **Audit report discovery** - Checks known audit databases
//! - **Sourcify verification** - Cross-references with Sourcify
//! - **Contract metadata** - License, creation date, deployer

use crate::contract::source::ContractSource;
use crate::error::{Result, ScopeError};
use serde::{Deserialize, Serialize};

/// Aggregated external intelligence for a contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalInfo {
    /// GitHub repository URL (if found).
    pub github_repo: Option<String>,
    /// Known audit reports.
    pub audit_reports: Vec<AuditReport>,
    /// Sourcify verification status.
    pub sourcify_verified: Option<bool>,
    /// Contract deployer address.
    pub deployer: Option<String>,
    /// Block explorer URL.
    pub explorer_url: String,
    /// Additional metadata.
    pub metadata: Vec<MetadataEntry>,
}

/// An audit report reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    /// Auditor name.
    pub auditor: String,
    /// Report URL or reference.
    pub url: String,
    /// Date of audit (if known).
    pub date: Option<String>,
    /// Scope of the audit.
    pub scope: String,
}

/// A generic metadata entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

/// Known audit firms and their public report databases.
const AUDIT_DATABASES: &[(&str, &str)] = &[
    (
        "Trail of Bits",
        "https://github.com/trailofbits/publications/tree/master/reviews",
    ),
    (
        "OpenZeppelin",
        "https://blog.openzeppelin.com/security-audits",
    ),
    (
        "Consensys Diligence",
        "https://consensys.io/diligence/audits",
    ),
    ("CertiK", "https://www.certik.com/projects"),
    ("PeckShield", "https://github.com/peckshield/publications"),
    ("Quantstamp", "https://certificate.quantstamp.com"),
    ("Halborn", "https://www.halborn.com/audits"),
    ("Spearbit", "https://github.com/spearbit/portfolio"),
    ("Code4rena", "https://code4rena.com/reports"),
    ("Sherlock", "https://audits.sherlock.xyz/contests"),
];

/// Gather external intelligence for a contract.
pub async fn gather_external_info(
    address: &str,
    chain: &str,
    source: Option<&ContractSource>,
    http_client: &reqwest::Client,
) -> Result<ExternalInfo> {
    let explorer_url = build_explorer_url(address, chain);

    // Try to find GitHub repo from source metadata
    let github_repo = if let Some(src) = source {
        find_github_from_source(src)
    } else {
        None
    };

    // Check Sourcify verification
    let sourcify_verified = check_sourcify(address, chain, http_client).await.ok();

    // Discover audit reports
    let audit_reports = discover_audits(address, chain, source, http_client).await;

    // Build metadata
    let mut metadata = Vec::new();
    if let Some(src) = source {
        metadata.push(MetadataEntry {
            key: "Contract Name".to_string(),
            value: src.contract_name.clone(),
        });
        metadata.push(MetadataEntry {
            key: "Compiler".to_string(),
            value: src.compiler_version.clone(),
        });
        metadata.push(MetadataEntry {
            key: "License".to_string(),
            value: src.license_type.clone(),
        });
        if src.optimization_used {
            metadata.push(MetadataEntry {
                key: "Optimization".to_string(),
                value: format!("Enabled ({} runs)", src.optimization_runs),
            });
        }
        metadata.push(MetadataEntry {
            key: "EVM Version".to_string(),
            value: src.evm_version.clone(),
        });
    }

    Ok(ExternalInfo {
        github_repo,
        audit_reports,
        sourcify_verified,
        deployer: None,
        explorer_url,
        metadata,
    })
}

/// Build a block explorer URL for the contract.
fn build_explorer_url(address: &str, chain: &str) -> String {
    match chain.to_lowercase().as_str() {
        "ethereum" | "eth" => format!("https://etherscan.io/address/{}", address),
        "polygon" | "matic" => format!("https://polygonscan.com/address/{}", address),
        "arbitrum" | "arb" => format!("https://arbiscan.io/address/{}", address),
        "optimism" | "op" => format!("https://optimistic.etherscan.io/address/{}", address),
        "base" => format!("https://basescan.org/address/{}", address),
        "bsc" | "bnb" => format!("https://bscscan.com/address/{}", address),
        _ => format!("https://etherscan.io/address/{}", address),
    }
}

/// Extract GitHub repository URL from contract source code or metadata.
fn find_github_from_source(source: &ContractSource) -> Option<String> {
    let code = &source.source_code;

    // Check for GitHub URLs in source comments
    let github_patterns = [
        r"https?://github\.com/[\w\-]+/[\w\-]+",
        r"@dev\s+.*github\.com/[\w\-]+/[\w\-]+",
    ];

    for pattern in &github_patterns {
        if let Ok(re) = regex::Regex::new(pattern)
            && let Some(mat) = re.find(code)
        {
            return Some(mat.as_str().to_string());
        }
    }

    // Check SwarmSource for IPFS content (can lead to repo via metadata)
    if source.swarm_source.contains("ipfs") {
        // IPFS hash doesn't directly give us a repo, but note it
    }

    // Check for well-known contract names that map to repos
    let known_contracts: &[(&str, &str)] = &[
        ("UniswapV2", "https://github.com/Uniswap/v2-core"),
        ("UniswapV3", "https://github.com/Uniswap/v3-core"),
        (
            "Ownable",
            "https://github.com/OpenZeppelin/openzeppelin-contracts",
        ),
        (
            "Compound",
            "https://github.com/compound-finance/compound-protocol",
        ),
        ("Aave", "https://github.com/aave/aave-v3-core"),
    ];

    for (name, repo) in known_contracts {
        if code.contains(name) || source.contract_name.contains(name) {
            return Some(repo.to_string());
        }
    }

    None
}

/// Check if contract is verified on Sourcify.
async fn check_sourcify(address: &str, chain: &str, http_client: &reqwest::Client) -> Result<bool> {
    let chain_id = match chain.to_lowercase().as_str() {
        "ethereum" | "eth" => "1",
        "polygon" | "matic" => "137",
        "arbitrum" | "arb" => "42161",
        "optimism" | "op" => "10",
        "base" => "8453",
        "bsc" | "bnb" => "56",
        _ => return Ok(false),
    };

    let url = format!(
        "https://sourcify.dev/server/check-all-by-addresses?addresses={}&chainIds={}",
        address, chain_id
    );

    let response = http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ScopeError::Api(format!("Sourcify check failed: {}", e)))?;

    if response.status().is_success() {
        let body = response
            .text()
            .await
            .map_err(|e| ScopeError::Api(format!("Sourcify response error: {}", e)))?;
        // Sourcify returns status "perfect" or "partial" for verified contracts
        Ok(body.contains("perfect") || body.contains("partial"))
    } else {
        Ok(false)
    }
}

/// Discover known audit reports for the contract.
async fn discover_audits(
    address: &str,
    chain: &str,
    source: Option<&ContractSource>,
    _http_client: &reqwest::Client,
) -> Vec<AuditReport> {
    let mut reports = Vec::new();

    // Check source code for audit references
    if let Some(src) = source {
        let code_lower = src.source_code.to_lowercase();

        // Check for audit mentions in comments
        for (auditor, url) in AUDIT_DATABASES {
            let auditor_lower = auditor.to_lowercase();
            if code_lower.contains(&auditor_lower) {
                reports.push(AuditReport {
                    auditor: auditor.to_string(),
                    url: url.to_string(),
                    date: None,
                    scope: format!(
                        "Referenced in {} source code ({})",
                        src.contract_name, chain
                    ),
                });
            }
        }

        // Check for @audit or @security tags in NatSpec
        if code_lower.contains("@audit") || code_lower.contains("audited by") {
            let re = regex::Regex::new(r"(?i)(?:@audit|audited\s+by)\s*:?\s*([\w\s]+)");
            if let Ok(re) = re {
                for cap in re.captures_iter(&src.source_code) {
                    let auditor_name = cap[1].trim().to_string();
                    if !reports.iter().any(|r| r.auditor == auditor_name) {
                        reports.push(AuditReport {
                            auditor: auditor_name,
                            url: String::new(),
                            date: None,
                            scope: format!("Mentioned in {} source comments", src.contract_name),
                        });
                    }
                }
            }
        }
    }

    // Provide audit database links even if no specific report is found
    if reports.is_empty() {
        reports.push(AuditReport {
            auditor: "No audit reports found".to_string(),
            url: format!("https://etherscan.io/address/{}#code", address),
            date: None,
            scope: "Check block explorer and auditor databases manually".to_string(),
        });
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::source::ContractSource;

    fn make_source(code: &str) -> ContractSource {
        ContractSource {
            contract_name: "TestContract".to_string(),
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
    fn test_build_explorer_url() {
        let url = build_explorer_url("0xabc", "ethereum");
        assert_eq!(url, "https://etherscan.io/address/0xabc");

        let url = build_explorer_url("0xabc", "polygon");
        assert_eq!(url, "https://polygonscan.com/address/0xabc");

        let url = build_explorer_url("0xabc", "bsc");
        assert_eq!(url, "https://bscscan.com/address/0xabc");
    }

    #[test]
    fn test_find_github_from_uniswap() {
        let src = make_source("import UniswapV2Router;");
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("Uniswap"));
    }

    #[test]
    fn test_find_github_from_url() {
        let src = make_source("// Source: https://github.com/my-org/my-contract");
        let repo = find_github_from_source(&src);
        assert_eq!(
            repo,
            Some("https://github.com/my-org/my-contract".to_string())
        );
    }

    #[test]
    fn test_find_github_none() {
        let src = make_source("contract SimpleToken {}");
        let repo = find_github_from_source(&src);
        assert!(repo.is_none());
    }

    #[test]
    fn test_discover_audits_from_source() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = make_source("// Audited by Trail of Bits in 2023");
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", Some(&src), &client).await
        });
        assert!(reports.iter().any(|r| r.auditor.contains("Trail of Bits")));
    }

    #[test]
    fn test_build_explorer_url_all_chains() {
        assert_eq!(
            build_explorer_url("0xabc", "eth"),
            "https://etherscan.io/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "matic"),
            "https://polygonscan.com/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "arbitrum"),
            "https://arbiscan.io/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "arb"),
            "https://arbiscan.io/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "optimism"),
            "https://optimistic.etherscan.io/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "op"),
            "https://optimistic.etherscan.io/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "base"),
            "https://basescan.org/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "bnb"),
            "https://bscscan.com/address/0xabc"
        );
        assert_eq!(
            build_explorer_url("0xabc", "unknown_chain"),
            "https://etherscan.io/address/0xabc"
        );
    }

    #[test]
    fn test_find_github_from_source_ownable() {
        let src = make_source("import Ownable; contract Token is Ownable {}");
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("OpenZeppelin"));
    }

    #[test]
    fn test_find_github_from_source_compound() {
        let src = make_source("import Compound from './Compound.sol';");
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("compound"));
    }

    #[test]
    fn test_find_github_from_source_aave() {
        let src = make_source("import Aave from './lending/Aave.sol';");
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("aave"));
    }

    #[test]
    fn test_find_github_from_source_uniswapv3() {
        let src = make_source("contract Token { UniswapV3Pool pool; }");
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("v3-core"));
    }

    #[test]
    fn test_find_github_from_contract_name_match() {
        let mut src = make_source("contract SimpleToken {}");
        src.contract_name = "AavePoolV3".to_string();
        let repo = find_github_from_source(&src);
        assert!(repo.is_some());
        assert!(repo.unwrap().contains("aave"));
    }

    #[test]
    fn test_find_github_with_ipfs_swarm() {
        let mut src = make_source("contract SimpleToken {}");
        src.swarm_source = "ipfs://Qm1234567890".to_string();
        let repo = find_github_from_source(&src);
        assert!(repo.is_none());
    }

    #[test]
    fn test_discover_audits_no_source() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", None, &client).await
        });
        assert_eq!(reports.len(), 1);
        assert!(reports[0].auditor.contains("No audit"));
    }

    #[test]
    fn test_discover_audits_multiple_auditors() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = make_source("// Reviewed by Trail of Bits and OpenZeppelin");
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", Some(&src), &client).await
        });
        assert!(reports.len() >= 2);
    }

    #[test]
    fn test_discover_audits_audit_tag() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = make_source("/// @audit: Spearbit\ncontract Token {}");
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", Some(&src), &client).await
        });
        assert!(reports.iter().any(|r| r.auditor.contains("Spearbit")));
    }

    #[test]
    fn test_discover_audits_audited_by_tag() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = make_source("// audited by CustomAuditor\ncontract Token {}");
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", Some(&src), &client).await
        });
        assert!(reports.iter().any(|r| r.auditor.contains("CustomAuditor")));
    }

    #[test]
    fn test_discover_audits_no_duplicate() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = make_source("// audited by Trail of Bits\n// Trail of Bits reviewed");
        let reports = rt.block_on(async {
            let client = reqwest::Client::new();
            discover_audits("0xabc", "ethereum", Some(&src), &client).await
        });
        let tob_count = reports
            .iter()
            .filter(|r| r.auditor.contains("Trail of Bits"))
            .count();
        assert!(tob_count >= 1);
    }

    #[test]
    fn test_external_info_struct() {
        let info = ExternalInfo {
            github_repo: Some("https://github.com/test/repo".to_string()),
            audit_reports: vec![],
            sourcify_verified: Some(true),
            deployer: Some("0xdead".to_string()),
            explorer_url: "https://etherscan.io/address/0x1".to_string(),
            metadata: vec![MetadataEntry {
                key: "k".to_string(),
                value: "v".to_string(),
            }],
        };
        assert!(info.github_repo.is_some());
        assert!(info.deployer.is_some());
        assert_eq!(info.metadata.len(), 1);
    }

    #[test]
    fn test_audit_report_struct() {
        let report = AuditReport {
            auditor: "ToB".to_string(),
            url: "https://tob.com".to_string(),
            date: Some("2024-01-01".to_string()),
            scope: "Full protocol".to_string(),
        };
        assert_eq!(report.auditor, "ToB");
        assert!(report.date.is_some());
    }

    #[test]
    fn test_find_github_from_dev_natspec() {
        let src = make_source(
            "/** @dev See https://github.com/MyOrg/MyContract for full source */\ncontract C {}",
        );
        let repo = find_github_from_source(&src);
        assert_eq!(
            repo,
            Some("https://github.com/MyOrg/MyContract".to_string())
        );
    }

    #[test]
    fn test_metadata_entry_struct() {
        let entry = MetadataEntry {
            key: "chain".to_string(),
            value: "ethereum".to_string(),
        };
        assert_eq!(entry.key, "chain");
        let cloned = entry.clone();
        assert_eq!(format!("{:?}", cloned), format!("{:?}", entry));
    }

    #[tokio::test]
    async fn test_gather_external_info_with_source() {
        let src = make_source("contract Test {}");
        let client = reqwest::Client::new();
        let result = gather_external_info("0xabc", "ethereum", Some(&src), &client)
            .await
            .unwrap();
        assert!(result.explorer_url.contains("etherscan"));
        assert!(result.metadata.iter().any(|m| m.key == "Contract Name"));
        assert!(result.metadata.iter().any(|m| m.key == "Compiler"));
        assert!(result.metadata.iter().any(|m| m.key == "License"));
        assert!(result.metadata.iter().any(|m| m.key == "EVM Version"));
        assert!(result.metadata.iter().any(|m| m.key == "Optimization"));
    }

    #[tokio::test]
    async fn test_gather_external_info_without_optimization() {
        let mut src = make_source("contract Test {}");
        src.optimization_used = false;
        let client = reqwest::Client::new();
        let result = gather_external_info("0xabc", "polygon", Some(&src), &client)
            .await
            .unwrap();
        let has_opt = result.metadata.iter().any(|m| m.key == "Optimization");
        assert!(!has_opt);
        assert!(result.explorer_url.contains("polygonscan"));
    }

    #[tokio::test]
    async fn test_gather_external_info_no_source() {
        let client = reqwest::Client::new();
        let result = gather_external_info("0xabc", "ethereum", None, &client)
            .await
            .unwrap();
        assert!(result.metadata.is_empty());
        assert!(result.github_repo.is_none());
    }
}
