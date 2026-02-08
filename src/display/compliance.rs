//! Display formatting for compliance reports

use crate::compliance::risk::RiskAssessment;
// Note: Using simple table formatting for now
// For production, add comfy_table to Cargo.toml

/// Output format options
#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Yaml,
    Markdown,
}

/// Format a risk assessment report
pub fn format_risk_report(
    assessment: &RiskAssessment,
    format: OutputFormat,
    detailed: bool,
) -> String {
    match format {
        OutputFormat::Table => format_risk_table(assessment, detailed),
        OutputFormat::Json => serde_json::to_string_pretty(assessment).unwrap_or_default(),
        OutputFormat::Yaml => serde_yaml::to_string(assessment).unwrap_or_default(),
        OutputFormat::Markdown => format_risk_markdown(assessment, detailed),
    }
}

/// Format as pretty table
fn format_risk_table(assessment: &RiskAssessment, detailed: bool) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "\n{} Risk Assessment Report\n",
        assessment.risk_level.emoji()
    ));
    output.push_str(&"═".repeat(60));
    output.push('\n');

    // Summary section (simple text format)
    output.push_str(&format!("{:<20} {}\n", "Address:", assessment.address));
    output.push_str(&format!("{:<20} {}\n", "Chain:", assessment.chain));
    output.push_str(&format!(
        "{:<20} {:.1}/10\n",
        "Risk Score:", assessment.overall_score
    ));
    output.push_str(&format!(
        "{:<20} {} {:?}\n",
        "Risk Level:",
        assessment.risk_level.emoji(),
        assessment.risk_level
    ));
    output.push_str(&format!(
        "{:<20} {}\n",
        "Assessed At:",
        assessment.assessed_at.format("%Y-%m-%d %H:%M UTC")
    ));

    // Risk factors
    if detailed {
        output.push_str("\n📊 Risk Factor Breakdown\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');
        output.push_str(&format!(
            "{:<25} {:<12} {:<8} {:<8} {:<10}\n",
            "Factor", "Category", "Score", "Weight", "Weighted"
        ));
        output.push_str(&"─".repeat(60));
        output.push('\n');

        for factor in &assessment.factors {
            let weighted = factor.score * factor.weight;
            output.push_str(&format!(
                "{:<25} {:<12} {:<8.1} {:<8.0}% {:<10.2}\n",
                factor.name.chars().take(24).collect::<String>(),
                format!("{:?}", factor.category)
                    .chars()
                    .take(11)
                    .collect::<String>(),
                factor.score,
                factor.weight * 100.0,
                weighted
            ));
        }
    }

    // Recommendations
    if !assessment.recommendations.is_empty() {
        output.push_str("\n💡 Recommendations\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');

        for (i, rec) in assessment.recommendations.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, rec));
        }
    }

    output
}

/// Format as markdown report
fn format_risk_markdown(assessment: &RiskAssessment, detailed: bool) -> String {
    let mut md = String::new();

    md.push_str("# Risk Assessment Report\n\n");
    md.push_str(&format!("**Address:** `{}`\n\n", assessment.address));
    md.push_str(&format!("**Chain:** {}\n\n", assessment.chain));
    md.push_str(&format!(
        "**Risk Score:** {:.1}/10\n\n",
        assessment.overall_score
    ));
    md.push_str(&format!(
        "**Risk Level:** {} {:?}\n\n",
        assessment.risk_level.emoji(),
        assessment.risk_level
    ));
    md.push_str(&format!(
        "**Assessed At:** {}\n\n",
        assessment.assessed_at.format("%Y-%m-%d %H:%M UTC")
    ));

    if detailed {
        md.push_str("## Risk Factor Breakdown\n\n");
        md.push_str("| Factor | Category | Score | Weight | Weighted |\n");
        md.push_str("|--------|----------|-------|--------|----------|\n");

        for factor in &assessment.factors {
            let weighted = factor.score * factor.weight;
            md.push_str(&format!(
                "| {} | {:?} | {:.1} | {:.0}% | {:.2} |\n",
                factor.name,
                factor.category,
                factor.score,
                factor.weight * 100.0,
                weighted
            ));
        }

        md.push('\n');

        // Detailed factor descriptions
        md.push_str("## Factor Details\n\n");
        for factor in &assessment.factors {
            md.push_str(&format!("### {} ({:?})\n\n", factor.name, factor.category));
            md.push_str(&format!("{}\n\n", factor.description));

            if !factor.evidence.is_empty() {
                md.push_str("**Evidence:**\n");
                for ev in &factor.evidence {
                    md.push_str(&format!("- {}\n", ev));
                }
                md.push('\n');
            }
        }
    }

    if !assessment.recommendations.is_empty() {
        md.push_str("## Recommendations\n\n");
        for rec in &assessment.recommendations {
            md.push_str(&format!("- {}\n", rec));
        }
        md.push('\n');
    }

    md.push_str("---\n\n");
    md.push_str("*This report was generated automatically. Always verify data from primary sources before making compliance decisions.*\n");

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::risk::{RiskAssessment, RiskCategory, RiskFactor, RiskLevel};
    use chrono::Utc;

    fn create_test_assessment() -> RiskAssessment {
        RiskAssessment {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            overall_score: 4.5,
            risk_level: RiskLevel::Medium,
            factors: vec![
                RiskFactor {
                    name: "Behavioral".to_string(),
                    category: RiskCategory::Behavioral,
                    score: 3.0,
                    weight: 0.25,
                    description: "Test behavioral".to_string(),
                    evidence: vec!["Evidence 1".to_string()],
                },
                RiskFactor {
                    name: "Association".to_string(),
                    category: RiskCategory::Association,
                    score: 6.0,
                    weight: 0.30,
                    description: "Test association".to_string(),
                    evidence: vec!["Evidence 2".to_string()],
                },
            ],
            assessed_at: Utc::now(),
            recommendations: vec!["Monitor closely".to_string()],
        }
    }

    #[test]
    fn test_format_risk_report_table() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(output.contains("Risk Assessment Report"));
        assert!(output.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(output.contains("ethereum"));
    }

    #[test]
    fn test_format_risk_report_detailed() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Table, true);
        assert!(output.contains("Risk Factor Breakdown"));
        assert!(output.contains("Behavioral"));
        assert!(output.contains("Association"));
    }

    #[test]
    fn test_format_risk_report_json() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Json, false);
        assert!(output.contains("address"));
        assert!(output.contains("ethereum"));
        assert!(output.contains("overall_score"));
    }

    #[test]
    fn test_format_risk_report_yaml() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Yaml, false);
        assert!(output.contains("address:"));
        assert!(output.contains("chain:"));
    }

    #[test]
    fn test_format_risk_report_markdown() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Markdown, true);
        assert!(output.contains("# Risk Assessment Report"));
        assert!(output.contains("## Risk Factor Breakdown"));
        assert!(output.contains("## Recommendations"));
    }

    #[test]
    fn test_format_low_risk() {
        let mut assessment = create_test_assessment();
        assessment.risk_level = RiskLevel::Low;
        assessment.overall_score = 2.0;

        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(output.contains("🟢"));
    }

    #[test]
    fn test_format_high_risk() {
        let mut assessment = create_test_assessment();
        assessment.risk_level = RiskLevel::High;
        assessment.overall_score = 7.5;

        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(output.contains("🔴"));
    }

    #[test]
    fn test_format_critical_risk() {
        let mut assessment = create_test_assessment();
        assessment.risk_level = RiskLevel::Critical;
        assessment.overall_score = 9.0;

        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(output.contains("⚫"));
    }

    #[test]
    fn test_empty_recommendations() {
        let mut assessment = create_test_assessment();
        assessment.recommendations = vec![];

        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        // Should not panic and should not contain recommendations section
        assert!(output.contains("Risk Assessment Report"));
    }

    #[test]
    fn test_markdown_no_detailed() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Markdown, false);
        // Should not contain detailed factor breakdown when detailed=false
        assert!(!output.contains("## Risk Factor Breakdown"));
    }

    // ========================================================================
    // Additional edge case tests
    // ========================================================================

    #[test]
    fn test_format_risk_report_no_factors() {
        let mut assessment = create_test_assessment();
        assessment.factors = vec![];
        // Table, detailed → should still render without factors
        let output = format_risk_report(&assessment, OutputFormat::Table, true);
        assert!(output.contains("Risk Assessment Report"));
        assert!(output.contains("Risk Factor Breakdown"));
    }

    #[test]
    fn test_format_risk_markdown_no_factors() {
        let mut assessment = create_test_assessment();
        assessment.factors = vec![];
        let output = format_risk_report(&assessment, OutputFormat::Markdown, true);
        assert!(output.contains("# Risk Assessment Report"));
        assert!(output.contains("## Risk Factor Breakdown"));
    }

    #[test]
    fn test_format_risk_json_roundtrip() {
        let assessment = create_test_assessment();
        let json = format_risk_report(&assessment, OutputFormat::Json, false);
        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["address"],
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
        );
        assert_eq!(parsed["chain"], "ethereum");
    }

    #[test]
    fn test_format_risk_yaml_roundtrip() {
        let assessment = create_test_assessment();
        let yaml = format_risk_report(&assessment, OutputFormat::Yaml, false);
        // Should be valid YAML that can be deserialized back
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(parsed["address"].as_str().unwrap().starts_with("0x742d"));
    }

    #[test]
    fn test_format_risk_table_no_recommendations() {
        let mut assessment = create_test_assessment();
        assessment.recommendations = vec![];
        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(!output.contains("Recommendations"));
    }

    #[test]
    fn test_format_risk_table_many_recommendations() {
        let mut assessment = create_test_assessment();
        assessment.recommendations = (0..10).map(|i| format!("Recommendation {}", i)).collect();
        let output = format_risk_report(&assessment, OutputFormat::Table, false);
        assert!(output.contains("1."));
        assert!(output.contains("10."));
    }

    #[test]
    fn test_format_risk_markdown_with_evidence() {
        let mut assessment = create_test_assessment();
        assessment.factors[0].evidence = vec![
            "Evidence A".to_string(),
            "Evidence B".to_string(),
            "Evidence C".to_string(),
        ];
        let output = format_risk_report(&assessment, OutputFormat::Markdown, true);
        assert!(output.contains("Evidence A"));
        assert!(output.contains("Evidence B"));
        assert!(output.contains("Evidence C"));
    }

    #[test]
    fn test_format_risk_markdown_empty_evidence() {
        let mut assessment = create_test_assessment();
        for factor in &mut assessment.factors {
            factor.evidence = vec![];
        }
        let output = format_risk_report(&assessment, OutputFormat::Markdown, true);
        // Should not contain "**Evidence:**" section since evidence is empty
        assert!(!output.contains("**Evidence:**"));
    }

    #[test]
    fn test_format_risk_table_long_factor_name() {
        let mut assessment = create_test_assessment();
        assessment.factors[0].name =
            "A Very Long Factor Name That Exceeds 24 Characters".to_string();
        let output = format_risk_report(&assessment, OutputFormat::Table, true);
        // Should be truncated to 24 chars
        assert!(output.contains("A Very Long Factor Name "));
    }

    #[test]
    fn test_all_output_formats_no_panic() {
        let assessment = create_test_assessment();
        for format in [
            OutputFormat::Table,
            OutputFormat::Json,
            OutputFormat::Yaml,
            OutputFormat::Markdown,
        ] {
            for detailed in [true, false] {
                let output = format_risk_report(&assessment, format, detailed);
                assert!(!output.is_empty());
            }
        }
    }

    #[test]
    fn test_output_format_default() {
        let format = OutputFormat::default();
        assert!(matches!(format, OutputFormat::Table));
    }

    #[test]
    fn test_markdown_contains_disclaimer() {
        let assessment = create_test_assessment();
        let output = format_risk_report(&assessment, OutputFormat::Markdown, false);
        assert!(output.contains("generated automatically"));
    }

    #[test]
    fn test_format_all_risk_levels() {
        let mut assessment = create_test_assessment();
        for (level, emoji) in [
            (RiskLevel::Low, "🟢"),
            (RiskLevel::Medium, "🟡"),
            (RiskLevel::High, "🔴"),
            (RiskLevel::Critical, "⚫"),
        ] {
            assessment.risk_level = level;
            let output = format_risk_report(&assessment, OutputFormat::Table, false);
            assert!(output.contains(emoji));
            let md = format_risk_report(&assessment, OutputFormat::Markdown, false);
            assert!(md.contains(emoji));
        }
    }

    #[test]
    fn test_format_risk_markdown_all_categories() {
        let mut assessment = create_test_assessment();
        assessment.factors = vec![
            RiskFactor {
                name: "Behavioral".to_string(),
                category: RiskCategory::Behavioral,
                score: 3.0,
                weight: 0.2,
                description: "Behavioral analysis".to_string(),
                evidence: vec!["evidence".to_string()],
            },
            RiskFactor {
                name: "Association".to_string(),
                category: RiskCategory::Association,
                score: 4.0,
                weight: 0.2,
                description: "Association analysis".to_string(),
                evidence: vec![],
            },
            RiskFactor {
                name: "Source".to_string(),
                category: RiskCategory::Source,
                score: 2.0,
                weight: 0.2,
                description: "Source analysis".to_string(),
                evidence: vec![],
            },
            RiskFactor {
                name: "Destination".to_string(),
                category: RiskCategory::Destination,
                score: 1.0,
                weight: 0.2,
                description: "Destination analysis".to_string(),
                evidence: vec![],
            },
            RiskFactor {
                name: "Entity".to_string(),
                category: RiskCategory::Entity,
                score: 5.0,
                weight: 0.2,
                description: "Entity analysis".to_string(),
                evidence: vec![],
            },
        ];
        let output = format_risk_report(&assessment, OutputFormat::Markdown, true);
        assert!(output.contains("Behavioral"));
        assert!(output.contains("Association"));
        assert!(output.contains("Source"));
        assert!(output.contains("Destination"));
        assert!(output.contains("Entity"));
    }
}
