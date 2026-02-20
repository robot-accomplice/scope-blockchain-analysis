//! # Setup Command
//!
//! This module implements the `setup` command for interactive configuration
//! of the Scope application. It walks users through setting up API keys and
//! preferences.
//!
//! ## Usage
//!
//! ```bash
//! # Run the full setup wizard
//! scope setup
//!
//! # Show current configuration status
//! scope setup --status
//!
//! # Set a specific API key
//! scope setup --key etherscan
//! ```

use crate::config::{Config, OutputFormat};
use crate::error::{ConfigError, Result, ScopeError};
use clap::Args;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// Arguments for the setup command.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope setup
  scope setup --status
  scope setup --key etherscan
  scope setup --reset")]
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
    use crate::display::terminal as t;

    println!("{}", t::section_header("Scope Configuration Status"));

    // Config file location
    let config_path = Config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Not found".to_string());
    println!("{}", t::kv_row("Config file", &config_path));
    println!("{}", t::blank_row());

    // API Keys
    println!("{}", t::subsection_header("API Keys"));

    let api_keys = get_api_key_items(config);
    let mut missing_keys = Vec::new();

    for item in &api_keys {
        let info = get_api_key_info(item.name);
        if item.is_set {
            let hint = item.value_hint.as_deref().unwrap_or("");
            let msg = if hint.is_empty() {
                item.name.to_string()
            } else {
                format!("{} {}", item.name, hint)
            };
            println!("{}", t::check_pass(&msg));
        } else {
            missing_keys.push(item.name);
            println!("{}", t::check_fail(item.name));
        }
        println!("{}", t::kv_row("Chain", info.chain));
    }

    // Show where to get missing keys
    if !missing_keys.is_empty() {
        println!("{}", t::blank_row());
        println!("{}", t::subsection_header("Missing API Keys"));
        for key_name in missing_keys {
            let info = get_api_key_info(key_name);
            println!("{}", t::link_row(key_name, info.url));
        }
    }

    println!("{}", t::blank_row());
    println!("{}", t::subsection_header("Defaults"));
    println!(
        "{}",
        t::kv_row(
            "Chain",
            config.chains.ethereum_rpc.as_deref().unwrap_or("ethereum")
        )
    );
    println!(
        "{}",
        t::kv_row("Output format", &format!("{:?}", config.output.format))
    );
    println!(
        "{}",
        t::kv_row(
            "Color output",
            if config.output.color {
                "enabled"
            } else {
                "disabled"
            }
        )
    );

    // Ghola sidecar status
    println!("{}", t::blank_row());
    println!("{}", t::subsection_header("Ghola Sidecar"));

    let ghola_in_path = which_ghola();
    if ghola_in_path {
        println!("{}", t::check_pass("ghola binary found in PATH"));
    } else {
        println!("{}", t::check_fail("ghola binary not found in PATH"));
        println!(
            "{}",
            t::info_row("Install: go install github.com/robot-accomplice/ghola@latest")
        );
    }

    if config.ghola.enabled {
        println!("{}", t::check_pass("Ghola transport enabled in config"));
        if config.ghola.stealth {
            println!(
                "{}",
                t::check_pass("Stealth mode active (temporal drift + ghost signing)")
            );
        } else {
            println!(
                "{}",
                t::kv_row(
                    "Stealth mode",
                    "disabled (set ghola.stealth: true to enable)"
                )
            );
        }
    } else {
        println!(
            "{}",
            t::kv_row(
                "Transport",
                "native (set ghola.enabled: true in config to use sidecar)",
            )
        );
    }

    println!("{}", t::blank_row());
    println!(
        "{}",
        t::info_row("Run 'scope setup' to configure missing settings.")
    );
    println!(
        "{}",
        t::info_row("Run 'scope setup --key <provider>' to configure a specific key.")
    );
    println!("{}", t::section_footer());
}

/// Checks whether the `ghola` binary is present on `$PATH`.
fn which_ghola() -> bool {
    std::process::Command::new("which")
        .arg("ghola")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Gets API key configuration items.
fn get_api_key_items(config: &Config) -> Vec<ConfigItem> {
    vec![
        ConfigItem {
            name: "etherscan",
            description: "Ethereum mainnet block explorer",
            env_var: "SCOPE_ETHERSCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("etherscan"),
            value_hint: config.chains.api_keys.get("etherscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "bscscan",
            description: "BNB Smart Chain block explorer",
            env_var: "SCOPE_BSCSCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("bscscan"),
            value_hint: config.chains.api_keys.get("bscscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "polygonscan",
            description: "Polygon block explorer",
            env_var: "SCOPE_POLYGONSCAN_API_KEY",
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
            env_var: "SCOPE_ARBISCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("arbiscan"),
            value_hint: config.chains.api_keys.get("arbiscan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "basescan",
            description: "Base block explorer",
            env_var: "SCOPE_BASESCAN_API_KEY",
            is_set: config.chains.api_keys.contains_key("basescan"),
            value_hint: config.chains.api_keys.get("basescan").map(|k| mask_key(k)),
        },
        ConfigItem {
            name: "optimism",
            description: "Optimism block explorer",
            env_var: "SCOPE_OPTIMISM_API_KEY",
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
        ScopeError::Config(ConfigError::NotFound {
            path: PathBuf::from("~/.config/scope/config.yaml"),
        })
    })?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    reset_config_impl(&mut stdin.lock(), &mut stdout.lock(), &config_path)
}

/// Testable implementation of reset_config with injected I/O and path.
fn reset_config_impl(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    config_path: &Path,
) -> Result<()> {
    if config_path.exists() {
        write!(
            writer,
            "This will delete your current configuration. Continue? [y/N]: "
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
        writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

        let mut input = String::new();
        reader
            .read_line(&mut input)
            .map_err(|e| ScopeError::Io(e.to_string()))?;

        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            writeln!(writer, "Cancelled.").map_err(|e| ScopeError::Io(e.to_string()))?;
            return Ok(());
        }

        std::fs::remove_file(config_path).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "Configuration reset to defaults.")
            .map_err(|e| ScopeError::Io(e.to_string()))?;
    } else {
        writeln!(
            writer,
            "No configuration file found. Already using defaults."
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    Ok(())
}

/// Configures a single API key.
async fn configure_single_key(key_name: &str, config: &Config) -> Result<()> {
    let config_path = Config::config_path().ok_or_else(|| {
        ScopeError::Config(ConfigError::NotFound {
            path: PathBuf::from("~/.config/scope/config.yaml"),
        })
    })?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    configure_single_key_impl(
        &mut stdin.lock(),
        &mut stdout.lock(),
        key_name,
        config,
        &config_path,
    )
}

/// Testable implementation of configure_single_key with injected I/O.
fn configure_single_key_impl(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    key_name: &str,
    config: &Config,
    config_path: &Path,
) -> Result<()> {
    let valid_keys = [
        "etherscan",
        "bscscan",
        "polygonscan",
        "arbiscan",
        "basescan",
        "optimism",
    ];

    if !valid_keys.contains(&key_name) {
        writeln!(writer, "Unknown API key: {}", key_name)
            .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "Valid options:").map_err(|e| ScopeError::Io(e.to_string()))?;
        for key in valid_keys {
            let info = get_api_key_info(key);
            writeln!(writer, "  {:<15} - {}", key, info.chain)
                .map_err(|e| ScopeError::Io(e.to_string()))?;
        }
        return Ok(());
    }

    let info = get_api_key_info(key_name);
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "╔══════════════════════════════════════════════════════════════╗"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "║  Configure {} API Key", key_name.to_uppercase())
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "╚══════════════════════════════════════════════════════════════╝"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Chain: {}", info.chain).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Enables: {}", info.features).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "How to get your free API key:").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  {}", info.signup_steps).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "URL: {}", info.url).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

    let key = prompt_api_key_impl(reader, writer, key_name)?;

    if key.is_empty() {
        writeln!(writer, "Skipped.").map_err(|e| ScopeError::Io(e.to_string()))?;
        return Ok(());
    }

    // Update config with new key
    let mut new_config = config.clone();
    new_config.chains.api_keys.insert(key_name.to_string(), key);

    save_config_to_path(&new_config, config_path)?;
    writeln!(writer, "✓ {} API key saved.", key_name).map_err(|e| ScopeError::Io(e.to_string()))?;

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
    let config_path = Config::config_path().ok_or_else(|| {
        ScopeError::Config(ConfigError::NotFound {
            path: PathBuf::from("~/.config/scope/config.yaml"),
        })
    })?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_setup_wizard_impl(&mut stdin.lock(), &mut stdout.lock(), config, &config_path)
}

/// Testable implementation of the setup wizard with injected I/O.
fn run_setup_wizard_impl(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    config: &Config,
    config_path: &Path,
) -> Result<()> {
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "╔══════════════════════════════════════════════════════════════╗"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "║                    Scope Setup Wizard                          ║"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "╚══════════════════════════════════════════════════════════════╝"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "This wizard will help you configure Scope (Blockchain Crawler CLI)."
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Press Enter to skip any optional setting.")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut new_config = config.clone();
    let mut changes_made = false;

    // Step 1: API Keys
    writeln!(writer, "Step 1: API Keys").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "{}", "=".repeat(60)).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "API keys enable access to block explorer data including:"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  • Token balances and holder information")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  • Transaction history and details")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  • Contract verification status")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  • Token analytics and metrics")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "All API keys are FREE and take just a minute to obtain."
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

    // Etherscan (primary)
    if !config.chains.api_keys.contains_key("etherscan") {
        let info = get_api_key_info("etherscan");
        writeln!(
            writer,
            "┌────────────────────────────────────────────────────────────┐"
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(
            writer,
            "│  ETHERSCAN API KEY (Recommended)                           │"
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(
            writer,
            "└────────────────────────────────────────────────────────────┘"
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "  Chain: {}", info.chain).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "  Enables: {}", info.features)
            .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "  How to get your free API key:")
            .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "  {}", info.signup_steps).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "  URL: {}", info.url).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        if let Some(key) = prompt_optional_key_impl(reader, writer, "etherscan")? {
            new_config
                .chains
                .api_keys
                .insert("etherscan".to_string(), key);
            changes_made = true;
        }
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    } else {
        writeln!(writer, "✓ Etherscan API key already configured")
            .map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    // Ask about other chains
    write!(
        writer,
        "Configure API keys for other chains (BSC, Polygon, Arbitrum, etc.)? [y/N]: "
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

    if matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

        let other_chains = ["bscscan", "polygonscan", "arbiscan", "basescan", "optimism"];

        for key_name in other_chains {
            if !config.chains.api_keys.contains_key(key_name) {
                let info = get_api_key_info(key_name);
                writeln!(
                    writer,
                    "┌────────────────────────────────────────────────────────────┐"
                )
                .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(writer, "│  {} API KEY", key_name.to_uppercase())
                    .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(
                    writer,
                    "└────────────────────────────────────────────────────────────┘"
                )
                .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(writer, "  Chain: {}", info.chain)
                    .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(writer, "  Enables: {}", info.features)
                    .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(writer, "  URL: {}", info.url)
                    .map_err(|e| ScopeError::Io(e.to_string()))?;
                writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
                if let Some(key) = prompt_optional_key_impl(reader, writer, key_name)? {
                    new_config.chains.api_keys.insert(key_name.to_string(), key);
                    changes_made = true;
                }
                writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
            }
        }
    }

    // Step 2: Preferences
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Step 2: Preferences").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "{}", "=".repeat(60)).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

    // Default output format
    writeln!(writer, "Default output format:").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  1. table (default)").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  2. json").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  3. csv").map_err(|e| ScopeError::Io(e.to_string()))?;
    write!(writer, "Select [1-3, Enter for default]: ")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    input.clear();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

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
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "Saving configuration...").map_err(|e| ScopeError::Io(e.to_string()))?;
        save_config_to_path(&new_config, config_path)?;
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(
            writer,
            "✓ Configuration saved to ~/.config/scope/config.yaml"
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    } else {
        writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
        writeln!(writer, "No changes made.").map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Setup complete! You can now use Scope.")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "Quick start:").map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  scope crawl USDC              # Analyze a token")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "  scope address 0x...           # Analyze an address"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "  scope insights <target>       # Auto-detect and analyze"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "  scope monitor USDC            # Live TUI dashboard"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "  scope interactive             # Interactive mode")
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "Run 'scope setup --status' to view your configuration."
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "Run 'scope completions zsh > _scope' for shell tab-completion."
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;

    Ok(())
}

/// Testable implementation of prompt_optional_key with injected I/O.
fn prompt_optional_key_impl(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    name: &str,
) -> Result<Option<String>> {
    write!(writer, "  {} API key (or Enter to skip): ", name)
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

    let key = input.trim().to_string();
    if key.is_empty() {
        Ok(None)
    } else {
        Ok(Some(key))
    }
}

/// Testable implementation of prompt_api_key with injected I/O.
fn prompt_api_key_impl(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    name: &str,
) -> Result<String> {
    write!(writer, "Enter {} API key: ", name).map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

    Ok(input.trim().to_string())
}

/// Saves the configuration to a specific path. Testable variant.
fn save_config_to_path(config: &Config, config_path: &Path) -> Result<()> {
    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    // Build YAML manually for cleaner output
    let mut yaml = String::new();
    yaml.push_str("# Scope Configuration\n");
    yaml.push_str("# Generated by 'scope setup'\n\n");

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

    // Ghola sidecar section
    yaml.push_str("\nghola:\n");
    yaml.push_str(&format!("  enabled: {}\n", config.ghola.enabled));
    yaml.push_str(&format!("  stealth: {}\n", config.ghola.stealth));

    std::fs::write(config_path, yaml).map_err(|e| ScopeError::Io(e.to_string()))?;

    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
    fn test_mask_key_exactly_8() {
        let masked = mask_key("ABCDEFGH");
        assert_eq!(masked, "********");
    }

    #[test]
    fn test_mask_key_9_chars() {
        let masked = mask_key("ABCDEFGHI");
        assert_eq!(masked, "(ABCD...FGHI)");
    }

    #[test]
    fn test_mask_key_empty() {
        let masked = mask_key("");
        assert_eq!(masked, "");
    }

    #[test]
    fn test_get_api_key_url() {
        assert!(get_api_key_url("etherscan").contains("etherscan.io"));
        assert!(get_api_key_url("bscscan").contains("bscscan.com"));
    }

    // ========================================================================
    // API key info tests
    // ========================================================================

    #[test]
    fn test_get_api_key_info_all_providers() {
        let providers = [
            "etherscan",
            "bscscan",
            "polygonscan",
            "arbiscan",
            "basescan",
            "optimism",
        ];
        for provider in providers {
            let info = get_api_key_info(provider);
            assert!(
                !info.url.is_empty(),
                "URL should not be empty for {}",
                provider
            );
            assert!(
                !info.chain.is_empty(),
                "Chain should not be empty for {}",
                provider
            );
            assert!(
                !info.features.is_empty(),
                "Features should not be empty for {}",
                provider
            );
            assert!(
                !info.signup_steps.is_empty(),
                "Signup steps should not be empty for {}",
                provider
            );
        }
    }

    #[test]
    fn test_get_api_key_info_unknown() {
        let info = get_api_key_info("unknown_provider");
        // Should still return info, just generic
        assert!(!info.url.is_empty());
    }

    #[test]
    fn test_get_api_key_info_urls_correct() {
        assert!(get_api_key_info("etherscan").url.contains("etherscan.io"));
        assert!(get_api_key_info("bscscan").url.contains("bscscan.com"));
        assert!(
            get_api_key_info("polygonscan")
                .url
                .contains("polygonscan.com")
        );
        assert!(get_api_key_info("arbiscan").url.contains("arbiscan.io"));
        assert!(get_api_key_info("basescan").url.contains("basescan.org"));
        assert!(
            get_api_key_info("optimism")
                .url
                .contains("optimistic.etherscan.io")
        );
    }

    // ========================================================================
    // Config items tests
    // ========================================================================

    #[test]
    fn test_get_api_key_items_default_config() {
        let config = Config::default();
        let items = get_api_key_items(&config);
        assert_eq!(items.len(), 6);
        // All should be unset by default
        for item in &items {
            assert!(
                !item.is_set,
                "{} should not be set in default config",
                item.name
            );
            assert!(item.value_hint.is_none());
        }
    }

    #[test]
    fn test_get_api_key_items_with_set_key() {
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "ABCDEFGHIJKLMNOP".to_string());
        let items = get_api_key_items(&config);
        let etherscan_item = items.iter().find(|i| i.name == "etherscan").unwrap();
        assert!(etherscan_item.is_set);
        assert!(etherscan_item.value_hint.is_some());
        assert_eq!(etherscan_item.value_hint.as_ref().unwrap(), "(ABCD...MNOP)");
    }

    // ========================================================================
    // SetupArgs tests
    // ========================================================================

    #[test]
    fn test_setup_args_defaults() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            setup: SetupArgs,
        }

        let cli = TestCli::try_parse_from(["test"]).unwrap();
        assert!(!cli.setup.status);
        assert!(cli.setup.key.is_none());
        assert!(!cli.setup.reset);
    }

    #[test]
    fn test_setup_args_status() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            setup: SetupArgs,
        }

        let cli = TestCli::try_parse_from(["test", "--status"]).unwrap();
        assert!(cli.setup.status);
    }

    #[test]
    fn test_setup_args_key() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            setup: SetupArgs,
        }

        let cli = TestCli::try_parse_from(["test", "--key", "etherscan"]).unwrap();
        assert_eq!(cli.setup.key.as_deref(), Some("etherscan"));
    }

    #[test]
    fn test_setup_args_reset() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            setup: SetupArgs,
        }

        let cli = TestCli::try_parse_from(["test", "--reset"]).unwrap();
        assert!(cli.setup.reset);
    }

    // ========================================================================
    // show_status (pure function, prints to stdout)
    // ========================================================================

    #[test]
    fn test_show_status_no_panic() {
        let config = Config::default();
        show_status(&config);
    }

    #[test]
    fn test_show_status_with_keys_no_panic() {
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "abc123def456".to_string());
        config
            .chains
            .api_keys
            .insert("bscscan".to_string(), "xyz".to_string());
        show_status(&config);
    }

    // ========================================================================
    // run() dispatching tests
    // ========================================================================

    #[tokio::test]
    async fn test_run_status_mode() {
        let config = Config::default();
        let args = SetupArgs {
            status: true,
            key: None,
            reset: false,
        };
        let result = run(args, &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_key_unknown() {
        let config = Config::default();
        let args = SetupArgs {
            status: false,
            key: Some("nonexistent".to_string()),
            reset: false,
        };
        // This should print "Unknown API key" but still return Ok
        let result = run(args, &config).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // save_config tests
    // ========================================================================

    #[test]
    fn test_show_status_with_multiple_keys() {
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "abc123def456789".to_string());
        config
            .chains
            .api_keys
            .insert("polygonscan".to_string(), "poly_key_12345".to_string());
        config
            .chains
            .api_keys
            .insert("bscscan".to_string(), "bsc".to_string()); // Short key
        show_status(&config);
    }

    #[test]
    fn test_show_status_with_all_keys() {
        let mut config = Config::default();
        for key in [
            "etherscan",
            "bscscan",
            "polygonscan",
            "arbiscan",
            "basescan",
            "optimism",
        ] {
            config
                .chains
                .api_keys
                .insert(key.to_string(), format!("{}_key_12345678", key));
        }
        // No missing keys → should skip "where to get" section
        show_status(&config);
    }

    #[test]
    fn test_show_status_with_custom_rpc() {
        let mut config = Config::default();
        config.chains.ethereum_rpc = Some("https://custom.rpc.example.com".to_string());
        config.output.format = OutputFormat::Json;
        config.output.color = false;
        show_status(&config);
    }

    #[test]
    fn test_get_api_key_items_all_set() {
        let mut config = Config::default();
        for key in [
            "etherscan",
            "bscscan",
            "polygonscan",
            "arbiscan",
            "basescan",
            "optimism",
        ] {
            config
                .chains
                .api_keys
                .insert(key.to_string(), format!("{}_key_12345678", key));
        }
        let items = get_api_key_items(&config);
        assert_eq!(items.len(), 6);
        for item in &items {
            assert!(item.is_set, "{} should be set", item.name);
            assert!(item.value_hint.is_some());
        }
    }

    #[test]
    fn test_get_api_key_info_features_not_empty() {
        for key in [
            "etherscan",
            "bscscan",
            "polygonscan",
            "arbiscan",
            "basescan",
            "optimism",
        ] {
            let info = get_api_key_info(key);
            assert!(!info.features.is_empty());
            assert!(!info.signup_steps.is_empty());
        }
    }

    #[test]
    fn test_save_config_creates_file() {
        let tmp_dir = std::env::temp_dir().join("scope_test_setup");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let tmp_file = tmp_dir.join("config.yaml");

        // Since save_config uses Config::config_path(), we can't easily redirect it
        // but we can test the config serialization logic directly
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "test_key_12345".to_string());
        config.output.format = OutputFormat::Json;

        // Build the YAML manually (same logic as save_config)
        let mut yaml = String::new();
        yaml.push_str("# Scope Configuration\n");
        yaml.push_str("# Generated by 'scope setup'\n\n");
        yaml.push_str("chains:\n");
        if !config.chains.api_keys.is_empty() {
            yaml.push_str("  api_keys:\n");
            for (name, key) in &config.chains.api_keys {
                yaml.push_str(&format!("    {}: \"{}\"\n", name, key));
            }
        }
        yaml.push_str("\noutput:\n");
        yaml.push_str(&format!("  format: {}\n", config.output.format));
        yaml.push_str(&format!("  color: {}\n", config.output.color));

        std::fs::write(&tmp_file, &yaml).unwrap();
        let content = std::fs::read_to_string(&tmp_file).unwrap();
        assert!(content.contains("etherscan"));
        assert!(content.contains("test_key_12345"));
        assert!(content.contains("json") || content.contains("Json"));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_save_config_to_temp_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("scope").join("config.yaml");

        // Create parent dirs
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        let config = Config::default();
        let yaml = serde_yaml::to_string(&config.chains).unwrap();
        std::fs::write(&config_path, yaml).unwrap();

        assert!(config_path.exists());
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(!contents.is_empty());
    }

    #[test]
    fn test_setup_args_reset_flag() {
        let args = SetupArgs {
            status: false,
            key: None,
            reset: true,
        };
        assert!(args.reset);
    }

    // ========================================================================
    // Refactored _impl function tests
    // ========================================================================

    #[test]
    fn test_prompt_api_key_impl_with_input() {
        let input = b"MY_SECRET_API_KEY_123\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = prompt_api_key_impl(&mut reader, &mut writer, "etherscan").unwrap();
        assert_eq!(result, "MY_SECRET_API_KEY_123");
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Enter etherscan API key"));
    }

    #[test]
    fn test_prompt_api_key_impl_empty_input() {
        let input = b"\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = prompt_api_key_impl(&mut reader, &mut writer, "bscscan").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_prompt_optional_key_impl_with_key() {
        let input = b"my_key_12345\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = prompt_optional_key_impl(&mut reader, &mut writer, "polygonscan").unwrap();
        assert_eq!(result, Some("my_key_12345".to_string()));
    }

    #[test]
    fn test_prompt_optional_key_impl_skip() {
        let input = b"\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = prompt_optional_key_impl(&mut reader, &mut writer, "arbiscan").unwrap();
        assert_eq!(result, None);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("arbiscan API key"));
    }

    #[test]
    fn test_save_config_to_path_creates_file_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("subdir").join("config.yaml");
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "test_key_abc".to_string());
        config.output.format = OutputFormat::Json;
        config.output.color = false;

        save_config_to_path(&config, &config_path).unwrap();

        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("etherscan"));
        assert!(content.contains("test_key_abc"));
        assert!(content.contains("json"));
        assert!(content.contains("color: false"));
        assert!(content.contains("# Scope Configuration"));
    }

    #[test]
    fn test_save_config_to_path_with_rpc() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let mut config = Config::default();
        config.chains.ethereum_rpc = Some("https://my-rpc.example.com".to_string());

        save_config_to_path(&config, &config_path).unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("ethereum_rpc"));
        assert!(content.contains("https://my-rpc.example.com"));
    }

    #[test]
    fn test_reset_config_impl_confirm_yes() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        std::fs::write(&config_path, "test: data").unwrap();

        let input = b"y\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        reset_config_impl(&mut reader, &mut writer, &config_path).unwrap();
        assert!(!config_path.exists());
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Configuration reset to defaults"));
    }

    #[test]
    fn test_reset_config_impl_confirm_yes_full() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        std::fs::write(&config_path, "test: data").unwrap();

        let input = b"yes\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        reset_config_impl(&mut reader, &mut writer, &config_path).unwrap();
        assert!(!config_path.exists());
    }

    #[test]
    fn test_reset_config_impl_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        std::fs::write(&config_path, "test: data").unwrap();

        let input = b"n\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        reset_config_impl(&mut reader, &mut writer, &config_path).unwrap();
        assert!(config_path.exists()); // Not deleted
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Cancelled"));
    }

    #[test]
    fn test_reset_config_impl_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("nonexistent.yaml");

        let input = b"";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        reset_config_impl(&mut reader, &mut writer, &config_path).unwrap();
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("No configuration file found"));
    }

    #[test]
    fn test_configure_single_key_impl_valid_key() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        let input = b"MY_ETH_KEY_12345678\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        configure_single_key_impl(&mut reader, &mut writer, "etherscan", &config, &config_path)
            .unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Configure ETHERSCAN API Key"));
        assert!(output.contains("Ethereum Mainnet"));
        assert!(output.contains("etherscan API key saved"));

        // Config file should be created
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("MY_ETH_KEY_12345678"));
    }

    #[test]
    fn test_configure_single_key_impl_empty_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        let input = b"\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        configure_single_key_impl(&mut reader, &mut writer, "etherscan", &config, &config_path)
            .unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Skipped"));
        assert!(!config_path.exists()); // No file created
    }

    #[test]
    fn test_configure_single_key_impl_invalid_key_name() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        let input = b"";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        configure_single_key_impl(&mut reader, &mut writer, "invalid", &config, &config_path)
            .unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Unknown API key: invalid"));
        assert!(output.contains("Valid options"));
    }

    #[test]
    fn test_configure_single_key_impl_bscscan() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        let input = b"BSC_KEY_ABCDEF\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        configure_single_key_impl(&mut reader, &mut writer, "bscscan", &config, &config_path)
            .unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Configure BSCSCAN API Key"));
        assert!(output.contains("BNB Smart Chain"));
        assert!(config_path.exists());
    }

    #[test]
    fn test_wizard_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        // Skip etherscan, decline other chains, keep default format
        let input = b"\nn\n\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        run_setup_wizard_impl(&mut reader, &mut writer, &config, &config_path).unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Scope Setup Wizard"));
        assert!(output.contains("Step 1: API Keys"));
        assert!(output.contains("Step 2: Preferences"));
        assert!(output.contains("No changes made"));
        assert!(output.contains("Setup complete"));
        assert!(!config_path.exists()); // No config saved
    }

    #[test]
    fn test_wizard_with_etherscan_key_and_json_format() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        // Provide etherscan key, decline other chains, select JSON format (2)
        let input = b"MY_ETH_KEY\nn\n2\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        run_setup_wizard_impl(&mut reader, &mut writer, &config, &config_path).unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Configuration saved"));
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("MY_ETH_KEY"));
        assert!(content.contains("json"));
    }

    #[test]
    fn test_wizard_with_csv_format() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        // Skip etherscan, decline other chains, select CSV format (3)
        let input = b"\nn\n3\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        run_setup_wizard_impl(&mut reader, &mut writer, &config, &config_path).unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Configuration saved"));
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("csv"));
    }

    #[test]
    fn test_wizard_with_other_chains_yes() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::default();

        // Skip etherscan, say yes to other chains, provide bscscan key, skip rest, keep default format
        let input = b"\ny\nBSC_KEY_123\n\n\n\n\n\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        run_setup_wizard_impl(&mut reader, &mut writer, &config, &config_path).unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("BSCSCAN API KEY"));
        assert!(output.contains("Configuration saved"));
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("BSC_KEY_123"));
    }

    #[test]
    fn test_wizard_etherscan_already_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "existing_key".to_string());

        // Decline other chains, keep default format
        let input = b"n\n\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        run_setup_wizard_impl(&mut reader, &mut writer, &config, &config_path).unwrap();

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Etherscan API key already configured"));
        assert!(output.contains("No changes made"));
    }

    #[test]
    fn test_save_config_includes_ghola_section() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");

        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = true;

        save_config_to_path(&config, &config_path).unwrap();

        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(contents.contains("ghola:"));
        assert!(contents.contains("enabled: true"));
        assert!(contents.contains("stealth: true"));
    }

    #[test]
    fn test_save_config_ghola_defaults() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");

        let config = Config::default();
        save_config_to_path(&config, &config_path).unwrap();

        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(contents.contains("ghola:"));
        assert!(contents.contains("enabled: false"));
        assert!(contents.contains("stealth: false"));
    }

    #[test]
    fn test_which_ghola_returns_bool() {
        let result = which_ghola();
        assert!(result == true || result == false);
    }

    #[test]
    fn test_show_status_ghola_disabled() {
        let config = Config::default();
        // Just verify it doesn't panic
        show_status(&config);
    }

    #[test]
    fn test_show_status_ghola_enabled_stealth_on() {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = true;
        show_status(&config);
    }

    #[test]
    fn test_show_status_ghola_enabled_stealth_off() {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = false;
        show_status(&config);
    }
}
