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
pub fn format_risk_report(assessment: &RiskAssessment, format: OutputFormat, detailed: bool) -> String {
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
    output.push_str(&format!("\n{} Risk Assessment Report\n", assessment.risk_level.emoji()));
    output.push_str(&"═".repeat(60));
    output.push('\n');

    // Summary section (simple text format)
    output.push_str(&format!("{:<20} {}\n", "Address:", assessment.address));
    output.push_str(&format!("{:<20} {}\n", "Chain:", assessment.chain));
    output.push_str(&format!("{:<20} {:.1}/10\n", "Risk Score:", assessment.overall_score));
    output.push_str(&format!("{:<20} {} {:?}\n", "Risk Level:", assessment.risk_level.emoji(), assessment.risk_level));
    output.push_str(&format!("{:<20} {}\n", "Assessed At:", assessment.assessed_at.format("%Y-%m-%d %H:%M UTC")));

    // Risk factors
    if detailed {
        output.push_str("\n📊 Risk Factor Breakdown\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');
        output.push_str(&format!("{:<25} {:<12} {:<8} {:<8} {:<10}\n", "Factor", "Category", "Score", "Weight", "Weighted"));
        output.push_str(&"─".repeat(60));
        output.push('\n');

        for factor in &assessment.factors {
            let weighted = factor.score * factor.weight;
            output.push_str(&format!(
                "{:<25} {:<12} {:<8.1} {:<8.0}% {:<10.2}\n",
                factor.name.chars().take(24).collect::<String>(),
                format!("{:?}", factor.category).chars().take(11).collect::<String>(),
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

    md.push_str(&format!("# Risk Assessment Report\n\n"));
    md.push_str(&format!("**Address:** `{}`\n\n", assessment.address));
    md.push_str(&format!("**Chain:** {}\n\n", assessment.chain));
    md.push_str(&format!("**Risk Score:** {:.1}/10\n\n", assessment.overall_score));
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
                factor.name, factor.category, factor.score, 
                factor.weight * 100.0, weighted
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
