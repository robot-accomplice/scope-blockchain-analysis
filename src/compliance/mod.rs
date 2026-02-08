//! Compliance module for Scope
//!
//! Provides risk scoring, transaction taint analysis, pattern detection,
//! and compliance reporting for blockchain addresses and transactions.

pub mod datasource;
pub mod risk;

use risk::{RiskAssessment, RiskEngine};

/// Main compliance analyzer
pub struct ComplianceAnalyzer {
    risk_engine: RiskEngine,
}

impl Default for ComplianceAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ComplianceAnalyzer {
    /// Create a new compliance analyzer
    pub fn new() -> Self {
        Self {
            risk_engine: RiskEngine::new(),
        }
    }

    /// Analyze an address for compliance risks
    pub async fn analyze_address(
        &self,
        address: &str,
        chain: &str,
    ) -> anyhow::Result<RiskAssessment> {
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

    #[test]
    fn test_sanctions_no_match_summary() {
        let result = SanctionsCheckResult {
            is_sanctioned: false,
            lists_checked: vec!["OFAC".to_string(), "EU".to_string()],
            matches: vec![],
        };

        let summary = result.summary();
        assert!(summary.contains("No sanctions matches"));
        assert!(summary.contains("OFAC"));
        assert!(summary.contains("EU"));
    }

    #[test]
    fn test_compliance_analyzer_new() {
        let analyzer = ComplianceAnalyzer::new();
        let result = analyzer.check_sanctions("0xtest");
        assert!(!result.is_sanctioned);
        assert!(result.lists_checked.is_empty());
        assert!(result.matches.is_empty());
    }

    #[test]
    fn test_compliance_analyzer_default() {
        let analyzer = ComplianceAnalyzer::default();
        let result = analyzer.check_sanctions("0xtest");
        assert!(!result.has_matches());
    }

    #[tokio::test]
    async fn test_compliance_analyze_address() {
        let analyzer = ComplianceAnalyzer::new();
        let assessment = analyzer
            .analyze_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum")
            .await
            .unwrap();
        assert_eq!(
            assessment.address,
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
        );
        assert_eq!(assessment.chain, "ethereum");
        assert!(!assessment.factors.is_empty());
    }

    #[test]
    fn test_match_type_variants() {
        // Test all match type variants
        let exact = MatchType::Exact;
        let partial = MatchType::Partial;
        let associated = MatchType::Associated;
        assert!(format!("{:?}", exact).contains("Exact"));
        assert!(format!("{:?}", partial).contains("Partial"));
        assert!(format!("{:?}", associated).contains("Associated"));
    }

    #[test]
    fn test_sanctions_check_result_debug() {
        let result = SanctionsCheckResult {
            is_sanctioned: false,
            lists_checked: vec![],
            matches: vec![],
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("SanctionsCheckResult"));
    }

    #[test]
    fn test_sanctions_match_debug() {
        let m = SanctionsMatch {
            list_name: "OFAC".to_string(),
            entity_name: "Test".to_string(),
            match_type: MatchType::Partial,
            confidence: 0.85,
        };
        let debug = format!("{:?}", m);
        assert!(debug.contains("OFAC"));
        assert!(debug.contains("0.85"));
    }
}
