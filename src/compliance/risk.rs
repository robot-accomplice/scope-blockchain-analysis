//! Risk Scoring Engine for BCC
//! 
//! Provides compliance-grade risk analysis for blockchain addresses.
//! Aggregates data from multiple sources to produce comprehensive risk scores.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

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
    pub score: f32,        // 0-10
    pub weight: f32,       // 0-1, contribution to final score
    pub description: String,
    pub evidence: Vec<String>,
}

/// Risk category for organizing factors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskCategory {
    Behavioral,    // Transaction patterns, velocity
    Association,   // Connected to known bad addresses
    Source,        // Funds from suspicious sources
    Destination,   // Funds to suspicious destinations
    Entity,        // Known entity (exchange, mixer, etc.)
    Sanctions,     // OFAC, sanctions lists
    Reputation,    // Community reports, scam databases
}

/// Comprehensive risk assessment for an address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub address: String,
    pub chain: String,
    pub overall_score: f32,           // 0-10
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

impl RiskEngine {
    /// Create new risk engine without data sources (basic scoring only)
    pub fn new() -> Self {
        Self {
            data_client: None,
        }
    }

    /// Create new risk engine with data sources for enhanced analysis
    pub fn with_data_client(client: BlockchainDataClient) -> Self {
        Self {
            data_client: Some(client),
        }
    }

    /// Assess risk for a single address
    pub async fn assess_address(&self, address: &str, chain: &str) -> anyhow::Result<RiskAssessment> {
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
                        evidence.push("Structuring pattern detected (amounts just under thresholds)".to_string());
                    }
                    
                    if analysis.round_number_pattern {
                        score += 1.5;
                        evidence.push("Round number pattern suggests automation".to_string());
                    }
                    
                    if analysis.velocity_score > 10.0 {
                        score += 2.0;
                        evidence.push(format!("High transaction velocity: {:.1} tx/day", analysis.velocity_score));
                    }
                    
                    if analysis.unusual_hours > 0 {
                        score += 1.0;
                        evidence.push(format!("{} transactions during unusual hours", analysis.unusual_hours));
                    }
                    
                    evidence.push(format!("Analyzed {} transactions", analysis.total_transactions));
                }
                Err(e) => {
                    evidence.push(format!("Could not fetch transaction data: {}", e));
                }
            }
        } else {
            evidence.push("No data client configured - using default scores".to_string());
        }

        // Ensure score stays in bounds
        score = score.min(10.0).max(0.0);
        
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
                    
                    evidence.push(format!("Found {} unique counterparties", counterparties.len()));
                    
                    // High number of counterparties can indicate mixing
                    if counterparties.len() > 100 {
                        score += 2.0;
                        evidence.push("High number of counterparties may indicate mixing service".to_string());
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

        score = score.min(10.0).max(0.0);
        
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
                    let incoming: Vec<_> = txs.iter().filter(|tx| tx.to.to_lowercase() == address.to_lowercase()).collect();
                    
                    evidence.push(format!("Analyzed {} incoming transactions", incoming.len()));
                    
                    // Check for failed transactions
                    let failed = txs.iter().filter(|tx| tx.is_error == "1").count();
                    if failed > 0 {
                        score += 1.0;
                        evidence.push(format!("{} failed transactions detected", failed));
                    }
                    
                    // Check for contract interactions (more complex, higher risk)
                    let contract_calls = txs.iter().filter(|tx| !tx.contract_address.is_empty()).count();
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

        score = score.min(10.0).max(0.0);
        
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
                        evidence.push(format!("Contract interactions detected: {} internal transactions", internal_txs.len()));
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
        
        score = score.min(10.0).max(0.0);
        
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

        let weighted_sum: f32 = factors.iter()
            .map(|f| f.score * f.weight)
            .sum();
        
        let total_weight: f32 = factors.iter()
            .map(|f| f.weight)
            .sum();

        if total_weight == 0.0 {
            return 0.0;
        }

        (weighted_sum / total_weight).min(10.0).max(0.0)
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
                recommendations.push(
                    format!("Address {} concerns immediately", factor.name)
                );
            }
        }

        recommendations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_level_from_score() {
        assert!(matches!(RiskLevel::from_score(2.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(5.0), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(7.5), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(9.0), RiskLevel::Critical));
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
}
