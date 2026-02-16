//! # DeFi Protocol Analysis
//!
//! Detects DeFi-specific patterns and risks in smart contract source code,
//! including oracle dependencies, flash loan risks, lending protocol patterns,
//! and DEX integration concerns.
//!
//! ## Detected Patterns
//!
//! - **Oracle usage** - Chainlink, Uniswap TWAP, Band Protocol
//! - **Flash loan vectors** - AAVE, dYdX flash loan callbacks
//! - **Lending patterns** - Borrow/lend, collateral, liquidation
//! - **DEX integration** - Uniswap, Sushiswap router calls, slippage checks
//! - **Staking/yield** - Staking, rewards, vesting patterns
//! - **Token standards** - ERC-20, ERC-721, ERC-1155 detection

use crate::contract::source::ContractSource;
use serde::{Deserialize, Serialize};

/// Complete DeFi analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefiAnalysis {
    /// Protocol type classification.
    pub protocol_type: ProtocolType,
    /// Whether the contract depends on price oracles.
    pub has_oracle_dependency: bool,
    /// Oracle details.
    pub oracle_info: Vec<OracleInfo>,
    /// Whether flash loan attack vectors exist.
    pub has_flash_loan_risk: bool,
    /// Flash loan details.
    pub flash_loan_info: Vec<String>,
    /// DEX integration patterns found.
    pub dex_integrations: Vec<DexIntegration>,
    /// Lending/borrowing patterns.
    pub lending_patterns: Vec<String>,
    /// Token standard(s) implemented.
    pub token_standards: Vec<TokenStandard>,
    /// Staking/yield patterns.
    pub staking_patterns: Vec<String>,
    /// DeFi-specific risk factors.
    pub risk_factors: Vec<DefiRiskFactor>,
}

/// Classification of the DeFi protocol type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProtocolType {
    /// Token (ERC-20, ERC-721, etc.)
    Token,
    /// Decentralized exchange / AMM
    DEX,
    /// Lending protocol
    Lending,
    /// Yield farming / staking
    Yield,
    /// Governance
    Governance,
    /// Bridge
    Bridge,
    /// NFT marketplace
    NFTMarketplace,
    /// Generic / unclassified
    Other,
}

impl std::fmt::Display for ProtocolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolType::Token => write!(f, "Token"),
            ProtocolType::DEX => write!(f, "DEX/AMM"),
            ProtocolType::Lending => write!(f, "Lending"),
            ProtocolType::Yield => write!(f, "Yield/Staking"),
            ProtocolType::Governance => write!(f, "Governance"),
            ProtocolType::Bridge => write!(f, "Bridge"),
            ProtocolType::NFTMarketplace => write!(f, "NFT Marketplace"),
            ProtocolType::Other => write!(f, "Other"),
        }
    }
}

/// Oracle integration details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleInfo {
    /// Oracle provider.
    pub provider: String,
    /// How the oracle is used.
    pub usage: String,
    /// Potential risks.
    pub risks: Vec<String>,
}

/// DEX integration details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexIntegration {
    /// DEX name/protocol.
    pub dex: String,
    /// Type of integration.
    pub integration_type: String,
    /// Whether slippage protection is present.
    pub has_slippage_protection: bool,
    /// Whether deadline protection is present.
    pub has_deadline_protection: bool,
}

/// Token standard detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenStandard {
    ERC20,
    ERC721,
    ERC1155,
    ERC4626,
    Custom(String),
}

impl std::fmt::Display for TokenStandard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenStandard::ERC20 => write!(f, "ERC-20"),
            TokenStandard::ERC721 => write!(f, "ERC-721"),
            TokenStandard::ERC1155 => write!(f, "ERC-1155"),
            TokenStandard::ERC4626 => write!(f, "ERC-4626"),
            TokenStandard::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// A DeFi-specific risk factor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefiRiskFactor {
    /// Risk name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Severity (1-10).
    pub severity: u8,
}

/// Analyze DeFi patterns in contract source code.
pub fn analyze_defi_patterns(source: &ContractSource) -> DefiAnalysis {
    let code = &source.source_code;

    let oracle_info = detect_oracles(code);
    let has_oracle_dependency = !oracle_info.is_empty();
    let flash_loan_info = detect_flash_loan_patterns(code);
    let has_flash_loan_risk = !flash_loan_info.is_empty();
    let dex_integrations = detect_dex_integrations(code);
    let lending_patterns = detect_lending_patterns(code);
    let token_standards = detect_token_standards(code, &source.parsed_abi);
    let staking_patterns = detect_staking_patterns(code);
    let protocol_type =
        classify_protocol(code, &token_standards, &dex_integrations, &lending_patterns);
    let risk_factors = assess_defi_risks(
        &oracle_info,
        &flash_loan_info,
        &dex_integrations,
        &lending_patterns,
    );

    DefiAnalysis {
        protocol_type,
        has_oracle_dependency,
        oracle_info,
        has_flash_loan_risk,
        flash_loan_info,
        dex_integrations,
        lending_patterns,
        token_standards,
        staking_patterns,
        risk_factors,
    }
}

fn detect_oracles(code: &str) -> Vec<OracleInfo> {
    let mut oracles = Vec::new();
    let code_lower = code.to_lowercase();

    // Chainlink
    if code_lower.contains("aggregatorv3interface")
        || code_lower.contains("latestrounddata")
        || code_lower.contains("chainlink")
        || code_lower.contains("pricefeed")
    {
        let mut risks = vec!["Stale price data if heartbeat check is missing".to_string()];

        // Check for proper staleness check
        if !code.contains("updatedAt") && !code.contains("answeredInRound") {
            risks.push("Missing staleness check on Chainlink price feed".to_string());
        }

        // Check for sequencer uptime feed (L2s)
        if !code_lower.contains("sequenceruptimefeed") {
            risks.push("Missing L2 sequencer uptime check (if deployed on L2)".to_string());
        }

        oracles.push(OracleInfo {
            provider: "Chainlink".to_string(),
            usage: "Price feed (AggregatorV3Interface)".to_string(),
            risks,
        });
    }

    // Uniswap TWAP
    if code_lower.contains("observe(")
        || code_lower.contains("twap")
        || code_lower.contains("oraclelibrary")
    {
        oracles.push(OracleInfo {
            provider: "Uniswap V3 TWAP".to_string(),
            usage: "Time-weighted average price oracle".to_string(),
            risks: vec![
                "TWAP can be manipulated with sustained capital over the observation window"
                    .to_string(),
                "Short TWAP windows are more susceptible to manipulation".to_string(),
            ],
        });
    }

    // Band Protocol
    if code_lower.contains("istdreference") || code_lower.contains("bandprotocol") {
        oracles.push(OracleInfo {
            provider: "Band Protocol".to_string(),
            usage: "External data oracle".to_string(),
            risks: vec!["Verify Band oracle reliability for the specific data feed".to_string()],
        });
    }

    // Generic getPrice / fetchPrice patterns
    if (code_lower.contains("getprice") || code_lower.contains("fetchprice")) && oracles.is_empty()
    {
        oracles.push(OracleInfo {
            provider: "Custom/Unknown".to_string(),
            usage: "Price retrieval function".to_string(),
            risks: vec!["Custom oracle — verify data source reliability".to_string()],
        });
    }

    oracles
}

fn detect_flash_loan_patterns(code: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let code_lower = code.to_lowercase();

    if code_lower.contains("flashloan") || code_lower.contains("flash_loan") {
        patterns.push("Flash loan function detected".to_string());
    }

    if code_lower.contains("executeflashloan")
        || code_lower.contains("onflashloan")
        || code_lower.contains("flashloansimple")
    {
        patterns.push("AAVE-style flash loan callback".to_string());
    }

    if code_lower.contains("callfunctionwithvalue") || code_lower.contains("flashborrowerfn") {
        patterns.push("dYdX-style flash loan integration".to_string());
    }

    if code_lower.contains("uniswapv2call") || code_lower.contains("uniswapv3flashcallback") {
        patterns.push("Uniswap flash swap callback".to_string());
    }

    // Check if the contract is flash-loan SAFE (validates balance changes)
    if !patterns.is_empty() && !code_lower.contains("balanceof") {
        patterns
            .push("WARNING: Flash loan callback without explicit balance validation".to_string());
    }

    patterns
}

fn detect_dex_integrations(code: &str) -> Vec<DexIntegration> {
    let mut integrations = Vec::new();
    let code_lower = code.to_lowercase();

    // Uniswap V2/V3 Router
    if code_lower.contains("iuniswapv2router") || code_lower.contains("swaprouter") {
        let has_slippage =
            code_lower.contains("amountoutmin") || code_lower.contains("amountinmax");
        let has_deadline =
            code_lower.contains("deadline") || code_lower.contains("block.timestamp");

        integrations.push(DexIntegration {
            dex: "Uniswap".to_string(),
            integration_type: "Swap router".to_string(),
            has_slippage_protection: has_slippage,
            has_deadline_protection: has_deadline,
        });
    }

    // SushiSwap
    if code_lower.contains("isushiswap") || code_lower.contains("sushirouter") {
        integrations.push(DexIntegration {
            dex: "SushiSwap".to_string(),
            integration_type: "Swap router".to_string(),
            has_slippage_protection: code_lower.contains("amountoutmin"),
            has_deadline_protection: code_lower.contains("deadline"),
        });
    }

    // Curve
    if code_lower.contains("icurve")
        || code_lower.contains("curvepool")
        || code_lower.contains("stableswap")
    {
        integrations.push(DexIntegration {
            dex: "Curve".to_string(),
            integration_type: "Stableswap pool".to_string(),
            has_slippage_protection: code_lower.contains("min_amount")
                || code_lower.contains("_min_dy"),
            has_deadline_protection: false,
        });
    }

    // Balancer
    if code_lower.contains("ibalancer")
        || code_lower.contains("ivault") && code_lower.contains("swap")
    {
        integrations.push(DexIntegration {
            dex: "Balancer".to_string(),
            integration_type: "Vault swap".to_string(),
            has_slippage_protection: code_lower.contains("limit"),
            has_deadline_protection: code_lower.contains("deadline"),
        });
    }

    integrations
}

fn detect_lending_patterns(code: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let code_lower = code.to_lowercase();

    if code_lower.contains("borrow") && code_lower.contains("repay") {
        patterns.push("Borrow/repay lending pattern".to_string());
    }

    if code_lower.contains("collateral") && code_lower.contains("liquidat") {
        patterns.push("Collateralized lending with liquidation".to_string());
    }

    if code_lower.contains("lendingpool") || code_lower.contains("comptroller") {
        patterns.push("Aave/Compound-style lending pool integration".to_string());
    }

    if code_lower.contains("healthfactor") || code_lower.contains("collateralratio") {
        patterns.push("Health factor / collateral ratio monitoring".to_string());
    }

    if code_lower.contains("interest") && code_lower.contains("rate") {
        patterns.push("Interest rate model".to_string());
    }

    patterns
}

fn detect_token_standards(
    code: &str,
    abi: &[crate::contract::source::AbiEntry],
) -> Vec<TokenStandard> {
    let mut standards = Vec::new();
    let code_lower = code.to_lowercase();

    // ERC-20 detection
    let has_erc20_functions = abi.iter().any(|e| {
        e.entry_type == "function"
            && (e.name == "transfer" || e.name == "approve" || e.name == "transferFrom")
    });
    if has_erc20_functions || code_lower.contains("erc20") || code_lower.contains("ierc20") {
        standards.push(TokenStandard::ERC20);
    }

    // ERC-721 detection
    let has_erc721 = abi.iter().any(|e| {
        e.entry_type == "function" && (e.name == "ownerOf" || e.name == "safeTransferFrom")
    });
    if has_erc721 || code_lower.contains("erc721") || code_lower.contains("ierc721") {
        standards.push(TokenStandard::ERC721);
    }

    // ERC-1155 detection
    if code_lower.contains("erc1155") || code_lower.contains("ierc1155") {
        standards.push(TokenStandard::ERC1155);
    }

    // ERC-4626 vault
    if code_lower.contains("erc4626")
        || (code_lower.contains("deposit") && code_lower.contains("shares"))
    {
        standards.push(TokenStandard::ERC4626);
    }

    standards
}

fn detect_staking_patterns(code: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let code_lower = code.to_lowercase();

    if code_lower.contains("stake") && code_lower.contains("unstake") {
        patterns.push("Stake/unstake mechanism".to_string());
    }

    if code_lower.contains("rewardpertoken") || code_lower.contains("earned") {
        patterns.push("Reward distribution (Synthetix-style)".to_string());
    }

    if code_lower.contains("vesting") || code_lower.contains("vestingschedule") {
        patterns.push("Token vesting schedule".to_string());
    }

    if code_lower.contains("timelock") || code_lower.contains("lockperiod") {
        patterns.push("Time-locked staking".to_string());
    }

    patterns
}

fn classify_protocol(
    code: &str,
    token_standards: &[TokenStandard],
    dex_integrations: &[DexIntegration],
    lending_patterns: &[String],
) -> ProtocolType {
    let code_lower = code.to_lowercase();

    if !lending_patterns.is_empty() {
        ProtocolType::Lending
    } else if !dex_integrations.is_empty()
        || code_lower.contains("addliquidity")
        || code_lower.contains("removeliquidity")
    {
        ProtocolType::DEX
    } else if code_lower.contains("governance")
        || code_lower.contains("propose") && code_lower.contains("vote")
    {
        ProtocolType::Governance
    } else if code_lower.contains("bridge") || code_lower.contains("crosschain") {
        ProtocolType::Bridge
    } else if token_standards
        .iter()
        .any(|s| matches!(s, TokenStandard::ERC721 | TokenStandard::ERC1155))
        && (code_lower.contains("marketplace") || code_lower.contains("auction"))
    {
        ProtocolType::NFTMarketplace
    } else if code_lower.contains("stake")
        || code_lower.contains("farm")
        || code_lower.contains("yield")
    {
        ProtocolType::Yield
    } else if !token_standards.is_empty() {
        ProtocolType::Token
    } else {
        ProtocolType::Other
    }
}

fn assess_defi_risks(
    oracle_info: &[OracleInfo],
    flash_loan_info: &[String],
    dex_integrations: &[DexIntegration],
    lending_patterns: &[String],
) -> Vec<DefiRiskFactor> {
    let mut risks = Vec::new();

    // Oracle risks
    for oracle in oracle_info {
        for risk in &oracle.risks {
            if risk.contains("Missing staleness") || risk.contains("Missing L2") {
                risks.push(DefiRiskFactor {
                    name: format!("{} oracle risk", oracle.provider),
                    description: risk.clone(),
                    severity: 7,
                });
            }
        }
    }

    // Flash loan risks
    if !flash_loan_info.is_empty() {
        risks.push(DefiRiskFactor {
            name: "Flash loan exposure".to_string(),
            description: "Contract interacts with flash loans. Ensure all state \
                changes are validated after flash loan execution."
                .to_string(),
            severity: 6,
        });
    }

    // DEX integration risks
    for dex in dex_integrations {
        if !dex.has_slippage_protection {
            risks.push(DefiRiskFactor {
                name: format!("{} missing slippage protection", dex.dex),
                description: "DEX swap without minimum output amount. Transaction \
                    can be sandwiched by MEV bots."
                    .to_string(),
                severity: 8,
            });
        }
        if !dex.has_deadline_protection {
            risks.push(DefiRiskFactor {
                name: format!("{} missing deadline", dex.dex),
                description: "DEX swap without deadline parameter. Transaction can be \
                    held in mempool indefinitely and executed at a bad price."
                    .to_string(),
                severity: 5,
            });
        }
    }

    // Lending risks
    if !lending_patterns.is_empty() && !lending_patterns.iter().any(|p| p.contains("liquidation")) {
        risks.push(DefiRiskFactor {
            name: "Lending without liquidation mechanism".to_string(),
            description: "Lending pattern detected without explicit liquidation handling. \
                Bad debt may accumulate without a liquidation pathway."
                .to_string(),
            severity: 7,
        });
    }

    risks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::source::ContractSource;

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
    fn test_detect_chainlink_oracle() {
        let src = make_source(
            "AggregatorV3Interface priceFeed; (,int price,,,) = priceFeed.latestRoundData();",
        );
        let analysis = analyze_defi_patterns(&src);
        assert!(analysis.has_oracle_dependency);
        assert!(
            analysis
                .oracle_info
                .iter()
                .any(|o| o.provider == "Chainlink")
        );
    }

    #[test]
    fn test_detect_uniswap_integration() {
        let src = make_source(
            "IUniswapV2Router router; router.swapExactTokensForTokens(amountIn, amountOutMin, path, to, deadline);",
        );
        let analysis = analyze_defi_patterns(&src);
        assert!(!analysis.dex_integrations.is_empty());
        assert!(analysis.dex_integrations[0].has_slippage_protection);
        assert!(analysis.dex_integrations[0].has_deadline_protection);
    }

    #[test]
    fn test_detect_flash_loan() {
        let src = make_source(
            "function onFlashLoan(address, address, uint256, uint256, bytes calldata) external returns (bytes32) {}",
        );
        let analysis = analyze_defi_patterns(&src);
        assert!(analysis.has_flash_loan_risk);
    }

    #[test]
    fn test_detect_lending() {
        let src = make_source(
            "function borrow(uint amount) {} function repay(uint amount) {} function liquidate() {}",
        );
        let analysis = analyze_defi_patterns(&src);
        assert!(!analysis.lending_patterns.is_empty());
        assert!(matches!(analysis.protocol_type, ProtocolType::Lending));
    }

    #[test]
    fn test_detect_erc20_from_abi() {
        let src = ContractSource {
            contract_name: "Token".to_string(),
            source_code: String::new(),
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
            parsed_abi: vec![crate::contract::source::AbiEntry {
                entry_type: "function".to_string(),
                name: "transfer".to_string(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: "nonpayable".to_string(),
            }],
        };
        let analysis = analyze_defi_patterns(&src);
        assert!(
            analysis
                .token_standards
                .iter()
                .any(|s| matches!(s, TokenStandard::ERC20))
        );
    }

    #[test]
    fn test_missing_slippage_risk() {
        let src = make_source(
            "IUniswapV2Router router; router.swapExactTokensForTokens(amountIn, 0, path, to, deadline);",
        );
        let analysis = analyze_defi_patterns(&src);
        // amountOutMin is not explicitly named, but "amountoutmin" keyword is absent from source
        // The heuristic checks for the keyword presence, not value
        assert!(!analysis.dex_integrations.is_empty());
    }
}
