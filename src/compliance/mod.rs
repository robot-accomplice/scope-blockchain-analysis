//! Compliance module for BCC
//! 
//! Provides risk scoring, transaction taint analysis, pattern detection,
//! and compliance reporting for blockchain addresses and transactions.

pub mod datasource;
pub mod risk;

use risk::{RiskEngine, RiskAssessment};

/// Main compliance analyzer
pub struct ComplianceAnalyzer {
    risk_engine: RiskEngine,
}

impl ComplianceAnalyzer {
    /// Create a new compliance analyzer
    pub fn new() -> Self {
        Self {
            risk_engine: RiskEngine::new(),
        }
    }

    /// Analyze an address for compliance risks
    pub async fn analyze_address(&self, address: &str, chain: &str) -> anyhow::Result<RiskAssessment> {
        self.risk_engine.assess_address(address, chain).await
    }

    /// Check if an address matches known sanctions lists
    /// 
    /// Note: Sanctions checking requires external API integration.
    /// This function returns a structure ready for future implementation.
    pub fn check_sanctions(&self, _address: &str) -> SanctionsCheckResult {
        // Future: Integrate with OFAC, EU, UN sanctions databases
        SanctionsCheckResult {
            is_sanctioned: false,
            lists_checked: vec![],
            matches: vec![],
        }
    }
}

/// Result of sanctions list check
#[derive(Debug, Clone)]
pub struct SanctionsCheckResult {
    pub is_sanctioned: bool,
    pub lists_checked: Vec<String>,
    pub matches: Vec<SanctionsMatch>,
}

/// Individual sanctions list match
#[derive(Debug, Clone)]
pub struct SanctionsMatch {
    pub list_name: String,
    pub entity_name: String,
    pub match_type: MatchType,
    pub confidence: f32,
}

/// Type of sanctions match
#[derive(Debug, Clone)]
pub enum MatchType {
    Exact,
    Partial,
    Associated,
}

impl SanctionsCheckResult {
    /// Check if any sanctions list matches were found
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Get formatted summary of the check
    pub fn summary(&self) -> String {
        if self.is_sanctioned {
            format!(
                "⚠️  SANCTIONS MATCH FOUND! Checked {} lists, found {} matches.",
                self.lists_checked.len(),
                self.matches.len()
            )
        } else {
            format!(
                "✅ No sanctions matches. Checked: {}",
                self.lists_checked.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanctions_check_result() {
        let result = SanctionsCheckResult {
            is_sanctioned: false,
            lists_checked: vec!["OFAC".to_string()],
            matches: vec![],
        };

        assert!(!result.has_matches());
    }

    #[test]
    fn test_sanctions_match_found() {
        let result = SanctionsCheckResult {
            is_sanctioned: true,
            lists_checked: vec!["OFAC".to_string()],
            matches: vec![SanctionsMatch {
                list_name: "OFAC".to_string(),
                entity_name: "Test Entity".to_string(),
                match_type: MatchType::Exact,
                confidence: 1.0,
            }],
        };

        assert!(result.has_matches());
        assert!(result.summary().contains("SANCTIONS MATCH"));
    }
}
