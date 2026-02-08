//! Risk Scoring Engine for BCC
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
        let assessment = engine
            .assess_address("0xtest", "ethereum")
            .await
            .unwrap();

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
        let assessment = engine
            .assess_address("0xtest", "ethereum")
            .await
            .unwrap();

        for factor in &assessment.factors {
            assert!(!factor.evidence.is_empty(), "Factor {} has no evidence", factor.name);
            // Without data client, evidence should mention "No data client configured"
            assert!(
                factor.evidence.iter().any(|e| e.contains("No data client configured") || e.contains("not in known entity")),
                "Factor {} doesn't have expected evidence: {:?}",
                factor.name,
                factor.evidence
            );
        }
    }

    #[tokio::test]
    async fn test_assess_address_score_in_bounds() {
        let engine = RiskEngine::new();
        let assessment = engine
            .assess_address("0xtest", "ethereum")
            .await
            .unwrap();

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
}
