//! # Setup Command
//!
//! This module implements the `setup` command for interactive configuration
//! of the BCC application. It walks users through setting up API keys and
//! preferences.
//!
//! ## Usage
//!
//! ```bash
//! # Run the full setup wizard
//! bca setup
//!
//! # Show current configuration status
//! bca setup --status
//!
//! # Set a specific API key
//! bca setup --key etherscan
//! ```

use crate::config::{Config, OutputFormat};
use crate::error::{BccError, ConfigError, Result};
use clap::Args;
use std::io::{self, Write};
use std::path::PathBuf;

/// Arguments for the setup command.
#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Show current configuration status without making changes.
    #[arg(long, short)]
    pub status: bool,

    /// Configure a specific API key only.
    #[arg(long, short, value_name = "PROVIDER")]
    pub key: Option<String>,

    /// Reset configuration to defaults.
    #[arg(long)]
    pub reset: bool,
}

/// Configuration item with metadata for display.
#[allow(dead_code)]
struct ConfigItem {
    name: &'static str,
    description: &'static str,
    env_var: &'static str,
    is_set: bool,
    value_hint: Option<String>,
}

/// Runs the setup command.
pub async fn run(args: SetupArgs, config: &Config) -> Result<()> {
    if args.status {
        show_status(config);
        return Ok(());
    }

    if args.reset {
        return reset_config();
    }

    if let Some(ref key_name) = args.key {
        return configure_single_key(key_name, config).await;
    }

    // Run full setup wizard
    run_setup_wizard(config).await
}

/// Shows the current configuration status.
fn show_status(config: &Config) {
    println!();
    println!("BCC Configuration Status");
    println!("{}", "=".repeat(60));
    println!();

    // Config file location
    let config_path = Config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Not found".to_string());
    println!("Config file: {}", config_path);
    println!();

    // API Keys
    println!("API Keys:");
    println!("{}", "-".repeat(60));

    let api_keys = get_api_key_items(config);
    let mut missing_keys = Vec::new();

    for item in &api_keys {
        let status = if item.is_set {
            "✓ Set"
        } else {
            missing_keys.push(item.name);
            "✗ Not set"
        };
        let hint = item.value_hint.as_deref().unwrap_or("");
        let info = get_api_key_info(item.name);
        println!(
            "  {:<15} {} {}",
            item.name,
            status,
            if item.is_set { hint } else { "" }
        );
        println!("    Chain: {}", info.chain);
    }

    // Show where to get missing keys
    if !missing_keys.is_empty() {
        println!();
        println!("Where to get API keys:");
        println!("{}", "-".repeat(60));
        for key_name in missing_keys {
            let info = get_api_key_info(key_name);
            println!("  {}: {}", key_name, info.url);
        }
    }

    println!();
    println!("Defaults:");
    println!("{}", "-".repeat(40));
    println!(
        "  Chain:         {}",
        config.chains.ethereum_rpc.as_deref().unwrap_or("ethereum")
    );
    println!("  Output format: {:?}", config.output.format);
    println!(
        "  Color output:  {}",
        if config.output.color {
            "enabled"
        } else {
            "disabled"
        }
    );

    println!();
    println!("Run 'bcc setup' to configure missing settings.");
    println!("Run 'bcc setup --key <provider>' to configure a specific key.");
    println!();
}

/// Gets API key configuration items.
fn get_api_key_items(config: &Config) -> Vec<ConfigItem> {
    vec![
        ConfigItem {
            name: "etherscan",
            description: "Ethereum mainnet block explorer",
            env_var: "BCC_ETHERSCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("etherscan"),
            value_hint: config.chains.api_keys.get("etherscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "bscscan",
            description: "BNB Smart Chain block explorer",
            env_var: "BCC_BSCSCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("bscscan"),
            value_hint: config.chains.api_keys.get("bscscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "polygonscan",
            description: "Polygon block explorer",
            env_var: "BCC_POLYGONSCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("polygonscan"),
            value_hint: config
                .chains
                .api_keys
                .get("polygonscan")
                .map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "arbiscan",
            description: "Arbitrum block explorer",
            env_var: "BCC_ARBISCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("arbiscan"),
            value_hint: config.chains.api_keys.get("arbiscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "basescan",
            description: "Base block explorer",
            env_var: "BCC_BASESCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("basescan"),
            value_hint: config.chains.api_keys.get("basescan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "optimism",
            description: "Optimism block explorer",
            env_var: "BCC_OPTIMISM_API_KEY",
            is_set: config.chains.api_keys.contains_key("optimism"),
            value_hint: config.chains.api_keys.get("optimism").map(|k| mask_key(k)),
        },
    ]
}

/// Masks an API key for display (shows first 4 and last 4 chars).
fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "*".repeat(key.len());
    }
    format!("({}...{})", &key[..4], &key[key.len() - 4..])
}

/// Resets configuration to defaults.
fn reset_config() -> Result<()> {
    let config_path = Config::config_path().ok_or_else(|| {
        BccError::Config(ConfigError::NotFound {
            path: PathBuf::from("~/.config/bcc/config.yaml"),
        })
    })?;

    if config_path.exists() {
        print!("This will delete your current configuration. Continue? [y/N]: ");
        io::stdout()
            .flush()
            .map_err(|e| BccError::Io(e.to_string()))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| BccError::Io(e.to_string()))?;

        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Cancelled.");
            return Ok(());
        }

        std::fs::remove_file(&config_path).map_err(|e| BccError::Io(e.to_string()))?;
        println!("Configuration reset to defaults.");
    } else {
        println!("No configuration file found. Already using defaults.");
    }

    Ok(())
}

/// Configures a single API key.
async fn configure_single_key(key_name: &str, config: &Config) -> Result<()> {
    let valid_keys = [
        "etherscan",
        "bscscan",
        "polygonscan",
        "arbiscan",
        "basescan",
        "optimism",
    ];

    if !valid_keys.contains(&key_name) {
        println!("Unknown API key: {}", key_name);
        println!();
        println!("Valid options:");
        for key in valid_keys {
            let info = get_api_key_info(key);
            println!("  {:<15} - {}", key, info.chain);
        }
        return Ok(());
    }

    let info = get_api_key_info(key_name);
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Configure {} API Key", key_name.to_uppercase());
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Chain: {}", info.chain);
    println!("Enables: {}", info.features);
    println!();
    println!("How to get your free API key:");
    println!("  {}", info.signup_steps);
    println!();
    println!("URL: {}", info.url);
    println!();

    let key = prompt_api_key(key_name)?;

    if key.is_empty() {
        println!("Skipped.");
        return Ok(());
    }

    // Update config with new key
    let mut new_config = config.clone();
    new_config.chains.api_keys.insert(key_name.to_string(), key);

    save_config(&new_config)?;
    println!("✓ {} API key saved.", key_name);

    Ok(())
}

/// API key information for each supported provider.
struct ApiKeyInfo {
    url: &'static str,
    chain: &'static str,
    features: &'static str,
    signup_steps: &'static str,
}

/// Gets detailed information for obtaining an API key.
fn get_api_key_info(key_name: &str) -> ApiKeyInfo {
    match key_name {
        "etherscan" => ApiKeyInfo {
            url: "https://etherscan.io/apis",
            chain: "Ethereum Mainnet",
            features: "token balances, transactions, holders, contract verification",
            signup_steps: "1. Visit etherscan.io/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        "bscscan" => ApiKeyInfo {
            url: "https://bscscan.com/apis",
            chain: "BNB Smart Chain (BSC)",
            features: "BSC token data, BEP-20 holders, transactions",
            signup_steps: "1. Visit bscscan.com/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        "polygonscan" => ApiKeyInfo {
            url: "https://polygonscan.com/apis",
            chain: "Polygon (MATIC)",
            features: "Polygon token data, transactions, holders",
            signup_steps: "1. Visit polygonscan.com/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        "arbiscan" => ApiKeyInfo {
            url: "https://arbiscan.io/apis",
            chain: "Arbitrum One",
            features: "Arbitrum token data, L2 transactions, holders",
            signup_steps: "1. Visit arbiscan.io/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        "basescan" => ApiKeyInfo {
            url: "https://basescan.org/apis",
            chain: "Base (Coinbase L2)",
            features: "Base token data, transactions, holders",
            signup_steps: "1. Visit basescan.org/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        "optimism" => ApiKeyInfo {
            url: "https://optimistic.etherscan.io/apis",
            chain: "Optimism (OP Mainnet)",
            features: "Optimism token data, L2 transactions, holders",
            signup_steps: "1. Visit optimistic.etherscan.io/register\n     2. Create a free account\n     3. Go to API-Keys in your account\n     4. Click 'Add' to generate a new key",
        },
        _ => ApiKeyInfo {
            url: "https://etherscan.io/apis",
            chain: "Ethereum",
            features: "blockchain data",
            signup_steps: "Visit the provider's website to register",
        },
    }
}

/// Gets the URL for obtaining an API key (for backwards compatibility).
#[cfg(test)]
fn get_api_key_url(key_name: &str) -> &'static str {
    get_api_key_info(key_name).url
}

/// Runs the full setup wizard.
async fn run_setup_wizard(config: &Config) -> Result<()> {
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                    BCC Setup Wizard                          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("This wizard will help you configure BCC (Blockchain Crawler CLI).");
    println!("Press Enter to skip any optional setting.");
    println!();

    let mut new_config = config.clone();
    let mut changes_made = false;

    // Step 1: API Keys
    println!("Step 1: API Keys");
    println!("{}", "=".repeat(60));
    println!();
    println!("API keys enable access to block explorer data including:");
    println!("  • Token balances and holder information");
    println!("  • Transaction history and details");
    println!("  • Contract verification status");
    println!("  • Token analytics and metrics");
    println!();
    println!("All API keys are FREE and take just a minute to obtain.");
    println!();

    // Etherscan (primary)
    if !config.chains.api_keys.contains_key("etherscan") {
        let info = get_api_key_info("etherscan");
        println!("┌────────────────────────────────────────────────────────────┐");
        println!("│  ETHERSCAN API KEY (Recommended)                           │");
        println!("└────────────────────────────────────────────────────────────┘");
        println!("  Chain: {}", info.chain);
        println!("  Enables: {}", info.features);
        println!();
        println!("  How to get your free API key:");
        println!("  {}", info.signup_steps);
        println!();
        println!("  URL: {}", info.url);
        println!();
        if let Some(key) = prompt_optional_key("etherscan")? {
            new_config
                .chains
                .api_keys
                .insert("etherscan".to_string(), key);
            changes_made = true;
        }
        println!();
    } else {
        println!("✓ Etherscan API key already configured");
        println!();
    }

    // Ask about other chains
    print!("Configure API keys for other chains (BSC, Polygon, Arbitrum, etc.)? [y/N]: ");
    io::stdout()
        .flush()
        .map_err(|e| BccError::Io(e.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BccError::Io(e.to_string()))?;

    if matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
        println!();

        let other_chains = ["bscscan", "polygonscan", "arbiscan", "basescan", "optimism"];

        for key_name in other_chains {
            if !config.chains.api_keys.contains_key(key_name) {
                let info = get_api_key_info(key_name);
                println!("┌────────────────────────────────────────────────────────────┐");
                println!("│  {} API KEY", key_name.to_uppercase());
                println!("└────────────────────────────────────────────────────────────┘");
                println!("  Chain: {}", info.chain);
                println!("  Enables: {}", info.features);
                println!("  URL: {}", info.url);
                println!();
                if let Some(key) = prompt_optional_key(key_name)? {
                    new_config.chains.api_keys.insert(key_name.to_string(), key);
                    changes_made = true;
                }
                println!();
            }
        }
    }

    // Step 2: Preferences
    println!();
    println!("Step 2: Preferences");
    println!("{}", "=".repeat(60));
    println!();

    // Default output format
    println!("Default output format:");
    println!("  1. table (default)");
    println!("  2. json");
    println!("  3. csv");
    print!("Select [1-3, Enter for default]: ");
    io::stdout()
        .flush()
        .map_err(|e| BccError::Io(e.to_string()))?;

    input.clear();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BccError::Io(e.to_string()))?;

    match input.trim() {
        "2" => {
            new_config.output.format = OutputFormat::Json;
            changes_made = true;
        }
        "3" => {
            new_config.output.format = OutputFormat::Csv;
            changes_made = true;
        }
        _ => {} // Keep default (table)
    }

    // Save configuration
    if changes_made {
        println!();
        println!("Saving configuration...");
        save_config(&new_config)?;
        println!();
        println!("✓ Configuration saved to ~/.config/bcc/config.yaml");
    } else {
        println!();
        println!("No changes made.");
    }

    println!();
    println!("Setup complete! You can now use BCC.");
    println!();
    println!("Quick start:");
    println!("  bcc crawl USDC              # Analyze a token");
    println!("  bcc address 0x...           # Analyze an address");
    println!("  bcc interactive             # Interactive mode");
    println!();
    println!("Run 'bcc setup --status' to view your configuration.");
    println!();

    Ok(())
}

/// Prompts for an optional API key.
fn prompt_optional_key(name: &str) -> Result<Option<String>> {
    print!("  {} API key (or Enter to skip): ", name);
    io::stdout()
        .flush()
        .map_err(|e| BccError::Io(e.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BccError::Io(e.to_string()))?;

    let key = input.trim().to_string();
    if key.is_empty() {
        Ok(None)
    } else {
        Ok(Some(key))
    }
}

/// Prompts for an API key (for single key configuration).
fn prompt_api_key(name: &str) -> Result<String> {
    print!("Enter {} API key: ", name);
    io::stdout()
        .flush()
        .map_err(|e| BccError::Io(e.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BccError::Io(e.to_string()))?;

    Ok(input.trim().to_string())
}

/// Saves the configuration to file.
fn save_config(config: &Config) -> Result<()> {
    let config_path = Config::config_path().ok_or_else(|| {
        BccError::Config(ConfigError::NotFound {
            path: PathBuf::from("~/.config/bcc/config.yaml"),
        })
    })?;

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| BccError::Io(e.to_string()))?;
    }

    // Build YAML manually for cleaner output
    let mut yaml = String::new();
    yaml.push_str("# BCC Configuration\n");
    yaml.push_str("# Generated by 'bca setup'\n\n");

    // Chains section
    yaml.push_str("chains:\n");

    // API keys
    if !config.chains.api_keys.is_empty() {
        yaml.push_str("  api_keys:\n");
        for (name, key) in &config.chains.api_keys {
            yaml.push_str(&format!("    {}: \"{}\"\n", name, key));
        }
    }

    // RPC endpoints (if configured)
    if let Some(ref rpc) = config.chains.ethereum_rpc {
        yaml.push_str(&format!("  ethereum_rpc: \"{}\"\n", rpc));
    }

    // Output section
    yaml.push_str("\noutput:\n");
    yaml.push_str(&format!("  format: {}\n", config.output.format));
    yaml.push_str(&format!("  color: {}\n", config.output.color));

    std::fs::write(&config_path, yaml).map_err(|e| BccError::Io(e.to_string()))?;

    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_key_long() {
        let masked = mask_key("ABCDEFGHIJKLMNOP");
        assert_eq!(masked, "(ABCD...MNOP)");
    }

    #[test]
    fn test_mask_key_short() {
        let masked = mask_key("SHORT");
        assert_eq!(masked, "*****");
    }

    #[test]
    fn test_get_api_key_url() {
        assert!(get_api_key_url("etherscan").contains("etherscan.io"));
        assert!(get_api_key_url("bscscan").contains("bscscan.com"));
    }
}
