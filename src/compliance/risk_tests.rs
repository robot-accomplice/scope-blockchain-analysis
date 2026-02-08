#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_risk_assessment_low_risk() {
        let engine = RiskEngine::new();
        let assessment = engine.assess_address(
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "ethereum"
        ).await.unwrap();
        
        assert_eq!(assessment.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(assessment.chain, "ethereum");
        assert!(assessment.overall_score >= 0.0 && assessment.overall_score <= 10.0);
        assert!(!assessment.factors.is_empty());
        assert!(!assessment.recommendations.is_empty());
    }

    #[test]
    fn test_risk_level_emojis() {
        assert_eq!(RiskLevel::Low.emoji(), "🟢");
        assert_eq!(RiskLevel::Medium.emoji(), "🟡");
        assert_eq!(RiskLevel::High.emoji(), "🔴");
        assert_eq!(RiskLevel::Critical.emoji(), "⚫");
    }

    #[test]
    fn test_risk_level_from_score_boundaries() {
        assert!(matches!(RiskLevel::from_score(0.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(3.0), RiskLevel::Low));
        assert!(matches!(RiskLevel::from_score(3.1), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(6.0), RiskLevel::Medium));
        assert!(matches!(RiskLevel::from_score(6.1), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(8.0), RiskLevel::High));
        assert!(matches!(RiskLevel::from_score(8.1), RiskLevel::Critical));
        assert!(matches!(RiskLevel::from_score(10.0), RiskLevel::Critical));
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
                description: "Test factor 1".to_string(),
                evidence: vec!["Evidence 1".to_string()],
            },
            RiskFactor {
                name: "Test2".to_string(),
                category: RiskCategory::Association,
                score: 3.0,
                weight: 0.5,
                description: "Test factor 2".to_string(),
                evidence: vec!["Evidence 2".to_string()],
            },
        ];
        
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
    fn test_recommendations_by_level() {
        let engine = RiskEngine::new();
        let factors = vec![];
        
        let low_recs = engine.generate_recommendations(&factors, RiskLevel::Low);
        assert!(low_recs.iter().any(|r| r.contains("Standard monitoring")));
        
        let med_recs = engine.generate_recommendations(&factors, RiskLevel::Medium);
        assert!(med_recs.iter().any(|r| r.contains("Standard due diligence")));
        
        let high_recs = engine.generate_recommendations(&factors, RiskLevel::High);
        assert!(high_recs.iter().any(|r| r.contains("Enhanced due diligence")));
        
        let crit_recs = engine.generate_recommendations(&factors, RiskLevel::Critical);
        assert!(crit_recs.iter().any(|r| r.contains("Immediate investigation")));
    }

    #[test]
    fn test_risk_factor_categories() {
        let categories = vec![
            RiskCategory::Behavioral,
            RiskCategory::Association,
            RiskCategory::Source,
            RiskCategory::Destination,
            RiskCategory::Entity,
            RiskCategory::Sanctions,
            RiskCategory::Reputation,
        ];
        
        for category in categories {
            let factor = RiskFactor {
                name: "Test".to_string(),
                category,
                score: 5.0,
                weight: 0.5,
                description: "Test".to_string(),
                evidence: vec![],
            };
            assert_eq!(factor.category, category);
        }
    }
}
