//! # Smart Contract Analysis Module
//!
//! Provides comprehensive contract analysis capabilities for EVM-compatible
//! blockchain contracts, including:
//!
//! - **Source code retrieval** from block explorers (Etherscan, Sourcify)
//! - **ABI parsing** and function signature decoding (4byte.directory)
//! - **Proxy pattern detection** (EIP-1967, EIP-1822, UUPS, Transparent)
//! - **Access control mapping** (Ownable, AccessControl, Roles)
//! - **Vulnerability heuristics** (reentrancy, unchecked calls, selfdestruct, etc.)
//! - **DeFi protocol checks** (oracle manipulation, flash loans, lending, swaps)
//! - **External intelligence** (GitHub linking, audit report discovery)
//!
//! ## Architecture
//!
//! The module is organized into submodules that build on each other:
//!
//! ```text
//! contract::source       → Raw data retrieval (source, ABI, bytecode metadata)
//! contract::abi          → Function signature lookup and calldata decoding
//! contract::proxy        → Proxy pattern identification and implementation resolution
//! contract::access       → Access control and privilege mapping
//! contract::vulnerability → Security heuristic scanning
//! contract::defi         → DeFi-specific protocol analysis
//! contract::external     → GitHub repo linking and audit report discovery
//! ```

pub mod abi;
pub mod access;
pub mod defi;
pub mod external;
pub mod proxy;
pub mod source;
pub mod vulnerability;

use serde::{Deserialize, Serialize};

/// Complete contract analysis result aggregating all submodule outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAnalysis {
    /// Contract address analyzed.
    pub address: String,
    /// Chain the contract is deployed on.
    pub chain: String,
    /// Whether the contract source is verified on the block explorer.
    pub is_verified: bool,
    /// Contract source information (if verified).
    pub source_info: Option<source::ContractSource>,
    /// Proxy detection results.
    pub proxy_info: Option<proxy::ProxyInfo>,
    /// Access control mapping.
    pub access_control: Option<access::AccessControlMap>,
    /// Vulnerability scan findings.
    pub vulnerabilities: Vec<vulnerability::VulnerabilityFinding>,
    /// DeFi protocol analysis.
    pub defi_analysis: Option<defi::DefiAnalysis>,
    /// External intelligence (GitHub, audits).
    pub external_info: Option<external::ExternalInfo>,
    /// Overall security score (0-100, higher = safer).
    pub security_score: u32,
    /// Human-readable security summary.
    pub security_summary: String,
}

/// Run a full contract analysis pipeline on an address.
///
/// Orchestrates all submodules: source retrieval, proxy detection,
/// access control, vulnerability scanning, DeFi checks, and external intel.
pub async fn analyze_contract(
    address: &str,
    chain: &str,
    client: &dyn crate::chains::ChainClient,
    http_client: &reqwest::Client,
) -> crate::error::Result<ContractAnalysis> {
    // Step 1: Verify it's actually a contract
    let code = client.get_code(address).await?;
    if code.is_empty() || code == "0x" {
        return Err(crate::error::ScopeError::Chain(format!(
            "{} is an EOA (externally owned account), not a contract",
            address
        )));
    }

    // Step 2: Fetch source code and ABI
    let source_result = source::fetch_contract_source(address, chain, http_client).await;
    let source_info = source_result.ok();
    let is_verified = source_info.is_some();

    // Step 3: Proxy detection (uses bytecode + source metadata)
    let proxy_info = proxy::detect_proxy(
        address,
        chain,
        &code,
        source_info.as_ref(),
        client,
        http_client,
    )
    .await
    .ok();

    // Step 4: Access control analysis (requires source)
    let access_control = source_info.as_ref().map(access::analyze_access_control);

    // Step 5: Vulnerability scan (requires source + ABI context)
    let vulnerabilities = if let Some(src) = source_info.as_ref() {
        vulnerability::scan_vulnerabilities(src)
    } else {
        vulnerability::scan_bytecode_only(&code)
    };

    // Step 6: DeFi protocol checks
    let defi_analysis = source_info.as_ref().map(defi::analyze_defi_patterns);

    // Step 7: External intelligence
    let external_info =
        external::gather_external_info(address, chain, source_info.as_ref(), http_client)
            .await
            .ok();

    // Compute security score
    let security_score = compute_security_score(
        is_verified,
        &proxy_info,
        &access_control,
        &vulnerabilities,
        &defi_analysis,
        &external_info,
    );

    let security_summary = generate_security_summary(
        is_verified,
        &proxy_info,
        &access_control,
        &vulnerabilities,
        security_score,
    );

    Ok(ContractAnalysis {
        address: address.to_string(),
        chain: chain.to_string(),
        is_verified,
        source_info,
        proxy_info,
        access_control,
        vulnerabilities,
        defi_analysis,
        external_info,
        security_score,
        security_summary,
    })
}

/// Compute an overall security score (0-100) from analysis components.
fn compute_security_score(
    is_verified: bool,
    proxy_info: &Option<proxy::ProxyInfo>,
    access_control: &Option<access::AccessControlMap>,
    vulnerabilities: &[vulnerability::VulnerabilityFinding],
    defi_analysis: &Option<defi::DefiAnalysis>,
    external_info: &Option<external::ExternalInfo>,
) -> u32 {
    let mut score: i32 = 50; // Base score

    // Verification bonus
    if is_verified {
        score += 15;
    } else {
        score -= 20;
    }

    // Proxy considerations
    if let Some(proxy) = proxy_info
        && proxy.is_proxy
    {
        score -= 5; // Proxies add complexity
        if proxy.admin_address.is_some() {
            score += 3; // At least admin is identifiable
        }
    }

    // Access control
    if let Some(ac) = access_control {
        if ac.has_renounced_ownership {
            score += 10;
        }
        if ac.has_role_based_access {
            score += 5;
        }
        if ac.uses_tx_origin {
            score -= 15;
        }
    }

    // Vulnerabilities
    for vuln in vulnerabilities {
        match vuln.severity {
            vulnerability::Severity::Critical => score -= 20,
            vulnerability::Severity::High => score -= 12,
            vulnerability::Severity::Medium => score -= 6,
            vulnerability::Severity::Low => score -= 2,
            vulnerability::Severity::Informational => score -= 1,
        }
    }

    // DeFi risk factors
    if let Some(defi) = defi_analysis {
        if defi.has_oracle_dependency {
            score -= 5;
        }
        if defi.has_flash_loan_risk {
            score -= 8;
        }
    }

    // External validation
    if let Some(ext) = external_info {
        if !ext.audit_reports.is_empty() {
            score += 15;
        }
        if ext.github_repo.is_some() {
            score += 5;
        }
    }

    score.clamp(0, 100) as u32
}

/// Generate a human-readable security summary.
fn generate_security_summary(
    is_verified: bool,
    proxy_info: &Option<proxy::ProxyInfo>,
    access_control: &Option<access::AccessControlMap>,
    vulnerabilities: &[vulnerability::VulnerabilityFinding],
    score: u32,
) -> String {
    let mut parts = Vec::new();

    // Verification status
    if is_verified {
        parts.push("Source code is verified on the block explorer.".to_string());
    } else {
        parts.push(
            "WARNING: Source code is NOT verified — unable to perform source-level analysis."
                .to_string(),
        );
    }

    // Proxy status
    if let Some(proxy) = proxy_info
        && proxy.is_proxy
    {
        parts.push(format!(
            "Contract is a {} proxy{}.",
            proxy.proxy_type,
            proxy
                .implementation_address
                .as_ref()
                .map(|a| format!(" pointing to {}", a))
                .unwrap_or_default()
        ));
    }

    // Access control
    if let Some(ac) = access_control {
        if ac.has_renounced_ownership {
            parts.push("Ownership has been renounced.".to_string());
        } else if !ac.privileged_functions.is_empty() {
            parts.push(format!(
                "{} privileged function(s) found with owner/admin restrictions.",
                ac.privileged_functions.len()
            ));
        }
        if ac.uses_tx_origin {
            parts.push("DANGER: Uses tx.origin for authorization.".to_string());
        }
    }

    // Vulnerability summary
    let critical = vulnerabilities
        .iter()
        .filter(|v| v.severity == vulnerability::Severity::Critical)
        .count();
    let high = vulnerabilities
        .iter()
        .filter(|v| v.severity == vulnerability::Severity::High)
        .count();
    if critical > 0 || high > 0 {
        parts.push(format!(
            "Found {} critical and {} high severity issue(s).",
            critical, high
        ));
    } else if vulnerabilities.is_empty() {
        parts.push("No vulnerability heuristics triggered.".to_string());
    } else {
        parts.push(format!(
            "{} lower-severity finding(s) detected.",
            vulnerabilities.len()
        ));
    }

    // Score assessment
    let rating = match score {
        80..=100 => "GOOD",
        60..=79 => "MODERATE",
        40..=59 => "CAUTION",
        20..=39 => "HIGH RISK",
        _ => "CRITICAL RISK",
    };
    parts.push(format!("Security Score: {}/100 ({})", score, rating));

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_security_score_verified() {
        let score = compute_security_score(true, &None, &None, &[], &None, &None);
        assert_eq!(score, 65); // 50 base + 15 verified
    }

    #[test]
    fn test_compute_security_score_unverified() {
        let score = compute_security_score(false, &None, &None, &[], &None, &None);
        assert_eq!(score, 30); // 50 base - 20 unverified
    }

    #[test]
    fn test_compute_security_score_with_critical_vuln() {
        let vulns = vec![vulnerability::VulnerabilityFinding {
            id: "TEST-001".to_string(),
            title: "Test".to_string(),
            severity: vulnerability::Severity::Critical,
            category: vulnerability::VulnCategory::Reentrancy,
            description: "Test finding".to_string(),
            source_location: None,
            recommendation: "Fix it".to_string(),
        }];
        let score = compute_security_score(true, &None, &None, &vulns, &None, &None);
        assert_eq!(score, 45); // 65 - 20 critical
    }

    #[test]
    fn test_security_summary_unverified() {
        let summary = generate_security_summary(false, &None, &None, &[], 30);
        assert!(summary.contains("NOT verified"));
        assert!(summary.contains("30/100"));
    }

    #[test]
    fn test_score_clamped_to_valid_range() {
        let many_vulns: Vec<_> = (0..10)
            .map(|i| vulnerability::VulnerabilityFinding {
                id: format!("V-{}", i),
                title: "Critical".to_string(),
                severity: vulnerability::Severity::Critical,
                category: vulnerability::VulnCategory::Reentrancy,
                description: "Bad".to_string(),
                source_location: None,
                recommendation: "Fix".to_string(),
            })
            .collect();
        let score = compute_security_score(false, &None, &None, &many_vulns, &None, &None);
        assert_eq!(score, 0); // Clamped to 0
    }
}
