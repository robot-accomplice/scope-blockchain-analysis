//! # Interactive Mode
//!
//! This module implements an interactive REPL for the BCC CLI where
//! context is preserved between commands. Users can set a chain once
//! and subsequent commands will use it automatically.
//!
//! ## Usage
//!
//! ```bash
//! bcc interactive
//!
//! bcc> chain solana
//! Chain set to: solana
//!
//! bcc> address 7xKXtg...
//! # Uses solana chain automatically
//! ```

use crate::config::{Config, OutputFormat};
use crate::error::Result;
use clap::Args;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use super::{AddressArgs, CrawlArgs, PortfolioArgs, TxArgs};
use super::{address, crawl, monitor, portfolio, tx};

/// Arguments for the interactive command.
#[derive(Debug, Clone, Args)]
pub struct InteractiveArgs {
    /// Skip displaying the banner on startup.
    #[arg(long)]
    pub no_banner: bool,
}

/// Session context that persists between commands in interactive mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    /// Current blockchain network (default: "ethereum").
    pub chain: String,

    /// Whether the chain was explicitly set by the user (vs. default or auto-inferred).
    #[serde(default)]
    pub chain_explicit: bool,

    /// Current output format.
    pub format: OutputFormat,

    /// Last analyzed address (for quick re-analysis).
    pub last_address: Option<String>,

    /// Last analyzed transaction hash.
    pub last_tx: Option<String>,

    /// Include token balances in address analysis.
    pub include_tokens: bool,

    /// Include transactions in address analysis.
    pub include_txs: bool,

    /// Include internal transactions in tx analysis.
    pub trace: bool,

    /// Decode transaction input data.
    pub decode: bool,

    /// Transaction limit for queries.
    pub limit: u32,
}

impl Default for SessionContext {
    fn default() -> Self {
        Self {
            chain: "ethereum".to_string(),
            chain_explicit: false,
            format: OutputFormat::Table,
            last_address: None,
            last_tx: None,
            include_tokens: false,
            include_txs: false,
            trace: false,
            decode: false,
            limit: 100,
        }
    }
}

impl fmt::Display for SessionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Current Context:")?;
        let chain_status = if self.chain_explicit { "" } else { " (auto)" };
        writeln!(f, "  Chain:          {}{}", self.chain, chain_status)?;
        writeln!(f, "  Format:         {:?}", self.format)?;
        writeln!(f, "  Include Tokens: {}", self.include_tokens)?;
        writeln!(f, "  Include TXs:    {}", self.include_txs)?;
        writeln!(f, "  Trace:          {}", self.trace)?;
        writeln!(f, "  Decode:         {}", self.decode)?;
        writeln!(f, "  Limit:          {}", self.limit)?;
        if let Some(ref addr) = self.last_address {
            writeln!(f, "  Last Address:   {}", addr)?;
        }
        if let Some(ref tx) = self.last_tx {
            writeln!(f, "  Last TX:        {}", tx)?;
        }
        Ok(())
    }
}

impl SessionContext {
    /// Returns the path to the session context file.
    fn context_path() -> Option<PathBuf> {
        dirs::data_dir().map(|p| p.join("bcc").join("session.yaml"))
    }

    /// Loads session context from file, or returns default if not found.
    pub fn load() -> Self {
        Self::context_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|contents| serde_yaml::from_str(&contents).ok())
            .unwrap_or_default()
    }

    /// Saves session context to file.
    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::context_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let contents = serde_yaml::to_string(self)
                .map_err(|e| crate::error::BccError::Export(e.to_string()))?;
            std::fs::write(&path, contents)?;
        }
        Ok(())
    }
}

/// Runs the interactive REPL.
pub async fn run(args: InteractiveArgs, config: &Config) -> Result<()> {
    // Show banner unless disabled
    if !args.no_banner {
        let banner = include_str!("../../assets/banner.txt");
        eprintln!("{}", banner);
    }

    println!("Welcome to BCC Interactive Mode!");
    println!("Type 'help' for available commands, 'exit' to quit.\n");

    // Load previous session context or start fresh
    let mut context = SessionContext::load();

    // Apply config defaults if context is fresh (default chain)
    if context.chain == "ethereum" && context.format == OutputFormat::Table {
        context.format = config.output.format;
    }

    // Create readline editor
    let mut rl = DefaultEditor::new().map_err(|e| {
        crate::error::BccError::Chain(format!("Failed to initialize readline: {}", e))
    })?;

    // Try to load history
    let history_path = dirs::data_dir().map(|p| p.join("bcc").join("history.txt"));
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let prompt = format!("bcc:{}> ", context.chain);

        match rl.readline(&prompt) {
            Ok(input_line) => {
                let line = input_line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Parse and execute command
                match execute_input(line, &mut context, config).await {
                    Ok(should_exit) => {
                        if should_exit {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("exit");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    if let Some(ref path) = history_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.save_history(path);
    }

    // Save session context for next time
    if let Err(e) = context.save() {
        tracing::warn!("Failed to save session context: {}", e);
    }

    println!("Goodbye!");
    Ok(())
}

/// Executes a single input line. Returns Ok(true) if should exit.
async fn execute_input(input: &str, context: &mut SessionContext, config: &Config) -> Result<bool> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(false);
    }

    let command = parts[0].to_lowercase();
    let args = &parts[1..];

    match command.as_str() {
        // Exit commands
        "exit" | "quit" | ".exit" | ".quit" | "q" => {
            return Ok(true);
        }

        // Help
        "help" | "?" | ".help" => {
            print_help();
        }

        // Show context
        "ctx" | "context" | ".ctx" | ".context" => {
            println!("{}", context);
        }

        // Clear/reset context
        "clear" | ".clear" | "reset" | ".reset" => {
            *context = SessionContext::default();
            context.format = config.output.format;
            println!("Context reset to defaults.");
        }

        // Set chain
        "chain" | ".chain" => {
            if args.is_empty() {
                let status = if context.chain_explicit {
                    " (explicit)"
                } else {
                    " (auto-detect enabled)"
                };
                println!("Current chain: {}{}", context.chain, status);
            } else {
                let new_chain = args[0].to_lowercase();
                // Validate chain name
                let valid_chains = [
                    "ethereum", "polygon", "arbitrum", "optimism", "base", "bsc", "aegis",
                    "solana", "tron",
                ];
                if valid_chains.contains(&new_chain.as_str()) {
                    context.chain = new_chain.clone();
                    context.chain_explicit = true;
                    println!("Chain set to: {} (auto-detect disabled)", new_chain);
                } else if new_chain == "auto" {
                    // Special value to re-enable auto-detection
                    context.chain = "ethereum".to_string();
                    context.chain_explicit = false;
                    println!("Chain auto-detection enabled (default: ethereum)");
                } else {
                    eprintln!(
                        "Unknown chain: {}. Valid chains: {}, auto",
                        new_chain,
                        valid_chains.join(", ")
                    );
                }
            }
        }

        // Set format
        "format" | ".format" => {
            if args.is_empty() {
                println!("Current format: {:?}", context.format);
            } else {
                match args[0].to_lowercase().as_str() {
                    "table" => {
                        context.format = OutputFormat::Table;
                        println!("Format set to: table");
                    }
                    "json" => {
                        context.format = OutputFormat::Json;
                        println!("Format set to: json");
                    }
                    "csv" => {
                        context.format = OutputFormat::Csv;
                        println!("Format set to: csv");
                    }
                    other => {
                        eprintln!("Unknown format: {}. Valid formats: table, json, csv", other);
                    }
                }
            }
        }

        // Toggle flags
        "+tokens" | "showtokens" => {
            context.include_tokens = !context.include_tokens;
            println!(
                "Include tokens: {}",
                if context.include_tokens { "on" } else { "off" }
            );
        }

        "+txs" | "showtxs" | "txs" | ".txs" => {
            context.include_txs = !context.include_txs;
            println!(
                "Include transactions: {}",
                if context.include_txs { "on" } else { "off" }
            );
        }

        "trace" | ".trace" => {
            context.trace = !context.trace;
            println!("Trace: {}", if context.trace { "on" } else { "off" });
        }

        "decode" | ".decode" => {
            context.decode = !context.decode;
            println!("Decode: {}", if context.decode { "on" } else { "off" });
        }

        // Set limit
        "limit" | ".limit" => {
            if args.is_empty() {
                println!("Current limit: {}", context.limit);
            } else if let Ok(n) = args[0].parse::<u32>() {
                context.limit = n;
                println!("Limit set to: {}", n);
            } else {
                eprintln!("Invalid limit: {}. Must be a positive integer.", args[0]);
            }
        }

        // Address command
        "address" | "addr" => {
            let addr = if args.is_empty() {
                // Use last address if available
                match &context.last_address {
                    Some(a) => a.clone(),
                    None => {
                        eprintln!("No address specified and no previous address in context.");
                        return Ok(false);
                    }
                }
            } else {
                args[0].to_string()
            };

            // Determine chain: check for inline override first
            let mut chain_override = None;
            for arg in args.iter().skip(1) {
                if arg.starts_with("--chain=") {
                    chain_override = Some(arg.trim_start_matches("--chain=").to_string());
                }
            }

            // If no explicit chain set (context or inline), try to infer from address
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if !context.chain_explicit {
                // Try auto-detection
                if let Some(inferred) = crate::chains::infer_chain_from_address(&addr) {
                    if inferred != context.chain {
                        println!("Auto-detected chain: {}", inferred);
                        // Update context chain (but keep chain_explicit = false)
                        context.chain = inferred.to_string();
                    }
                    inferred.to_string()
                } else {
                    context.chain.clone()
                }
            } else {
                context.chain.clone()
            };

            // Parse additional flags from args
            let mut address_args = AddressArgs {
                address: addr.clone(),
                chain: effective_chain,
                format: Some(context.format),
                include_txs: context.include_txs,
                include_tokens: context.include_tokens,
                limit: context.limit,
            };

            // Check for other inline overrides
            for arg in args.iter().skip(1) {
                if *arg == "--tokens" {
                    address_args.include_tokens = true;
                } else if *arg == "--txs" {
                    address_args.include_txs = true;
                }
            }

            // Update context
            context.last_address = Some(addr);

            // Execute
            address::run(address_args, config).await?;
        }

        // Transaction command
        "tx" | "transaction" => {
            let hash = if args.is_empty() {
                // Use last tx if available
                match &context.last_tx {
                    Some(h) => h.clone(),
                    None => {
                        eprintln!("No transaction hash specified and no previous hash in context.");
                        return Ok(false);
                    }
                }
            } else {
                args[0].to_string()
            };

            // Determine chain: check for inline override first
            let mut chain_override = None;
            for arg in args.iter().skip(1) {
                if arg.starts_with("--chain=") {
                    chain_override = Some(arg.trim_start_matches("--chain=").to_string());
                }
            }

            // If no explicit chain set (context or inline), try to infer from hash
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if !context.chain_explicit {
                // Try auto-detection
                if let Some(inferred) = crate::chains::infer_chain_from_hash(&hash) {
                    if inferred != context.chain {
                        println!("Auto-detected chain: {}", inferred);
                        // Update context chain (but keep chain_explicit = false)
                        context.chain = inferred.to_string();
                    }
                    inferred.to_string()
                } else {
                    context.chain.clone()
                }
            } else {
                context.chain.clone()
            };

            let mut tx_args = TxArgs {
                hash: hash.clone(),
                chain: effective_chain,
                format: Some(context.format),
                trace: context.trace,
                decode: context.decode,
            };

            // Check for other inline overrides
            for arg in args.iter().skip(1) {
                if *arg == "--trace" {
                    tx_args.trace = true;
                } else if *arg == "--decode" {
                    tx_args.decode = true;
                }
            }

            // Update context
            context.last_tx = Some(hash);

            // Execute
            tx::run(tx_args, config).await?;
        }

        // Crawl command for token analytics
        "crawl" | "token" => {
            if args.is_empty() {
                eprintln!(
                    "Usage: crawl <token_address> [--period <1h|24h|7d|30d>] [--report <path>]"
                );
                return Ok(false);
            }

            let token = args[0].to_string();

            // Determine chain: check for inline override first
            let mut chain_override = None;
            let mut period = crawl::Period::Hour24;
            let mut report_path = None;
            let mut no_charts = false;

            let mut i = 1;
            while i < args.len() {
                if args[i].starts_with("--chain=") {
                    chain_override = Some(args[i].trim_start_matches("--chain=").to_string());
                } else if args[i] == "--chain" && i + 1 < args.len() {
                    chain_override = Some(args[i + 1].to_string());
                    i += 1;
                } else if args[i].starts_with("--period=") {
                    let p = args[i].trim_start_matches("--period=");
                    period = match p {
                        "1h" => crawl::Period::Hour1,
                        "24h" => crawl::Period::Hour24,
                        "7d" => crawl::Period::Day7,
                        "30d" => crawl::Period::Day30,
                        _ => crawl::Period::Hour24,
                    };
                } else if args[i] == "--period" && i + 1 < args.len() {
                    period = match args[i + 1] {
                        "1h" => crawl::Period::Hour1,
                        "24h" => crawl::Period::Hour24,
                        "7d" => crawl::Period::Day7,
                        "30d" => crawl::Period::Day30,
                        _ => crawl::Period::Hour24,
                    };
                    i += 1;
                } else if args[i].starts_with("--report=") {
                    report_path = Some(std::path::PathBuf::from(
                        args[i].trim_start_matches("--report="),
                    ));
                } else if args[i] == "--report" && i + 1 < args.len() {
                    report_path = Some(std::path::PathBuf::from(args[i + 1]));
                    i += 1;
                } else if args[i] == "--no-charts" {
                    no_charts = true;
                }
                i += 1;
            }

            // If no explicit chain set, try to infer from token address
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if !context.chain_explicit {
                if let Some(inferred) = crate::chains::infer_chain_from_address(&token) {
                    if inferred != context.chain {
                        println!("Auto-detected chain: {}", inferred);
                        context.chain = inferred.to_string();
                    }
                    inferred.to_string()
                } else {
                    context.chain.clone()
                }
            } else {
                context.chain.clone()
            };

            let crawl_args = CrawlArgs {
                token,
                chain: effective_chain,
                period,
                holders_limit: 10,
                format: context.format,
                no_charts,
                report: report_path,
                yes: false,  // Interactive mode uses prompts
                save: false, // Will prompt if alias should be saved
            };

            crawl::run(crawl_args, config).await?;
        }

        // Portfolio command (pass through to existing)
        "portfolio" | "port" => {
            // Build portfolio args from remaining input
            let portfolio_input = args.join(" ");
            execute_portfolio(&portfolio_input, context, config).await?;
        }

        // Token alias management
        "tokens" | "aliases" => {
            execute_tokens_command(args).await?;
        }

        // Setup/config command
        "setup" | "config" => {
            use super::setup::{SetupArgs, run as setup_run};
            let setup_args = SetupArgs {
                status: args.contains(&"--status") || args.contains(&"-s"),
                key: args
                    .iter()
                    .find(|a| a.starts_with("--key="))
                    .map(|a| a.trim_start_matches("--key=").to_string())
                    .or_else(|| {
                        args.iter()
                            .position(|&a| a == "--key" || a == "-k")
                            .and_then(|i| args.get(i + 1).map(|s| s.to_string()))
                    }),
                reset: args.contains(&"--reset"),
            };
            setup_run(setup_args, config).await?;
        }

        // Live monitor command
        "monitor" | "mon" => {
            let token = args.first().map(|s| s.to_string());
            monitor::run(token, context, config).await?;
        }

        // Unknown command
        _ => {
            eprintln!(
                "Unknown command: {}. Type 'help' for available commands.",
                command
            );
        }
    }

    Ok(false)
}

/// Execute tokens subcommand for managing saved token aliases.
async fn execute_tokens_command(args: &[&str]) -> Result<()> {
    use crate::tokens::TokenAliases;

    let mut aliases = TokenAliases::load();

    if args.is_empty() {
        // List all saved tokens
        let tokens = aliases.list();
        if tokens.is_empty() {
            println!("No saved token aliases.");
            println!("Use 'crawl <token_name> --save' to save a token alias.");
            return Ok(());
        }

        println!("\nSaved Token Aliases\n{}\n", "=".repeat(60));
        println!("{:<10} {:<12} {:<20} Address", "Symbol", "Chain", "Name");
        println!("{}", "-".repeat(80));

        for token in tokens {
            println!(
                "{:<10} {:<12} {:<20} {}",
                token.symbol, token.chain, token.name, token.address
            );
        }
        println!();
        return Ok(());
    }

    let subcommand = args[0].to_lowercase();
    match subcommand.as_str() {
        "list" | "ls" => {
            let tokens = aliases.list();
            if tokens.is_empty() {
                println!("No saved token aliases.");
                return Ok(());
            }

            println!("\nSaved Token Aliases\n{}\n", "=".repeat(60));
            println!("{:<10} {:<12} {:<20} Address", "Symbol", "Chain", "Name");
            println!("{}", "-".repeat(80));

            for token in tokens {
                println!(
                    "{:<10} {:<12} {:<20} {}",
                    token.symbol, token.chain, token.name, token.address
                );
            }
            println!();
        }

        "recent" => {
            let recent = aliases.recent();
            if recent.is_empty() {
                println!("No recently used tokens.");
                return Ok(());
            }

            println!("\nRecently Used Tokens\n{}\n", "=".repeat(60));
            println!("{:<10} {:<12} {:<20} Address", "Symbol", "Chain", "Name");
            println!("{}", "-".repeat(80));

            for token in recent {
                println!(
                    "{:<10} {:<12} {:<20} {}",
                    token.symbol, token.chain, token.name, token.address
                );
            }
            println!();
        }

        "remove" | "rm" | "delete" => {
            if args.len() < 2 {
                eprintln!("Usage: tokens remove <symbol> [--chain <chain>]");
                return Ok(());
            }

            let symbol = args[1];
            let chain = if args.len() > 3 && args[2] == "--chain" {
                Some(args[3])
            } else {
                None
            };

            aliases.remove(symbol, chain);
            if let Err(e) = aliases.save() {
                eprintln!("Failed to save: {}", e);
            } else {
                println!("Removed alias: {}", symbol);
            }
        }

        "add" => {
            if args.len() < 4 {
                eprintln!("Usage: tokens add <symbol> <chain> <address> [name]");
                return Ok(());
            }

            let symbol = args[1];
            let chain = args[2];
            let address = args[3];
            let name = if args.len() > 4 {
                args[4..].join(" ")
            } else {
                symbol.to_string()
            };

            aliases.add(symbol, chain, address, &name);
            if let Err(e) = aliases.save() {
                eprintln!("Failed to save: {}", e);
            } else {
                println!("Added alias: {} -> {} on {}", symbol, address, chain);
            }
        }

        _ => {
            eprintln!("Unknown tokens subcommand: {}", subcommand);
            eprintln!("Available: list, recent, add, remove");
        }
    }

    Ok(())
}

/// Execute portfolio subcommand
async fn execute_portfolio(input: &str, context: &SessionContext, config: &Config) -> Result<()> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        eprintln!("Portfolio subcommand required: add, remove, list, summary");
        return Ok(());
    }

    use super::portfolio::{AddArgs, PortfolioCommands, RemoveArgs, SummaryArgs};

    let subcommand = parts[0].to_lowercase();

    let portfolio_args = match subcommand.as_str() {
        "add" => {
            if parts.len() < 2 {
                eprintln!("Usage: portfolio add <address> [--label <label>] [--tags <tags>]");
                return Ok(());
            }
            let address = parts[1].to_string();
            let mut label = None;
            let mut tags = Vec::new();

            let mut i = 2;
            while i < parts.len() {
                if parts[i] == "--label" && i + 1 < parts.len() {
                    label = Some(parts[i + 1].to_string());
                    i += 2;
                } else if parts[i] == "--tags" && i + 1 < parts.len() {
                    tags = parts[i + 1]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect();
                    i += 2;
                } else {
                    i += 1;
                }
            }

            PortfolioArgs {
                command: PortfolioCommands::Add(AddArgs {
                    address,
                    label,
                    chain: context.chain.clone(),
                    tags,
                }),
                format: Some(context.format),
            }
        }
        "remove" | "rm" => {
            if parts.len() < 2 {
                eprintln!("Usage: portfolio remove <address>");
                return Ok(());
            }
            PortfolioArgs {
                command: PortfolioCommands::Remove(RemoveArgs {
                    address: parts[1].to_string(),
                }),
                format: Some(context.format),
            }
        }
        "list" | "ls" => PortfolioArgs {
            command: PortfolioCommands::List,
            format: Some(context.format),
        },
        "summary" => {
            let mut chain = None;
            let mut tag = None;
            let mut include_tokens = context.include_tokens;

            let mut i = 1;
            while i < parts.len() {
                if parts[i] == "--chain" && i + 1 < parts.len() {
                    chain = Some(parts[i + 1].to_string());
                    i += 2;
                } else if parts[i] == "--tag" && i + 1 < parts.len() {
                    tag = Some(parts[i + 1].to_string());
                    i += 2;
                } else if parts[i] == "--tokens" {
                    include_tokens = true;
                    i += 1;
                } else {
                    i += 1;
                }
            }

            PortfolioArgs {
                command: PortfolioCommands::Summary(SummaryArgs {
                    chain,
                    tag,
                    include_tokens,
                }),
                format: Some(context.format),
            }
        }
        _ => {
            eprintln!(
                "Unknown portfolio subcommand: {}. Use: add, remove, list, summary",
                subcommand
            );
            return Ok(());
        }
    };

    portfolio::run(portfolio_args, config).await
}

/// Print help message for interactive mode.
fn print_help() {
    println!(
        r#"
BCC Interactive Mode - Available Commands
==========================================

Navigation & Control:
  help, ?           Show this help message
  exit, quit, q     Exit interactive mode
  ctx, context      Show current session context
  clear, reset      Reset context to defaults

Context Settings:
  chain [name]      Set or show current chain
                    Valid: ethereum, polygon, arbitrum, optimism, base, bsc, aegis, solana, tron
  format [fmt]      Set or show output format (table, json, csv)
  limit [n]         Set or show transaction limit
  +tokens           Toggle include_tokens flag for address analysis
  +txs              Toggle include_txs flag
  trace             Toggle trace flag
  decode            Toggle decode flag

Analysis Commands:
  address <addr>    Analyze an address (uses current chain/format)
  addr              Shorthand for address
  tx <hash>         Analyze a transaction (uses current chain/format)
  crawl <token>     Crawl token analytics (holders, volume, price)
  token             Shorthand for crawl
  monitor <token>   Live-updating charts for a token (TUI mode)
  mon               Shorthand for monitor

Token Search:
  crawl USDC        Search for token by name/symbol (interactive selection)
  crawl 0x...       Use address directly (no search)
  tokens            List saved token aliases
  tokens recent     Show recently used tokens
  tokens add <sym> <chain> <addr> [name]    Add a token alias
  tokens remove <sym> [--chain <chain>]     Remove a token alias

Portfolio Commands:
  portfolio add <addr> [--label <name>] [--tags <t1,t2>]
  portfolio remove <addr>
  portfolio list
  portfolio summary [--chain <name>] [--tag <tag>] [--tokens]

Configuration:
  setup             Run the setup wizard to configure API keys
  setup --status    Show current configuration status
  setup --key <provider>    Configure a specific API key
  config            Alias for setup

Inline Overrides:
  address 0x... --chain=polygon --tokens
  tx 0x... --chain=arbitrum --trace --decode
  crawl USDC --chain=ethereum --period=7d --report=report.md

Live Monitor:
  monitor USDC      Start live monitoring with real-time charts
  mon 0x...         Monitor by address
  Time periods: [1]=15m [2]=1h [3]=6h [4]=24h [T]=cycle
  Chart modes: [C]=toggle between Line and Candlestick
  Controls: [Q]uit [R]efresh [P]ause [+/-]speed [Esc]exit
  Data is cached to temp folder and persists between sessions (24h retention)

Tips:
  - Search by token name: 'crawl WETH' or 'crawl "wrapped ether"'
  - Save aliases for quick access: select a token and choose to save
  - Context persists: set chain once, use it for multiple commands
  - Use Ctrl+C to cancel, Ctrl+D to exit
"#
    );
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_context_default() {
        let ctx = SessionContext::default();
        assert_eq!(ctx.chain, "ethereum");
        assert_eq!(ctx.format, OutputFormat::Table);
        assert!(!ctx.include_tokens);
        assert!(!ctx.include_txs);
        assert!(!ctx.trace);
        assert!(!ctx.decode);
        assert_eq!(ctx.limit, 100);
        assert!(ctx.last_address.is_none());
        assert!(ctx.last_tx.is_none());
    }

    #[test]
    fn test_session_context_display() {
        let ctx = SessionContext::default();
        let display = format!("{}", ctx);
        assert!(display.contains("ethereum"));
        assert!(display.contains("Table"));
    }

    #[test]
    fn test_interactive_args_default() {
        let args = InteractiveArgs { no_banner: false };
        assert!(!args.no_banner);
    }
}
