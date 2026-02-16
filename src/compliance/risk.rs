//! Risk Scoring Engine for Scope
//!
//! Provides compliance-grade risk analysis for blockchain addresses.
//! Aggregates data from multiple sources to produce comprehensive risk scores.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Risk level classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,      // 0-3
    Medium,   // 4-6
    High,     // 7-8
    Critical, // 9-10
}

impl RiskLevel {
    pub fn from_score(score: f32) -> Self {
        match score {
            s if s <= 3.0 => RiskLevel::Low,
            s if s <= 6.0 => RiskLevel::Medium,
            s if s <= 8.0 => RiskLevel::High,
            _ => RiskLevel::Critical,
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            RiskLevel::Low => "🟢",
            RiskLevel::Medium => "🟡",
            RiskLevel::High => "🔴",
            RiskLevel::Critical => "⚫",
        }
    }
}

/// Individual risk factor with weight and score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub name: String,
    pub category: RiskCategory,
    pub score: f32,  // 0-10
    pub weight: f32, // 0-1, contribution to final score
    pub description: String,
    pub evidence: Vec<String>,
}

/// Risk category for organizing factors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskCategory {
    Behavioral,  // Transaction patterns, velocity
    Association, // Connected to known bad addresses
    Source,      // Funds from suspicious sources
    Destination, // Funds to suspicious destinations
    Entity,      // Known entity (exchange, mixer, etc.)
    Sanctions,   // OFAC, sanctions lists
    Reputation,  // Community reports, scam databases
}

/// Comprehensive risk assessment for an address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub address: String,
    pub chain: String,
    pub overall_score: f32, // 0-10
    pub risk_level: RiskLevel,
    pub factors: Vec<RiskFactor>,
    pub assessed_at: DateTime<Utc>,
    pub recommendations: Vec<String>,
}

use super::datasource::{BlockchainDataClient, analyze_patterns};

/// Risk scoring engine configuration
#[derive(Debug)]
pub struct RiskEngine {
    /// Data client for fetching blockchain data
    data_client: Option<BlockchainDataClient>,
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskEngine {
    /// Create new risk engine without data sources (basic scoring only)
    pub fn new() -> Self {
        Self { data_client: None }
    }

    /// Create new risk engine with data sources for enhanced analysis
    pub fn with_data_client(client: BlockchainDataClient) -> Self {
        Self {
            data_client: Some(client),
        }
    }

    /// Assess risk for a single address
    pub async fn assess_address(
        &self,
        address: &str,
        chain: &str,
    ) -> anyhow::Result<RiskAssessment> {
        let mut factors = Vec::new();

        // 1. Behavioral Analysis (Transaction Patterns)
        if let Ok(factor) = self.analyze_behavior(address, chain).await {
            factors.push(factor);
        }

        // 2. Association Analysis (Connected Addresses)
        if let Ok(factor) = self.analyze_associations(address, chain).await {
            factors.push(factor);
        }

        // 3. Source Analysis (Where funds came from)
        if let Ok(factor) = self.analyze_sources(address, chain).await {
            factors.push(factor);
        }

        // 4. Entity Recognition (Known services)
        if let Ok(factor) = self.identify_entity(address, chain).await {
            factors.push(factor);
        }

        // Calculate weighted score
        let overall_score = self.calculate_weighted_score(&factors);
        let risk_level = RiskLevel::from_score(overall_score);

        // Generate recommendations
        let recommendations = self.generate_recommendations(&factors, risk_level);

        Ok(RiskAssessment {
            address: address.to_string(),
            chain: chain.to_string(),
            overall_score,
            risk_level,
            factors,
            assessed_at: Utc::now(),
            recommendations,
        })
    }

    /// Analyze transaction behavior patterns
    async fn analyze_behavior(&self, address: &str, chain: &str) -> anyhow::Result<RiskFactor> {
        let mut evidence = Vec::new();
        let mut score: f32 = 2.0; // Default low score

        // Fetch real transaction data if available
        if let Some(client) = &self.data_client {
            match client.get_transactions(address, chain).await {
                Ok(txs) => {
                    let analysis = analyze_patterns(&txs);

                    // Adjust score based on patterns
                    if analysis.structuring_detected {
                        score += 3.0;
                        evidence.push(
                            "Structuring pattern detected (amounts just under thresholds)"
                                .to_string(),
                        );
                    }

                    if analysis.round_number_pattern {
                        score += 1.5;
                        evidence.push("Round number pattern suggests automation".to_string());
                    }

                    if analysis.velocity_score > 10.0 {
                        score += 2.0;
                        evidence.push(format!(
                            "High transaction velocity: {:.1} tx/day",
                            analysis.velocity_score
                        ));
                    }

                    if analysis.unusual_hours > 0 {
                        score += 1.0;
                        evidence.push(format!(
                            "{} transactions during unusual hours",
                            analysis.unusual_hours
                        ));
                    }

                    evidence.push(format!(
                        "Analyzed {} transactions",
                        analysis.total_transactions
                    ));
                }
                Err(e) => {
                    evidence.push(format!("Could not fetch transaction data: {}", e));
                }
            }
        } else {
            evidence.push("No data client configured - using default scores".to_string());
        }

        // Ensure score stays in bounds
        score = score.clamp(0.0, 10.0);

        Ok(RiskFactor {
            name: "Behavioral Patterns".to_string(),
            category: RiskCategory::Behavioral,
            score,
            weight: 0.25,
            description: "Transaction velocity and pattern analysis".to_string(),
            evidence,
        })
    }

    /// Analyze associations with known addresses
    async fn analyze_associations(&self, address: &str, chain: &str) -> anyhow::Result<RiskFactor> {
        let mut evidence = Vec::new();
        let mut score: f32 = 1.5; // Default low score

        // Fetch transaction data to analyze connections
        if let Some(client) = &self.data_client {
            match client.get_transactions(address, chain).await {
                Ok(txs) => {
                    // Count unique counterparties
                    let mut counterparties = std::collections::HashSet::new();
                    for tx in &txs {
                        counterparties.insert(tx.from.clone());
                        counterparties.insert(tx.to.clone());
                    }
                    counterparties.remove(address);

                    evidence.push(format!(
                        "Found {} unique counterparties",
                        counterparties.len()
                    ));

                    // High number of counterparties can indicate mixing
                    if counterparties.len() > 100 {
                        score += 2.0;
                        evidence.push(
                            "High number of counterparties may indicate mixing service".to_string(),
                        );
                    }

                    // Check for self-transfers (looping)
                    let self_transfers = txs.iter().filter(|tx| tx.from == tx.to).count();
                    if self_transfers > 0 {
                        score += 1.0;
                        evidence.push(format!("{} self-transfers detected", self_transfers));
                    }
                }
                Err(e) => {
                    evidence.push(format!("Could not analyze associations: {}", e));
                }
            }
        } else {
            evidence.push("No data client configured - using default scores".to_string());
        }

        score = score.clamp(0.0, 10.0);

        Ok(RiskFactor {
            name: "Address Associations".to_string(),
            category: RiskCategory::Association,
            score,
            weight: 0.30,
            description: "Connections to known high-risk addresses".to_string(),
            evidence,
        })
    }

    /// Analyze source of funds
    async fn analyze_sources(&self, address: &str, chain: &str) -> anyhow::Result<RiskFactor> {
        let mut evidence = Vec::new();
        let mut score: f32 = 2.0; // Default low-medium score

        if let Some(client) = &self.data_client {
            match client.get_transactions(address, chain).await {
                Ok(txs) => {
                    // Analyze incoming transactions (where this address is the recipient)
                    let incoming: Vec<_> = txs
                        .iter()
                        .filter(|tx| tx.to.to_lowercase() == address.to_lowercase())
                        .collect();

                    evidence.push(format!("Analyzed {} incoming transactions", incoming.len()));

                    // Check for failed transactions
                    let failed = txs.iter().filter(|tx| tx.is_error == "1").count();
                    if failed > 0 {
                        score += 1.0;
                        evidence.push(format!("{} failed transactions detected", failed));
                    }

                    // Check for contract interactions (more complex, higher risk)
                    let contract_calls = txs
                        .iter()
                        .filter(|tx| !tx.contract_address.is_empty())
                        .count();
                    if contract_calls > 0 {
                        evidence.push(format!("{} contract interactions", contract_calls));
                    }
                }
                Err(e) => {
                    evidence.push(format!("Could not analyze sources: {}", e));
                }
            }
        } else {
            evidence.push("No data client configured - using default scores".to_string());
        }

        score = score.clamp(0.0, 10.0);

        Ok(RiskFactor {
            name: "Source of Funds".to_string(),
            category: RiskCategory::Source,
            score,
            weight: 0.25,
            description: "Origin analysis of incoming funds".to_string(),
            evidence,
        })
    }

    /// Identify if address belongs to known entity
    async fn identify_entity(&self, address: &str, _chain: &str) -> anyhow::Result<RiskFactor> {
        let mut evidence = Vec::new();
        let mut score: f32 = 2.0;

        // Check for known entity patterns
        // This would typically integrate with a database of known addresses

        // Placeholder: Check if address has code (is a contract)
        if let Some(client) = &self.data_client {
            // Try to get internal transactions - contracts often have these
            match client.get_internal_transactions(address).await {
                Ok(internal_txs) => {
                    if !internal_txs.is_empty() {
                        evidence.push(format!(
                            "Contract interactions detected: {} internal transactions",
                            internal_txs.len()
                        ));
                        score += 0.5; // Slight increase for being a contract
                    }
                }
                Err(_) => {
                    // Not necessarily an error - EOAs don't have internal transactions
                }
            }
        }

        // Known exchange addresses would be checked here
        evidence.push("Address not in known entity database (implement integration)".to_string());

        score = score.clamp(0.0, 10.0);

        Ok(RiskFactor {
            name: "Entity Identification".to_string(),
            category: RiskCategory::Entity,
            score,
            weight: 0.20,
            description: "Known entity classification".to_string(),
            evidence,
        })
    }

    /// Calculate weighted score from factors
    fn calculate_weighted_score(&self, factors: &[RiskFactor]) -> f32 {
        if factors.is_empty() {
            return 0.0;
        }

        let weighted_sum: f32 = factors.iter().map(|f| f.score * f.weight).sum();

        let total_weight: f32 = factors.iter().map(|f| f.weight).sum();

        if total_weight == 0.0 {
            return 0.0;
        }

        (weighted_sum / total_weight).clamp(0.0, 10.0)
    }

    /// Generate recommendations based on risk factors
    fn generate_recommendations(&self, factors: &[RiskFactor], level: RiskLevel) -> Vec<String> {
        let mut recommendations = Vec::new();

        match level {
            RiskLevel::Critical => {
                recommendations.push("Immediate investigation required".to_string());
                recommendations.push("Consider suspending transactions".to_string());
                recommendations.push("File SAR if applicable".to_string());
            }
            RiskLevel::High => {
                recommendations.push("Enhanced due diligence recommended".to_string());
                recommendations.push("Monitor transactions closely".to_string());
                recommendations.push("Verify source of funds".to_string());
            }
            RiskLevel::Medium => {
                recommendations.push("Standard due diligence".to_string());
                recommendations.push("Periodic re-assessment".to_string());
            }
            RiskLevel::Low => {
                recommendations.push("Standard monitoring".to_string());
            }
        }

        // Add factor-specific recommendations
        for factor in factors {
            if factor.score > 7.0 {
                recommendations.push(format!("Address {} concerns immediately", factor.name));
            }
        }

        recommendations
    }
}

// ============================================================================
// Enhanced Risk Detection (Contract-Aware)
// ============================================================================

/// Enhanced holder concentration analysis with Gini coefficient.
pub fn compute_gini_coefficient(percentages: &[f64]) -> f64 {
    if percentages.is_empty() {
        return 0.0;
    }
    let n = percentages.len() as f64;
    let mut sorted = percentages.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let sum: f64 = sorted.iter().sum();
    if sum == 0.0 {
        return 0.0;
    }

    let mut numerator = 0.0;
    for (i, &val) in sorted.iter().enumerate() {
        numerator += (2.0 * (i as f64 + 1.0) - n - 1.0) * val;
    }

    (numerator / (n * sum)).clamp(0.0, 1.0)
}

/// Detect rugpull indicators from contract analysis.
///
/// Returns a list of rugpull risk indicators with severity (0-10).
pub fn detect_rugpull_indicators(
    contract_analysis: Option<&crate::contract::ContractAnalysis>,
    token_analytics: Option<&crate::chains::TokenAnalytics>,
) -> Vec<RiskFactor> {
    let mut factors = Vec::new();

    if let Some(ca) = contract_analysis {
        let mut evidence = Vec::new();
        let mut score: f32 = 0.0;

        // Check for owner can mint
        if let Some(ac) = &ca.access_control {
            for pf in &ac.privileged_functions {
                match pf.name.to_lowercase().as_str() {
                    n if n.contains("mint") => {
                        score += 3.0;
                        evidence.push(format!("Owner can mint tokens: {}", pf.name));
                    }
                    n if n.contains("pause") => {
                        score += 2.0;
                        evidence.push(format!("Owner can pause transfers: {}", pf.name));
                    }
                    n if n.contains("blacklist") => {
                        score += 2.5;
                        evidence.push(format!("Owner can blacklist addresses: {}", pf.name));
                    }
                    n if n.contains("setfee") || n.contains("settax") => {
                        score += 2.0;
                        evidence.push(format!("Owner can change fees: {}", pf.name));
                    }
                    _ => {}
                }
            }

            // Renounced ownership reduces risk
            if ac.has_renounced_ownership {
                score -= 3.0;
                evidence.push("Ownership has been renounced".to_string());
            }

            // tx.origin usage is a red flag
            if ac.uses_tx_origin {
                score += 2.0;
                evidence.push("Uses tx.origin for authorization".to_string());
            }
        }

        // Unverified source is a major red flag
        if !ca.is_verified {
            score += 3.0;
            evidence.push("Contract source code is not verified".to_string());
        }

        score = score.clamp(0.0, 10.0);

        if !evidence.is_empty() {
            factors.push(RiskFactor {
                name: "Rugpull Risk (Contract)".to_string(),
                category: RiskCategory::Reputation,
                score,
                weight: 0.35,
                description: "Contract-level rugpull indicators".to_string(),
                evidence,
            });
        }
    }

    if let Some(ta) = token_analytics {
        let mut evidence = Vec::new();
        let mut score: f32 = 0.0;

        // Extreme concentration
        if let Some(top10) = ta.top_10_concentration {
            if top10 > 80.0 {
                score += 4.0;
                evidence.push(format!("Top 10 holders control {:.1}% of supply", top10));
            } else if top10 > 50.0 {
                score += 2.0;
                evidence.push(format!("Top 10 holders control {:.1}% of supply", top10));
            }
        }

        // Very new token
        if let Some(age) = ta.token_age_hours {
            if age < 24.0 {
                score += 3.0;
                evidence.push(format!("Token is very new ({:.0}h old)", age));
            } else if age < 72.0 {
                score += 1.5;
                evidence.push(format!("Token is recently created ({:.0}h old)", age));
            }
        }

        // No sells (honeypot indicator)
        if ta.total_sells_24h == 0 && ta.total_buys_24h > 10 {
            score += 4.0;
            evidence.push(format!(
                "No sells in 24h with {} buys (potential honeypot)",
                ta.total_buys_24h
            ));
        }

        // Low liquidity
        if ta.liquidity_usd < 10_000.0 && ta.liquidity_usd > 0.0 {
            score += 2.0;
            evidence.push(format!("Very low liquidity: ${:.0}", ta.liquidity_usd));
        }

        score = score.clamp(0.0, 10.0);

        if !evidence.is_empty() {
            factors.push(RiskFactor {
                name: "Rugpull Risk (Token)".to_string(),
                category: RiskCategory::Reputation,
                score,
                weight: 0.35,
                description: "Token-level rugpull indicators".to_string(),
                evidence,
            });
        }
    }

    factors
}

/// Detect whale activity from transaction data.
///
/// Returns whale-related risk factors based on large transactions
/// and holder concentration.
pub fn detect_whale_activity(
    transactions: &[crate::chains::Transaction],
    avg_tx_value_usd: f64,
    whale_threshold_usd: f64,
) -> RiskFactor {
    let mut evidence = Vec::new();
    let mut score: f32 = 0.0;

    let large_txs: Vec<_> = transactions
        .iter()
        .filter(|tx| {
            // Parse value if possible (rough heuristic)
            tx.value
                .parse::<f64>()
                .map(|v| v > whale_threshold_usd)
                .unwrap_or(false)
        })
        .collect();

    if !large_txs.is_empty() {
        let pct = (large_txs.len() as f64 / transactions.len() as f64) * 100.0;
        score += (pct / 10.0) as f32;
        evidence.push(format!(
            "{} whale-sized transactions ({:.1}% of total)",
            large_txs.len(),
            pct
        ));
    }

    if avg_tx_value_usd > whale_threshold_usd * 0.5 {
        score += 2.0;
        evidence.push(format!(
            "Average transaction size ${:.0} is near whale threshold ${:.0}",
            avg_tx_value_usd, whale_threshold_usd
        ));
    }

    if evidence.is_empty() {
        evidence.push("No significant whale activity detected".to_string());
    }

    score = score.clamp(0.0, 10.0);

    RiskFactor {
        name: "Whale Activity".to_string(),
        category: RiskCategory::Behavioral,
        score,
        weight: 0.15,
        description: "Large transaction and whale holder analysis".to_string(),
        evidence,
    }
}

/// Detect timelock patterns from contract analysis.
pub fn detect_timelock(
    contract_analysis: &crate::contract::ContractAnalysis,
) -> Option<RiskFactor> {
    let src = contract_analysis.source_info.as_ref()?;
    let code_lower = src.source_code.to_lowercase();

    let mut evidence = Vec::new();
    let mut has_timelock = false;

    if code_lower.contains("timelockcontroller") || code_lower.contains("timelock") {
        has_timelock = true;
        evidence.push("TimelockController pattern detected".to_string());
    }

    if code_lower.contains("delay")
        && code_lower.contains("queue")
        && code_lower.contains("execute")
    {
        has_timelock = true;
        evidence.push("Queue/delay/execute governance pattern found".to_string());
    }

    if code_lower.contains("mindelay") || code_lower.contains("minimum_delay") {
        evidence.push("Minimum delay parameter found".to_string());
    }

    // Timelock presence reduces risk
    let score = if has_timelock { 2.0 } else { 5.0 };

    Some(RiskFactor {
        name: "Timelock".to_string(),
        category: RiskCategory::Entity,
        score,
        weight: 0.10,
        description: if has_timelock {
            "Timelock governance detected (reduces admin risk)".to_string()
        } else {
            "No timelock governance detected for admin operations".to_string()
        },
        evidence,
    })
}

/// Detect multisig patterns from contract analysis and bytecode.
pub fn detect_multisig(
    contract_analysis: &crate::contract::ContractAnalysis,
) -> Option<RiskFactor> {
    let mut evidence = Vec::new();
    let mut is_multisig = false;

    // Check source code
    if let Some(src) = &contract_analysis.source_info {
        let code_lower = src.source_code.to_lowercase();

        if code_lower.contains("gnosis")
            || code_lower.contains("safe") && code_lower.contains("multisig")
        {
            is_multisig = true;
            evidence.push("Gnosis Safe / multisig wallet pattern detected".to_string());
        }

        if code_lower.contains("threshold") && code_lower.contains("owners") {
            is_multisig = true;
            evidence.push("Multi-owner threshold pattern (M-of-N signatures)".to_string());
        }

        if code_lower.contains("confirmtransaction") && code_lower.contains("executetransaction") {
            is_multisig = true;
            evidence.push("Confirm/execute transaction pattern (multisig workflow)".to_string());
        }
    }

    // Check admin address (if known, could check if it's a multisig)
    if let Some(proxy) = &contract_analysis.proxy_info
        && let Some(admin) = &proxy.admin_address
    {
        evidence.push(format!(
            "Proxy admin address: {} (verify if multisig)",
            admin
        ));
    }

    let score = if is_multisig { 2.0 } else { 4.0 };

    Some(RiskFactor {
        name: "Multisig Governance".to_string(),
        category: RiskCategory::Entity,
        score,
        weight: 0.10,
        description: if is_multisig {
            "Multisig governance detected (reduces single-key risk)".to_string()
        } else {
            "No multisig governance detected".to_string()
        },
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::datasource;

    #[test]
    fn test_risk_level_from_score() {
        assert!(matches!(RiskLevel::from_score(2.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(5.0), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(7.5), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(9.0), RiskLevel::Critical));
    }

    #[test]
    fn test_risk_level_boundaries() {
        assert!(matches!(RiskLevel::from_score(0.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(3.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(3.01), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(6.0), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(6.01), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(8.0), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(8.01), RiskLevel::Critical));
        assert!(matches!(RiskLevel::from_score(10.0), RiskLevel::Critical));
    }

    #[test]
    fn test_risk_level_emojis() {
        assert_eq!(RiskLevel::Low.emoji(), "🟢");
        assert_eq!(RiskLevel::Medium.emoji(), "🟡");
        assert_eq!(RiskLevel::High.emoji(), "🔴");
        assert_eq!(RiskLevel::Critical.emoji(), "⚫");
    }

    #[test]
    fn test_weighted_score_calculation() {
        let engine = RiskEngine::new();
        let factors = vec![
            RiskFactor {
                name: "Test1".to_string(),
                category: RiskCategory::Behavioral,
                score: 5.0,
                weight: 0.5,
                description: "Test".to_string(),
                evidence: vec![],
            },
            RiskFactor {
                name: "Test2".to_string(),
                category: RiskCategory::Association,
                score: 3.0,
                weight: 0.5,
                description: "Test".to_string(),
                evidence: vec![],
            },
        ];

        // (5.0 * 0.5 + 3.0 * 0.5) / (0.5 + 0.5) = 4.0
        let score = engine.calculate_weighted_score(&factors);
        assert!((score - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_weighted_score_empty_factors() {
        let engine = RiskEngine::new();
        let score = engine.calculate_weighted_score(&[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_weighted_score_zero_weight() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "Test".to_string(),
            category: RiskCategory::Behavioral,
            score: 5.0,
            weight: 0.0,
            description: "Test".to_string(),
            evidence: vec![],
        }];
        let score = engine.calculate_weighted_score(&factors);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_weighted_score_clamped() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "High".to_string(),
            category: RiskCategory::Behavioral,
            score: 15.0,
            weight: 1.0,
            description: "Test".to_string(),
            evidence: vec![],
        }];
        let score = engine.calculate_weighted_score(&factors);
        assert_eq!(score, 10.0);
    }

    #[test]
    fn test_recommendations_by_level() {
        let engine = RiskEngine::new();
        let factors = vec![];

        let low_recs = engine.generate_recommendations(&factors, RiskLevel::Low);
        assert!(low_recs.iter().any(|r| r.contains("Standard monitoring")));

        let med_recs = engine.generate_recommendations(&factors, RiskLevel::Medium);
        assert!(
            med_recs
                .iter()
                .any(|r| r.contains("Standard due diligence"))
        );

        let high_recs = engine.generate_recommendations(&factors, RiskLevel::High);
        assert!(
            high_recs
                .iter()
                .any(|r| r.contains("Enhanced due diligence"))
        );

        let crit_recs = engine.generate_recommendations(&factors, RiskLevel::Critical);
        assert!(
            crit_recs
                .iter()
                .any(|r| r.contains("Immediate investigation"))
        );
    }

    #[test]
    fn test_recommendations_high_score_factors() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "CriticalIssue".to_string(),
            category: RiskCategory::Behavioral,
            score: 8.5,
            weight: 1.0,
            description: "Critical issue".to_string(),
            evidence: vec!["Evidence".to_string()],
        }];

        let recs = engine.generate_recommendations(&factors, RiskLevel::Low);
        assert!(recs.iter().any(|r| r.contains("CriticalIssue")));
    }

    #[test]
    fn test_risk_factor_creation() {
        let factor = RiskFactor {
            name: "TestFactor".to_string(),
            category: RiskCategory::Entity,
            score: 7.5,
            weight: 0.25,
            description: "Test description".to_string(),
            evidence: vec!["Evidence 1".to_string(), "Evidence 2".to_string()],
        };

        assert_eq!(factor.name, "TestFactor");
        assert!(matches!(factor.category, RiskCategory::Entity));
        assert_eq!(factor.score, 7.5);
        assert_eq!(factor.weight, 0.25);
        assert_eq!(factor.evidence.len(), 2);
    }

    #[test]
    fn test_all_risk_categories() {
        let _categories = [
            RiskCategory::Behavioral,
            RiskCategory::Association,
            RiskCategory::Source,
            RiskCategory::Destination,
            RiskCategory::Entity,
            RiskCategory::Sanctions,
            RiskCategory::Reputation,
        ];
    }

    #[tokio::test]
    async fn test_risk_engine_creation() {
        let engine = RiskEngine::new();
        let assessment = engine
            .assess_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();

        assert_eq!(
            assessment.address,
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
        );
        assert_eq!(assessment.chain, "ethereum");
        assert!(assessment.overall_score >= 0.0 && assessment.overall_score <= 10.0);
        assert!(!assessment.factors.is_empty());
        assert!(!assessment.recommendations.is_empty());
    }

    #[tokio::test]
    async fn test_risk_assessment_different_addresses() {
        let engine = RiskEngine::new();

        let addresses = vec![
            ("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum"),
            ("0x0000000000000000000000000000000000000000", "ethereum"),
        ];

        for (addr, chain) in addresses {
            let assessment = engine.assess_address(addr, chain).await.unwrap();
            assert_eq!(assessment.address, addr);
            assert_eq!(assessment.chain, chain);
        }
    }

    #[test]
    fn test_risk_engine_default() {
        let engine = RiskEngine::default();
        // Should create engine without data client
        let score = engine.calculate_weighted_score(&[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_risk_engine_with_data_client() {
        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::new(sources);
        let _engine = RiskEngine::with_data_client(client);
        // Just verify it creates without panicking
    }

    #[tokio::test]
    async fn test_assess_address_has_all_factors() {
        let engine = RiskEngine::new();
        let assessment = engine.assess_address("0xtest", "ethereum").await.unwrap();

        // Without a data client, should have 4 factors (behavior, association, source, entity)
        assert_eq!(assessment.factors.len(), 4);

        let categories: Vec<_> = assessment.factors.iter().map(|f| f.category).collect();
        assert!(categories.contains(&RiskCategory::Behavioral));
        assert!(categories.contains(&RiskCategory::Association));
        assert!(categories.contains(&RiskCategory::Source));
        assert!(categories.contains(&RiskCategory::Entity));
    }

    #[tokio::test]
    async fn test_assess_address_factors_have_evidence() {
        let engine = RiskEngine::new();
        let assessment = engine.assess_address("0xtest", "ethereum").await.unwrap();

        for factor in &assessment.factors {
            assert!(
                !factor.evidence.is_empty(),
                "Factor {} has no evidence",
                factor.name
            );
            // Without data client, evidence should mention "No data client configured"
            assert!(
                factor
                    .evidence
                    .iter()
                    .any(|e| e.contains("No data client configured")
                        || e.contains("not in known entity")),
                "Factor {} doesn't have expected evidence: {:?}",
                factor.name,
                factor.evidence
            );
        }
    }

    #[tokio::test]
    async fn test_assess_address_score_in_bounds() {
        let engine = RiskEngine::new();
        let assessment = engine.assess_address("0xtest", "ethereum").await.unwrap();

        assert!(assessment.overall_score >= 0.0);
        assert!(assessment.overall_score <= 10.0);

        for factor in &assessment.factors {
            assert!(factor.score >= 0.0);
            assert!(factor.score <= 10.0);
            assert!(factor.weight >= 0.0);
            assert!(factor.weight <= 1.0);
        }
    }

    #[test]
    fn test_risk_assessment_serialization() {
        let assessment = RiskAssessment {
            address: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            overall_score: 3.5,
            risk_level: RiskLevel::Medium,
            factors: vec![],
            assessed_at: Utc::now(),
            recommendations: vec!["Test recommendation".to_string()],
        };

        let json = serde_json::to_string(&assessment).unwrap();
        assert!(json.contains("0xtest"));
        assert!(json.contains("ethereum"));
        assert!(json.contains("Medium"));

        let deserialized: RiskAssessment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.address, "0xtest");
        assert_eq!(deserialized.overall_score, 3.5);
    }

    #[test]
    fn test_risk_factor_serialization() {
        let factor = RiskFactor {
            name: "Test".to_string(),
            category: RiskCategory::Behavioral,
            score: 5.0,
            weight: 0.25,
            description: "Test factor".to_string(),
            evidence: vec!["Evidence 1".to_string()],
        };

        let json = serde_json::to_string(&factor).unwrap();
        assert!(json.contains("Behavioral"));

        let deserialized: RiskFactor = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "Test");
        assert_eq!(deserialized.score, 5.0);
    }

    #[test]
    fn test_recommendations_critical_includes_sar() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::Critical);
        assert!(recs.iter().any(|r| r.contains("SAR")));
        assert!(recs.iter().any(|r| r.contains("suspending")));
    }

    #[test]
    fn test_recommendations_high_includes_verify_source() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::High);
        assert!(recs.iter().any(|r| r.contains("Verify source")));
    }

    #[test]
    fn test_recommendations_medium_includes_reassessment() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::Medium);
        assert!(recs.iter().any(|r| r.contains("re-assessment")));
    }

    #[test]
    fn test_weighted_score_single_factor() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "Single".to_string(),
            category: RiskCategory::Source,
            score: 7.0,
            weight: 1.0,
            description: "Test".to_string(),
            evidence: vec![],
        }];
        let score = engine.calculate_weighted_score(&factors);
        assert!((score - 7.0).abs() < 0.01);
    }

    fn make_test_tx(timestamp: &str, value_eth: &str) -> datasource::EtherscanTransaction {
        let value_wei = (value_eth.parse::<f64>().unwrap() * 1e18) as u64;
        datasource::EtherscanTransaction {
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
    fn test_pattern_analysis_no_structuring() {
        // Normal amounts, not just under thresholds
        let txs = vec![
            make_test_tx("1609459200", "1.5"),
            make_test_tx("1609459300", "2.3"),
            make_test_tx("1609459400", "0.7"),
        ];

        let analysis = analyze_patterns(&txs);
        assert!(!analysis.structuring_detected);
    }

    #[test]
    fn test_pattern_analysis_no_round_numbers() {
        let txs = vec![
            make_test_tx("1609459200", "1.234"),
            make_test_tx("1609459300", "0.567"),
            make_test_tx("1609459400", "3.891"),
        ];

        let analysis = analyze_patterns(&txs);
        assert!(!analysis.round_number_pattern);
    }

    #[test]
    fn test_pattern_analysis_single_tx() {
        let txs = vec![make_test_tx("1609459200", "1.0")];

        let analysis = analyze_patterns(&txs);
        assert_eq!(analysis.total_transactions, 1);
        // With a single timestamp, velocity can't be computed
        assert_eq!(analysis.velocity_score, 0.0);
    }

    #[tokio::test]
    async fn test_assess_address_generates_all_factors() {
        let engine = RiskEngine::new();
        let assessment = engine
            .assess_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();
        // Should have 4 risk factors (behavior, associations, sources, entity)
        assert_eq!(assessment.factors.len(), 4);
        // Check factor names
        let factor_names: Vec<&str> = assessment.factors.iter().map(|f| f.name.as_str()).collect();
        assert!(factor_names.contains(&"Behavioral Patterns"));
        assert!(factor_names.contains(&"Address Associations"));
        assert!(factor_names.contains(&"Source of Funds"));
        assert!(factor_names.contains(&"Entity Identification"));
    }

    #[test]
    fn test_risk_assessment_json_roundtrip() {
        let assessment = RiskAssessment {
            address: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            overall_score: 35.0,
            risk_level: RiskLevel::Medium,
            factors: vec![RiskFactor {
                name: "Test Factor".to_string(),
                category: RiskCategory::Behavioral,
                score: 30.0,
                weight: 0.25,
                description: "test details".to_string(),
                evidence: vec!["evidence1".to_string()],
            }],
            recommendations: vec!["recommendation".to_string()],
            assessed_at: Utc::now(),
        };
        let json = serde_json::to_string(&assessment).unwrap();
        let deserialized: RiskAssessment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.address, "0xtest");
        assert_eq!(deserialized.overall_score, 35.0);
        assert_eq!(deserialized.factors.len(), 1);
    }

    #[test]
    fn test_generate_recommendations_low_risk() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::Low);
        assert!(!recs.is_empty());
        // Low risk should have standard monitoring recommendation
        assert!(recs.iter().any(|r| r.contains("Standard monitoring")));
    }

    #[test]
    fn test_generate_recommendations_high_risk() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "Behavioral Patterns".to_string(),
            category: RiskCategory::Behavioral,
            score: 80.0,
            weight: 0.3,
            description: "concerning".to_string(),
            evidence: vec!["High velocity".to_string()],
        }];
        let recs = engine.generate_recommendations(&factors, RiskLevel::High);
        assert!(!recs.is_empty());
    }

    #[test]
    fn test_calculate_weighted_score_empty() {
        let engine = RiskEngine::new();
        let score = engine.calculate_weighted_score(&[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_analyze_patterns_structuring() {
        // Create transactions with values just under 10000 (structuring pattern)
        let txs: Vec<datasource::EtherscanTransaction> = (0..5)
            .map(|i| {
                let mut tx = make_test_tx(&format!("{}", 1700000000 + i * 3600), "9.5");
                tx.value = format!(
                    "{}",
                    (9500 + i * 100) as u128 * 1_000_000_000_000_000_000u128
                );
                tx
            })
            .collect();
        let analysis = analyze_patterns(&txs);
        assert_eq!(analysis.total_transactions, 5);
    }

    #[test]
    fn test_analyze_patterns_round_numbers() {
        // Create transactions with round ETH values
        let txs: Vec<datasource::EtherscanTransaction> = (0..10)
            .map(|i| {
                let mut tx = make_test_tx(&format!("{}", 1700000000 + i * 3600), "1.0");
                // 1 ETH = 1e18 wei, 10 ETH = 10e18 wei, etc.
                tx.value = format!("{}", 10u128.pow(18) * (i + 1) as u128);
                tx
            })
            .collect();
        let analysis = analyze_patterns(&txs);
        assert!(analysis.round_number_pattern);
    }

    #[test]
    fn test_analyze_patterns_high_velocity() {
        // Create many transactions spread over 2 days (high velocity)
        // Velocity = tx_count / days; 100 txs over 2 days = 50 tx/day
        let txs: Vec<datasource::EtherscanTransaction> = (0..100)
            .map(|i| {
                make_test_tx(&format!("{}", 1700000000 + i * 1800), "0.1") // 100 txs over ~2 days
            })
            .collect();
        let analysis = analyze_patterns(&txs);
        assert!(analysis.velocity_score > 1.0); // More than 1 tx per day
    }

    fn mock_etherscan_tx_response(txs: &[datasource::EtherscanTransaction]) -> String {
        let result_json = serde_json::to_string(txs).unwrap();
        format!(
            r#"{{"status":"1","message":"OK","result":{}}}"#,
            result_json
        )
    }

    #[tokio::test]
    async fn test_risk_engine_with_data_client_assess() {
        let mut server = mockito::Server::new_async().await;

        // Create test transactions with various patterns
        let txs: Vec<datasource::EtherscanTransaction> = (0..20)
            .map(|i| {
                let mut tx = make_test_tx(&format!("{}", 1700000000 + i * 3600), "1.0");
                tx.from = if i % 2 == 0 {
                    "0xSender".to_string()
                } else {
                    "0xAddr".to_string()
                };
                tx.to = if i % 2 == 0 {
                    "0xAddr".to_string()
                } else {
                    format!("0xRecipient{}", i)
                };
                tx.is_error = if i == 5 {
                    "1".to_string()
                } else {
                    "0".to_string()
                };
                tx.contract_address = if i == 10 {
                    "0xContract".to_string()
                } else {
                    String::new()
                };
                tx
            })
            .collect();

        let body = mock_etherscan_tx_response(&txs);
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .expect_at_least(1)
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);
        let assessment = engine.assess_address("0xAddr", "ethereum").await.unwrap();

        assert_eq!(assessment.factors.len(), 4);
        assert!(assessment.overall_score > 0.0);
        assert!(!assessment.recommendations.is_empty());

        // Behavioral factor should have evidence about analyzed transactions
        let behavior = assessment
            .factors
            .iter()
            .find(|f| f.name == "Behavioral Patterns")
            .unwrap();
        assert!(behavior.evidence.iter().any(|e| e.contains("Analyzed")));

        // Association factor should have counterparty evidence
        let assoc = assessment
            .factors
            .iter()
            .find(|f| f.name == "Address Associations")
            .unwrap();
        assert!(assoc.evidence.iter().any(|e| e.contains("counterpart")));

        // Source factor should mention incoming transactions
        let source = assessment
            .factors
            .iter()
            .find(|f| f.name == "Source of Funds")
            .unwrap();
        assert!(source.evidence.iter().any(|e| e.contains("incoming")));
    }

    #[tokio::test]
    async fn test_risk_engine_with_data_client_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":null}"#)
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);
        let assessment = engine.assess_address("0xAddr", "ethereum").await.unwrap();

        // Should still produce an assessment, but with error evidence
        assert_eq!(assessment.factors.len(), 4);
        // Behavior factor should mention the error
        let behavior = assessment
            .factors
            .iter()
            .find(|f| f.name == "Behavioral Patterns")
            .unwrap();
        assert!(
            behavior
                .evidence
                .iter()
                .any(|e| e.contains("Could not fetch"))
        );
    }

    #[tokio::test]
    async fn test_risk_engine_with_data_client_self_transfers() {
        let mut server = mockito::Server::new_async().await;

        // Create self-transfers (from == to)
        let mut txs = Vec::new();
        for i in 0..5 {
            let mut tx = make_test_tx(&format!("{}", 1700000000 + i * 3600), "1.0");
            tx.from = "0xAddr".to_string();
            tx.to = "0xAddr".to_string(); // self-transfer
            txs.push(tx);
        }

        let body = mock_etherscan_tx_response(&txs);
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);
        let assessment = engine.assess_address("0xAddr", "ethereum").await.unwrap();

        // Association factor should mention self-transfers
        let assoc = assessment
            .factors
            .iter()
            .find(|f| f.name == "Address Associations")
            .unwrap();
        assert!(assoc.evidence.iter().any(|e| e.contains("self-transfer")));
    }

    #[test]
    fn test_generate_recommendations_critical() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "Behavioral Patterns".to_string(),
            category: RiskCategory::Behavioral,
            score: 9.0,
            weight: 0.25,
            description: "test".to_string(),
            evidence: vec![],
        }];
        let recs = engine.generate_recommendations(&factors, RiskLevel::Critical);
        assert!(recs.iter().any(|r| r.contains("Immediate investigation")));
        assert!(recs.iter().any(|r| r.contains("SAR")));
    }

    #[test]
    fn test_generate_recommendations_high() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::High);
        assert!(recs.iter().any(|r| r.contains("Enhanced due diligence")));
    }

    #[test]
    fn test_generate_recommendations_medium() {
        let engine = RiskEngine::new();
        let recs = engine.generate_recommendations(&[], RiskLevel::Medium);
        assert!(recs.iter().any(|r| r.contains("Standard due diligence")));
    }

    #[test]
    fn test_generate_recommendations_with_high_score_factor() {
        let engine = RiskEngine::new();
        let factors = vec![RiskFactor {
            name: "Test Factor".to_string(),
            category: RiskCategory::Behavioral,
            score: 8.5,
            weight: 0.25,
            description: "test".to_string(),
            evidence: vec![],
        }];
        let recs = engine.generate_recommendations(&factors, RiskLevel::Low);
        assert!(
            recs.iter()
                .any(|r| r.contains("Address Test Factor concerns"))
        );
    }

    // ========================================================================
    // Tests with data client for pattern analysis branches
    // ========================================================================

    fn mock_etherscan_json_response(txs: &[serde_json::Value]) -> String {
        serde_json::json!({
            "status": "1",
            "message": "OK",
            "result": txs
        })
        .to_string()
    }

    fn make_tx_with_idx(
        idx: u64,
        from: &str,
        to: &str,
        value: &str,
        timestamp: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "hash": format!("0x{:064x}", idx),
            "from": from,
            "to": to,
            "value": value,
            "timeStamp": timestamp,
            "blockNumber": "18000000",
            "gasUsed": "21000",
            "gasPrice": "50000000000",
            "isError": "0",
            "input": "0x"
        })
    }

    #[tokio::test]
    async fn test_risk_engine_with_client_structuring_pattern() {
        let mut server = mockito::Server::new_async().await;

        // Create transactions that trigger structuring detection
        // (amounts just under $10,000 = ~2.86 ETH at $3500)
        let txs: Vec<serde_json::Value> = (0..15)
            .map(|i| {
                make_tx_with_idx(
                    i as u64,
                    "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                    &format!("0x{:040x}", i + 1),
                    "9900000000000000000", // ~9.9 ETH
                    &format!("{}", 1700000000 + i * 3600),
                )
            })
            .collect();

        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_json_response(&txs))
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);

        let assessment = engine
            .assess_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();

        // Should have run with the client and produced a valid assessment
        assert!(!assessment.address.is_empty());
        assert!(assessment.overall_score >= 0.0);
    }

    #[tokio::test]
    async fn test_risk_engine_with_client_api_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);

        // Should still succeed (error paths return default factors)
        let assessment = engine
            .assess_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();

        assert!(!assessment.address.is_empty());
    }

    #[tokio::test]
    async fn test_risk_engine_with_client_many_counterparties() {
        let mut server = mockito::Server::new_async().await;

        // Create transactions with > 100 unique counterparties
        let txs: Vec<serde_json::Value> = (0..120)
            .map(|i| {
                make_tx_with_idx(
                    i as u64,
                    "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                    &format!("0x{:040x}", i + 1),
                    "1000000000000000000",
                    &format!("{}", 1700000000 + i * 600),
                )
            })
            .collect();

        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_json_response(&txs))
            .create_async()
            .await;

        let sources = datasource::DataSources::new("test_key".to_string());
        let client = datasource::BlockchainDataClient::with_base_url(sources, &server.url());
        let engine = RiskEngine::with_data_client(client);

        let assessment = engine
            .assess_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();

        // Should have elevated risk due to high counterparty count
        assert!(assessment.overall_score > 0.0);
    }

    #[test]
    fn test_gini_coefficient_equal_distribution() {
        // All equal holdings = Gini 0
        let holdings = vec![10.0, 10.0, 10.0, 10.0, 10.0];
        let gini = compute_gini_coefficient(&holdings);
        assert!(gini < 0.01, "Expected near-zero Gini, got {}", gini);
    }

    #[test]
    fn test_gini_coefficient_concentrated() {
        // One holder has everything
        let holdings = vec![0.0, 0.0, 0.0, 0.0, 100.0];
        let gini = compute_gini_coefficient(&holdings);
        assert!(gini > 0.7, "Expected high Gini, got {}", gini);
    }

    #[test]
    fn test_gini_coefficient_empty() {
        let gini = compute_gini_coefficient(&[]);
        assert_eq!(gini, 0.0);
    }

    #[test]
    fn test_rugpull_indicators_none() {
        let factors = detect_rugpull_indicators(None, None);
        assert!(factors.is_empty());
    }

    #[test]
    fn test_whale_detection_no_whales() {
        let txs = vec![];
        let factor = detect_whale_activity(&txs, 100.0, 100_000.0);
        assert!(factor.score < 1.0);
    }
}
