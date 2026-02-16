//! # Access Control Mapping
//!
//! Analyzes smart contract source code to identify and map access control
//! patterns, privileged functions, ownership, and authorization mechanisms.
//!
//! ## Detected Patterns
//!
//! - **Ownable** - OpenZeppelin Ownable (onlyOwner modifier)
//! - **AccessControl** - Role-based access (OpenZeppelin AccessControl)
//! - **tx.origin** - Dangerous authorization via tx.origin
//! - **Custom modifiers** - Any modifier that gates function access
//! - **Renounced ownership** - renounceOwnership() calls detected
//! - **Multisig patterns** - Multiple signature requirements

use crate::contract::source::ContractSource;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Complete access control analysis for a contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessControlMap {
    /// Owner address pattern (if Ownable).
    pub ownership_pattern: Option<String>,
    /// Whether ownership has been renounced.
    pub has_renounced_ownership: bool,
    /// Whether role-based access control is used.
    pub has_role_based_access: bool,
    /// Whether tx.origin is used for authorization (dangerous).
    pub uses_tx_origin: bool,
    /// tx.origin usage locations.
    pub tx_origin_locations: Vec<SourceLocation>,
    /// Detected custom access modifiers.
    pub modifiers: Vec<AccessModifier>,
    /// Functions with access restrictions.
    pub privileged_functions: Vec<PrivilegedFunction>,
    /// Defined roles (AccessControl pattern).
    pub roles: Vec<String>,
    /// msg.sender vs tx.origin comparison.
    pub auth_analysis: AuthAnalysis,
}

/// A location in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocation {
    /// File path or contract name.
    pub file: String,
    /// Approximate line number (0 if unknown).
    pub line: usize,
    /// The relevant code snippet.
    pub snippet: String,
}

/// An access control modifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessModifier {
    /// Modifier name (e.g., "onlyOwner", "onlyRole").
    pub name: String,
    /// What the modifier checks.
    pub check_type: ModifierCheckType,
    /// Number of functions using this modifier.
    pub usage_count: usize,
}

/// Type of access check performed by a modifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModifierCheckType {
    /// Checks msg.sender == owner.
    OwnerOnly,
    /// Checks hasRole(role, msg.sender).
    RoleBased,
    /// Checks tx.origin (dangerous).
    TxOrigin,
    /// Custom/unknown check.
    Custom,
}

/// A function with access control restrictions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivilegedFunction {
    /// Function name.
    pub name: String,
    /// Access modifier(s) applied.
    pub modifiers: Vec<String>,
    /// What this function can do (e.g., "mint tokens", "pause contract").
    pub capability: String,
    /// Risk level of this privileged operation.
    pub risk: PrivilegeRisk,
}

/// Risk level for a privileged operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrivilegeRisk {
    /// Can drain funds or destroy contract.
    Critical,
    /// Can modify key parameters or pause.
    High,
    /// Can change configuration.
    Medium,
    /// Administrative but low risk.
    Low,
}

/// Authorization mechanism analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthAnalysis {
    /// Number of msg.sender checks found.
    pub msg_sender_checks: usize,
    /// Number of tx.origin checks found (should be 0 ideally).
    pub tx_origin_checks: usize,
    /// Whether require(tx.origin == msg.sender) pattern is used.
    pub has_origin_sender_comparison: bool,
    /// Summary of authorization approach.
    pub summary: String,
}

/// Analyze access control patterns in contract source code.
pub fn analyze_access_control(source: &ContractSource) -> AccessControlMap {
    let code = &source.source_code;

    let ownership_pattern = detect_ownership_pattern(code);
    let has_renounced_ownership = code.contains("renounceOwnership");
    let has_role_based_access =
        code.contains("AccessControl") || code.contains("hasRole") || code.contains("grantRole");
    let uses_tx_origin = code.contains("tx.origin");
    let tx_origin_locations = find_tx_origin_usage(code);
    let modifiers = detect_modifiers(code);
    let privileged_functions = detect_privileged_functions(code, &source.parsed_abi);
    let roles = detect_roles(code);
    let auth_analysis = analyze_auth_mechanisms(code);

    AccessControlMap {
        ownership_pattern,
        has_renounced_ownership,
        has_role_based_access,
        uses_tx_origin,
        tx_origin_locations,
        modifiers,
        privileged_functions,
        roles,
        auth_analysis,
    }
}

fn detect_ownership_pattern(code: &str) -> Option<String> {
    if code.contains("Ownable") {
        Some("OpenZeppelin Ownable".to_string())
    } else if code.contains("owner()") || code.contains("_owner") {
        Some("Custom owner pattern".to_string())
    } else if code.contains("AccessControl") {
        Some("Role-based (AccessControl)".to_string())
    } else {
        None
    }
}

fn find_tx_origin_usage(code: &str) -> Vec<SourceLocation> {
    let mut locations = Vec::new();
    for (line_num, line) in code.lines().enumerate() {
        if line.contains("tx.origin") {
            locations.push(SourceLocation {
                file: String::new(),
                line: line_num + 1,
                snippet: line.trim().to_string(),
            });
        }
    }
    locations
}

fn detect_modifiers(code: &str) -> Vec<AccessModifier> {
    let mut modifiers = Vec::new();

    // Match modifier definitions: `modifier name(...) {`
    let re = Regex::new(r"modifier\s+(\w+)").unwrap();
    for cap in re.captures_iter(code) {
        let name = cap[1].to_string();

        let check_type = if name.contains("onlyOwner") || name.contains("only_owner") {
            ModifierCheckType::OwnerOnly
        } else if name.contains("onlyRole")
            || name.contains("only_role")
            || name.contains("onlyAdmin")
        {
            ModifierCheckType::RoleBased
        } else {
            ModifierCheckType::Custom
        };

        // Count usage of this modifier in function declarations
        let usage_pattern = format!(r"\b{}\b", regex::escape(&name));
        let usage_re = Regex::new(&usage_pattern).unwrap();
        let usage_count = usage_re.find_iter(code).count().saturating_sub(1); // Subtract definition

        modifiers.push(AccessModifier {
            name,
            check_type,
            usage_count,
        });
    }

    // Detect onlyOwner even if defined in imported contract
    if code.contains("onlyOwner") && !modifiers.iter().any(|m| m.name == "onlyOwner") {
        let usage_re = Regex::new(r"\bonlyOwner\b").unwrap();
        let usage_count = usage_re.find_iter(code).count();
        modifiers.push(AccessModifier {
            name: "onlyOwner".to_string(),
            check_type: ModifierCheckType::OwnerOnly,
            usage_count,
        });
    }

    modifiers
}

fn detect_privileged_functions(
    code: &str,
    abi: &[crate::contract::source::AbiEntry],
) -> Vec<PrivilegedFunction> {
    let mut functions = Vec::new();

    // Detect common privileged function patterns
    let patterns: Vec<(&str, &str, PrivilegeRisk)> = vec![
        ("mint", "Mint/create new tokens", PrivilegeRisk::Critical),
        ("burn", "Burn/destroy tokens", PrivilegeRisk::High),
        ("pause", "Pause contract operations", PrivilegeRisk::High),
        (
            "unpause",
            "Unpause contract operations",
            PrivilegeRisk::High,
        ),
        ("setFee", "Modify fee parameters", PrivilegeRisk::Medium),
        ("setPrice", "Modify price parameters", PrivilegeRisk::Medium),
        (
            "withdraw",
            "Withdraw funds from contract",
            PrivilegeRisk::Critical,
        ),
        (
            "transferOwnership",
            "Transfer contract ownership",
            PrivilegeRisk::Critical,
        ),
        (
            "upgradeTo",
            "Upgrade contract implementation",
            PrivilegeRisk::Critical,
        ),
        ("selfdestruct", "Destroy contract", PrivilegeRisk::Critical),
        ("blacklist", "Blacklist addresses", PrivilegeRisk::High),
        ("whitelist", "Whitelist addresses", PrivilegeRisk::Medium),
        ("setOracle", "Change price oracle", PrivilegeRisk::Critical),
        ("setRouter", "Change DEX router", PrivilegeRisk::Critical),
    ];

    let fn_name_re = Regex::new(r"function\s+(\w+)").unwrap();

    for (pattern, capability, risk) in &patterns {
        // Check source for function + modifier combination
        let fn_re = Regex::new(&format!(
            r"function\s+\w*{}\w*\s*\([^)]*\)[^{{]*\b(onlyOwner|onlyRole|onlyAdmin|whenNotPaused)\b",
            regex::escape(pattern)
        ));
        if let Ok(re) = fn_re {
            for cap in re.captures_iter(code) {
                let full_match = cap.get(0).map_or("", |m| m.as_str());
                if let Some(fn_cap) = fn_name_re.captures(full_match) {
                    let fn_name = fn_cap[1].to_string();
                    let modifier = cap[1].to_string();
                    functions.push(PrivilegedFunction {
                        name: fn_name,
                        modifiers: vec![modifier],
                        capability: capability.to_string(),
                        risk: risk.clone(),
                    });
                }
            }
        }

        // Also check ABI for state-changing functions matching the pattern
        let pattern_lower = pattern.to_lowercase();
        for entry in abi {
            if entry.entry_type == "function"
                && entry.name.to_lowercase().contains(&pattern_lower)
                && entry.is_state_changing()
                && !functions.iter().any(|f| f.name == entry.name)
            {
                functions.push(PrivilegedFunction {
                    name: entry.name.clone(),
                    modifiers: vec!["(from ABI)".to_string()],
                    capability: capability.to_string(),
                    risk: risk.clone(),
                });
            }
        }
    }

    functions
}

fn detect_roles(code: &str) -> Vec<String> {
    let mut roles = Vec::new();

    // Match bytes32 constant role definitions
    // e.g., bytes32 public constant MINTER_ROLE = keccak256("MINTER_ROLE");
    let re = Regex::new(r#"(?:bytes32|constant)\s+.*?(\w+_ROLE)\s*="#).unwrap();
    for cap in re.captures_iter(code) {
        roles.push(cap[1].to_string());
    }

    // Common role patterns
    for role in &[
        "DEFAULT_ADMIN_ROLE",
        "MINTER_ROLE",
        "PAUSER_ROLE",
        "BURNER_ROLE",
        "UPGRADER_ROLE",
    ] {
        if code.contains(role) && !roles.contains(&role.to_string()) {
            roles.push(role.to_string());
        }
    }

    roles
}

fn analyze_auth_mechanisms(code: &str) -> AuthAnalysis {
    let msg_sender_re = Regex::new(r"msg\.sender").unwrap();
    let tx_origin_re = Regex::new(r"tx\.origin").unwrap();
    let origin_sender_re =
        Regex::new(r"(?:tx\.origin\s*==\s*msg\.sender|msg\.sender\s*==\s*tx\.origin)").unwrap();

    let msg_sender_checks = msg_sender_re.find_iter(code).count();
    let tx_origin_checks = tx_origin_re.find_iter(code).count();
    let has_origin_sender_comparison = origin_sender_re.is_match(code);

    let summary = if tx_origin_checks > 0 && !has_origin_sender_comparison {
        format!(
            "DANGER: Uses tx.origin ({} occurrence(s)) without msg.sender comparison. \
             This is vulnerable to phishing attacks via malicious contracts.",
            tx_origin_checks
        )
    } else if has_origin_sender_comparison {
        "Uses tx.origin == msg.sender comparison (anti-contract-call guard). \
         Less risky but blocks legitimate contract interactions."
            .to_string()
    } else if msg_sender_checks > 0 {
        format!(
            "Uses msg.sender for authorization ({} check(s)). This is the recommended approach.",
            msg_sender_checks
        )
    } else {
        "No explicit authorization checks detected.".to_string()
    };

    AuthAnalysis {
        msg_sender_checks,
        tx_origin_checks,
        has_origin_sender_comparison,
        summary,
    }
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
    fn test_detect_ownable() {
        let src = make_source("contract Token is Ownable { function mint() onlyOwner {} }");
        let ac = analyze_access_control(&src);
        assert_eq!(
            ac.ownership_pattern,
            Some("OpenZeppelin Ownable".to_string())
        );
        assert!(!ac.has_renounced_ownership);
    }

    #[test]
    fn test_detect_renounced_ownership() {
        let src = make_source("contract Token { function renounceOwnership() {} }");
        let ac = analyze_access_control(&src);
        assert!(ac.has_renounced_ownership);
    }

    #[test]
    fn test_detect_tx_origin() {
        let src = make_source("require(tx.origin == owner, 'not owner');");
        let ac = analyze_access_control(&src);
        assert!(ac.uses_tx_origin);
        assert_eq!(ac.tx_origin_locations.len(), 1);
    }

    #[test]
    fn test_detect_roles() {
        let src = make_source(
            "bytes32 public constant MINTER_ROLE = keccak256('MINTER_ROLE');\n\
             bytes32 public constant PAUSER_ROLE = keccak256('PAUSER_ROLE');",
        );
        let ac = analyze_access_control(&src);
        assert!(ac.roles.contains(&"MINTER_ROLE".to_string()));
        assert!(ac.roles.contains(&"PAUSER_ROLE".to_string()));
    }

    #[test]
    fn test_detect_access_control() {
        let src = make_source(
            "import AccessControl; contract Token is AccessControl { \
             function mint() onlyRole(MINTER_ROLE) {} }",
        );
        let ac = analyze_access_control(&src);
        assert!(ac.has_role_based_access);
    }

    #[test]
    fn test_auth_analysis_safe() {
        let src = make_source("require(msg.sender == owner);");
        let ac = analyze_access_control(&src);
        assert_eq!(ac.auth_analysis.msg_sender_checks, 1);
        assert_eq!(ac.auth_analysis.tx_origin_checks, 0);
        assert!(ac.auth_analysis.summary.contains("recommended approach"));
    }

    #[test]
    fn test_auth_analysis_dangerous() {
        let src = make_source("require(tx.origin == owner);");
        let ac = analyze_access_control(&src);
        assert!(ac.auth_analysis.summary.contains("DANGER"));
    }

    #[test]
    fn test_detect_ownership_pattern_custom_owner() {
        let result = detect_ownership_pattern("function owner() public view returns (address) { return _owner; }");
        assert_eq!(result, Some("Custom owner pattern".to_string()));
    }

    #[test]
    fn test_detect_ownership_pattern_none() {
        let result = detect_ownership_pattern("contract SimpleToken { function transfer() {} }");
        assert_eq!(result, None);
    }

    #[test]
    fn test_detect_modifiers_custom() {
        let code = "modifier onlyValidator() { require(isValidator[msg.sender]); _; }\nfunction doThing() onlyValidator() {}";
        let modifiers = detect_modifiers(code);
        assert!(modifiers.iter().any(|m| m.name == "onlyValidator"));
        let validator_mod = modifiers.iter().find(|m| m.name == "onlyValidator").unwrap();
        assert!(matches!(validator_mod.check_type, ModifierCheckType::Custom));
    }

    #[test]
    fn test_detect_modifiers_role_based() {
        let code = "modifier onlyRole(bytes32 role) { _checkRole(role); _; }\nfunction mint() onlyRole(MINTER) {}";
        let modifiers = detect_modifiers(code);
        assert!(modifiers.iter().any(|m| m.name == "onlyRole"));
        let role_mod = modifiers.iter().find(|m| m.name == "onlyRole").unwrap();
        assert!(matches!(role_mod.check_type, ModifierCheckType::RoleBased));
    }

    #[test]
    fn test_detect_modifiers_admin() {
        let code = "modifier onlyAdmin() { require(msg.sender == admin); _; }";
        let modifiers = detect_modifiers(code);
        assert!(modifiers.iter().any(|m| m.name == "onlyAdmin"));
        let admin_mod = modifiers.iter().find(|m| m.name == "onlyAdmin").unwrap();
        assert!(matches!(admin_mod.check_type, ModifierCheckType::RoleBased));
    }

    #[test]
    fn test_detect_modifiers_imported_only_owner() {
        let code = "function mint() onlyOwner { tokens[msg.sender] += 1; }\nfunction burn() onlyOwner {}";
        let modifiers = detect_modifiers(code);
        assert!(modifiers.iter().any(|m| m.name == "onlyOwner"));
        let owner_mod = modifiers.iter().find(|m| m.name == "onlyOwner").unwrap();
        assert!(matches!(owner_mod.check_type, ModifierCheckType::OwnerOnly));
        assert!(owner_mod.usage_count >= 2);
    }

    #[test]
    fn test_detect_privileged_functions_with_abi() {
        use crate::contract::source::{AbiEntry, AbiParam};
        let code = "function mint(address to) onlyOwner { _mint(to); }";
        let abi = vec![
            AbiEntry {
                entry_type: "function".to_string(),
                name: "mint".to_string(),
                inputs: vec![AbiParam {
                    name: "to".to_string(),
                    param_type: "address".to_string(),
                    indexed: false,
                    components: vec![],
                }],
                outputs: vec![],
                state_mutability: "nonpayable".to_string(),
            },
            AbiEntry {
                entry_type: "function".to_string(),
                name: "pause".to_string(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: "nonpayable".to_string(),
            },
        ];
        let fns = detect_privileged_functions(code, &abi);
        assert!(!fns.is_empty());
        assert!(fns.iter().any(|f| f.name == "mint"));
    }

    #[test]
    fn test_detect_privileged_functions_abi_only() {
        use crate::contract::source::{AbiEntry, AbiParam};
        let code = "contract Token {}";
        let abi = vec![AbiEntry {
            entry_type: "function".to_string(),
            name: "setFeeRecipient".to_string(),
            inputs: vec![AbiParam {
                name: "r".to_string(),
                param_type: "address".to_string(),
                indexed: false,
                components: vec![],
            }],
            outputs: vec![],
            state_mutability: "nonpayable".to_string(),
        }];
        let fns = detect_privileged_functions(code, &abi);
        assert!(fns.iter().any(|f| f.name == "setFeeRecipient"));
    }

    #[test]
    fn test_detect_roles_common_patterns() {
        let code = "contract Token {\n\
            bytes32 public constant UPGRADER_ROLE = keccak256('UPGRADER_ROLE');\n\
            DEFAULT_ADMIN_ROLE;\n\
            BURNER_ROLE;\n\
        }";
        let roles = detect_roles(code);
        assert!(roles.contains(&"UPGRADER_ROLE".to_string()));
        assert!(roles.contains(&"DEFAULT_ADMIN_ROLE".to_string()));
        assert!(roles.contains(&"BURNER_ROLE".to_string()));
    }

    #[test]
    fn test_analyze_auth_tx_origin_with_msg_sender() {
        let code = "require(tx.origin == msg.sender, 'no contracts');";
        let auth = analyze_auth_mechanisms(code);
        assert!(auth.has_origin_sender_comparison);
        assert!(auth.summary.contains("anti-contract-call"));
    }

    #[test]
    fn test_analyze_auth_no_checks() {
        let code = "contract Token { function transfer() {} }";
        let auth = analyze_auth_mechanisms(code);
        assert_eq!(auth.msg_sender_checks, 0);
        assert_eq!(auth.tx_origin_checks, 0);
        assert!(auth.summary.contains("No explicit authorization"));
    }

    #[test]
    fn test_find_tx_origin_usage_multiple() {
        let code = "require(tx.origin == owner);\nrequire(tx.origin != address(0));";
        let locations = find_tx_origin_usage(code);
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].line, 1);
        assert_eq!(locations[1].line, 2);
    }

    #[test]
    fn test_privilege_risk_debug() {
        assert_eq!(format!("{:?}", PrivilegeRisk::Critical), "Critical");
        assert_eq!(format!("{:?}", PrivilegeRisk::High), "High");
        assert_eq!(format!("{:?}", PrivilegeRisk::Medium), "Medium");
        assert_eq!(format!("{:?}", PrivilegeRisk::Low), "Low");
    }

    #[test]
    fn test_modifier_check_type_debug() {
        assert_eq!(format!("{:?}", ModifierCheckType::OwnerOnly), "OwnerOnly");
        assert_eq!(format!("{:?}", ModifierCheckType::RoleBased), "RoleBased");
        assert_eq!(format!("{:?}", ModifierCheckType::Custom), "Custom");
    }
}
