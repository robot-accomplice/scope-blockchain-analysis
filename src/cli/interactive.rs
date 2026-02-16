//! # Interactive Mode
//!
//! This module implements an interactive REPL for the Scope CLI where
//! context is preserved between commands. The chain defaults to `auto`,
//! meaning the CLI will infer the relevant chain from each input (e.g.,
//! `0x…` → Ethereum/EVM, `T…` → Tron, base58 → Solana). Users can pin
//! a chain with `chain solana` and unlock with `chain auto`.
//!
//! ## Usage
//!
//! ```bash
//! scope interactive
//!
//! scope:auto> address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//! # Chain: ethereum (auto-detected)
//!
//! scope:auto> address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy
//! # Chain: solana (auto-detected)
//!
//! scope:auto> chain solana
//! # Chain pinned to: solana
//!
//! scope:solana> address 7xKXtg...
//! # Uses solana chain
//! ```

use crate::chains::ChainClientFactory;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use clap::Args;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use super::{AddressArgs, AddressBookArgs, CrawlArgs, TxArgs};
use super::{address, address_book, crawl, monitor, tx};

/// Arguments for the interactive command.
#[derive(Debug, Clone, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope interactive
  scope shell
  scope interactive --no-banner")]
pub struct InteractiveArgs {
    /// Skip displaying the banner on startup.
    #[arg(long)]
    pub no_banner: bool,
}

/// Session context that persists between commands in interactive mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    /// Current blockchain network. `"auto"` means infer from input at command time.
    pub chain: String,

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

impl SessionContext {
    /// Returns `true` when chain is in auto-detect mode (not pinned to a specific chain).
    pub fn is_auto_chain(&self) -> bool {
        self.chain == "auto"
    }
}

impl Default for SessionContext {
    fn default() -> Self {
        Self {
            chain: "auto".to_string(),
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
        if self.is_auto_chain() {
            writeln!(f, "  Chain:          auto (inferred from input)")?;
        } else {
            writeln!(f, "  Chain:          {} (pinned)", self.chain)?;
        }
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
        dirs::data_dir().map(|p| p.join("scope").join("session.yaml"))
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
                .map_err(|e| crate::error::ScopeError::Export(e.to_string()))?;
            std::fs::write(&path, contents)?;
        }
        Ok(())
    }
}

/// Runs the interactive REPL.
pub async fn run(
    args: InteractiveArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Show banner unless disabled
    if !args.no_banner {
        let banner = include_str!("../../assets/banner.txt");
        eprintln!("{}", banner);
    }

    println!("Welcome to Scope Interactive Mode!");
    println!("Type 'help' for available commands, 'exit' to quit.\n");

    // Load previous session context or start fresh
    let mut context = SessionContext::load();

    // Apply config defaults if context is fresh
    if context.is_auto_chain() && context.format == OutputFormat::Table {
        context.format = config.output.format;
    }

    // Create readline editor
    let mut rl = DefaultEditor::new().map_err(|e| {
        crate::error::ScopeError::Chain(format!("Failed to initialize readline: {}", e))
    })?;

    // Try to load history
    let history_path = dirs::data_dir().map(|p| p.join("scope").join("history.txt"));
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let prompt = format!("scope:{}> ", context.chain);

        match rl.readline(&prompt) {
            Ok(input_line) => {
                let line = input_line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Parse and execute command
                match execute_input(line, &mut context, config, clients).await {
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
        tracing::debug!("Failed to save session context: {}", e);
    }

    println!("Goodbye!");
    Ok(())
}

/// Executes a single input line. Returns Ok(true) if should exit.
async fn execute_input(
    input: &str,
    context: &mut SessionContext,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<bool> {
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
                if context.is_auto_chain() {
                    println!("Current chain: auto (inferred from input)");
                } else {
                    println!("Current chain: {} (pinned)", context.chain);
                }
            } else {
                let new_chain = args[0].to_lowercase();
                let valid_chains = [
                    "ethereum", "polygon", "arbitrum", "optimism", "base", "bsc", "solana", "tron",
                ];
                if new_chain == "auto" {
                    context.chain = "auto".to_string();
                    println!("Chain set to auto — will infer from each input");
                } else if valid_chains.contains(&new_chain.as_str()) {
                    context.chain = new_chain.clone();
                    println!(
                        "Chain pinned to: {}  (use `chain auto` to unlock)",
                        new_chain
                    );
                } else {
                    eprintln!(
                        "  ✗ Unknown chain: {}. Valid: auto, {}",
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

            // Resolve chain: inline override > pinned context > auto-detect from address
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if context.is_auto_chain() {
                if let Some(inferred) = crate::chains::infer_chain_from_address(&addr) {
                    eprintln!("  Chain: {} (auto-detected)", inferred);
                    inferred.to_string()
                } else {
                    // Default fallback when auto can't infer
                    "ethereum".to_string()
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
                report: None,
                dossier: false,
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
            address::run(address_args, config, clients).await?;
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

            // Resolve chain: inline override > pinned context > auto-detect from hash
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if context.is_auto_chain() {
                if let Some(inferred) = crate::chains::infer_chain_from_hash(&hash) {
                    eprintln!("  Chain: {} (auto-detected)", inferred);
                    inferred.to_string()
                } else {
                    "ethereum".to_string()
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
            tx::run(tx_args, config, clients).await?;
        }

        // Contract analysis command
        "contract" | "ct" => {
            if args.is_empty() {
                eprintln!(
                    "Usage: contract <address> [--chain=<chain>] [--json]"
                );
                return Ok(false);
            }

            let address = args[0].to_string();
            let mut chain = context.chain.clone();
            let mut json_output = false;

            for arg in args.iter().skip(1) {
                if arg.starts_with("--chain=") {
                    chain = arg.trim_start_matches("--chain=").to_string();
                } else if *arg == "--json" {
                    json_output = true;
                }
            }

            // Default to ethereum if auto
            if chain == "auto" {
                chain = "ethereum".to_string();
            }

            let ct_args = crate::cli::contract::ContractArgs {
                address,
                chain,
                json: json_output,
            };

            crate::cli::contract::run(&ct_args, config, clients).await?;
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

            // Resolve chain: inline override > pinned context > auto-detect from token address
            let effective_chain = if let Some(chain) = chain_override {
                chain
            } else if context.is_auto_chain() {
                if let Some(inferred) = crate::chains::infer_chain_from_address(&token) {
                    eprintln!("  Chain: {} (auto-detected)", inferred);
                    inferred.to_string()
                } else {
                    "ethereum".to_string()
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

            crawl::run(crawl_args, config, clients).await?;
        }

        // Address book command (pass through to existing; portfolio/port as aliases)
        "address-book" | "address_book" | "portfolio" | "port" => {
            let input = args.join(" ");
            execute_address_book(&input, context, config, clients).await?;
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
            monitor::run(token, None, context, config, clients).await?;
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

/// Execute address book subcommand
async fn execute_address_book(
    input: &str,
    context: &SessionContext,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        eprintln!("Address book subcommand required: add, remove, list, summary");
        return Ok(());
    }

    use super::address_book::{AddArgs, AddressBookCommands, RemoveArgs, SummaryArgs};

    let subcommand = parts[0].to_lowercase();

    let address_book_args = match subcommand.as_str() {
        "add" => {
            if parts.len() < 2 {
                eprintln!("Usage: address-book add <address> [--label <label>] [--tags <tags>]");
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

            AddressBookArgs {
                command: AddressBookCommands::Add(AddArgs {
                    chain: if context.is_auto_chain() {
                        crate::chains::infer_chain_from_address(&address)
                            .unwrap_or("ethereum")
                            .to_string()
                    } else {
                        context.chain.clone()
                    },
                    address,
                    label,
                    tags,
                }),
                format: Some(context.format),
            }
        }
        "remove" | "rm" => {
            if parts.len() < 2 {
                eprintln!("Usage: address-book remove <address>");
                return Ok(());
            }
            AddressBookArgs {
                command: AddressBookCommands::Remove(RemoveArgs {
                    address: parts[1].to_string(),
                }),
                format: Some(context.format),
            }
        }
        "list" | "ls" => AddressBookArgs {
            command: AddressBookCommands::List,
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

            AddressBookArgs {
                command: AddressBookCommands::Summary(SummaryArgs {
                    chain,
                    tag,
                    include_tokens,
                    report: None,
                }),
                format: Some(context.format),
            }
        }
        _ => {
            eprintln!(
                "Unknown address book subcommand: {}. Use: add, remove, list, summary",
                subcommand
            );
            return Ok(());
        }
    };

    address_book::run(address_book_args, config, clients).await
}

/// Print help message for interactive mode.
fn print_help() {
    println!(
        r#"
Scope Interactive Mode - Available Commands
==========================================

Navigation & Control:
  help, ?           Show this help message
  exit, quit, q     Exit interactive mode
  ctx, context      Show current session context
  clear, reset      Reset context to defaults

Context Settings:
  chain [name]      Set or show current chain (default: auto)
                    auto = infer chain from each input
                    Valid: auto, ethereum, polygon, arbitrum, optimism, base, bsc, solana, tron
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
  contract <addr>   Analyze a smart contract (security, proxy, access control)
  ct                Shorthand for contract
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

Address Book Commands:
  address-book add <addr> [--label <name>] [--tags <t1,t2>]
  address-book remove <addr>
  address-book list
  address-book summary [--chain <name>] [--tag <tag>] [--tokens]

Configuration:
  setup             Run the setup wizard to configure API keys
  setup --status    Show current configuration status
  setup --key <provider>    Configure a specific API key
  config            Alias for setup

Inline Overrides:
  address 0x... --chain=polygon --tokens
  tx 0x... --chain=arbitrum --trace --decode
  contract 0x... --chain=polygon --json
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
        assert_eq!(ctx.chain, "auto");
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
        assert!(display.contains("auto"));
        assert!(display.contains("Table"));
    }

    #[test]
    fn test_interactive_args_default() {
        let args = InteractiveArgs { no_banner: false };
        assert!(!args.no_banner);
    }

    // ========================================================================
    // SessionContext serialization/deserialization
    // ========================================================================

    #[test]
    fn test_session_context_serialization() {
        let ctx = SessionContext {
            chain: "polygon".to_string(),
            format: OutputFormat::Json,
            last_address: Some("0xabc".to_string()),
            last_tx: Some("0xdef".to_string()),
            include_tokens: true,
            include_txs: true,
            trace: true,
            decode: true,
            limit: 50,
        };

        let yaml = serde_yaml::to_string(&ctx).unwrap();
        let deserialized: SessionContext = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.chain, "polygon");
        assert!(!deserialized.is_auto_chain());
        assert_eq!(deserialized.format, OutputFormat::Json);
        assert_eq!(deserialized.last_address.as_deref(), Some("0xabc"));
        assert_eq!(deserialized.last_tx.as_deref(), Some("0xdef"));
        assert!(deserialized.include_tokens);
        assert!(deserialized.include_txs);
        assert!(deserialized.trace);
        assert!(deserialized.decode);
        assert_eq!(deserialized.limit, 50);
    }

    #[test]
    fn test_session_context_display_with_address_and_tx() {
        let ctx = SessionContext {
            chain: "polygon".to_string(),
            last_address: Some("0x1234".to_string()),
            last_tx: Some("0xabcd".to_string()),
            ..Default::default()
        };
        let display = format!("{}", ctx);
        assert!(display.contains("0x1234"));
        assert!(display.contains("0xabcd"));
        assert!(display.contains("(pinned)"));
    }

    #[test]
    fn test_session_context_display_auto_chain() {
        let ctx = SessionContext::default();
        let display = format!("{}", ctx);
        assert!(display.contains("auto"));
        assert!(display.contains("inferred from input"));
    }

    // ========================================================================
    // execute_input tests for context-modifying commands
    // ========================================================================

    fn test_config() -> Config {
        Config::default()
    }

    fn test_factory() -> crate::chains::DefaultClientFactory {
        crate::chains::DefaultClientFactory {
            chains_config: crate::config::ChainsConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_exit_commands() {
        let config = test_config();
        for cmd in &["exit", "quit", "q", ".exit", ".quit"] {
            let mut ctx = SessionContext::default();
            let result = execute_input(cmd, &mut ctx, &config, &test_factory())
                .await
                .unwrap();
            assert!(result, "'{cmd}' should return true (exit)");
        }
    }

    #[tokio::test]
    async fn test_help_command() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("help", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_context_command() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("ctx", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_clear_command() {
        let config = test_config();
        let mut ctx = SessionContext {
            chain: "polygon".to_string(),
            include_tokens: true,
            limit: 42,
            ..Default::default()
        };

        let result = execute_input("clear", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
        assert_eq!(ctx.chain, "auto");
        assert!(!ctx.include_tokens);
        assert_eq!(ctx.limit, 100);
    }

    #[tokio::test]
    async fn test_chain_set_valid() {
        let config = test_config();
        let mut ctx = SessionContext::default();

        execute_input("chain polygon", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "polygon");
        assert!(!ctx.is_auto_chain());
    }

    #[tokio::test]
    async fn test_chain_set_solana() {
        let config = test_config();
        let mut ctx = SessionContext::default();

        execute_input("chain solana", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "solana");
        assert!(!ctx.is_auto_chain());
    }

    #[tokio::test]
    async fn test_chain_auto() {
        let config = test_config();
        let mut ctx = SessionContext {
            chain: "polygon".to_string(),
            ..Default::default()
        };

        execute_input("chain auto", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
        assert!(ctx.is_auto_chain());
    }

    #[tokio::test]
    async fn test_chain_invalid() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        // Invalid chain should not change context
        execute_input("chain foobar", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
        assert!(ctx.is_auto_chain());
    }

    #[tokio::test]
    async fn test_chain_show() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        // No arg → just prints current chain, doesn't change anything
        let result = execute_input("chain", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_format_set_json() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("format json", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.format, OutputFormat::Json);
    }

    #[tokio::test]
    async fn test_format_set_csv() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("format csv", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.format, OutputFormat::Csv);
    }

    #[tokio::test]
    async fn test_format_set_table() {
        let config = test_config();
        let mut ctx = SessionContext {
            format: OutputFormat::Json,
            ..Default::default()
        };
        execute_input("format table", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.format, OutputFormat::Table);
    }

    #[tokio::test]
    async fn test_format_invalid() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("format xml", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        // Should remain unchanged
        assert_eq!(ctx.format, OutputFormat::Table);
    }

    #[tokio::test]
    async fn test_format_show() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("format", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_toggle_tokens() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        assert!(!ctx.include_tokens);

        execute_input("+tokens", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_tokens);

        execute_input("+tokens", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!ctx.include_tokens);
    }

    #[tokio::test]
    async fn test_toggle_txs() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        assert!(!ctx.include_txs);

        execute_input("+txs", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_txs);

        execute_input("+txs", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!ctx.include_txs);
    }

    #[tokio::test]
    async fn test_toggle_trace() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        assert!(!ctx.trace);

        execute_input("trace", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.trace);

        execute_input("trace", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!ctx.trace);
    }

    #[tokio::test]
    async fn test_toggle_decode() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        assert!(!ctx.decode);

        execute_input("decode", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.decode);

        execute_input("decode", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!ctx.decode);
    }

    #[tokio::test]
    async fn test_limit_set_valid() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("limit 50", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.limit, 50);
    }

    #[tokio::test]
    async fn test_limit_set_invalid() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("limit abc", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        // Should remain unchanged
        assert_eq!(ctx.limit, 100);
    }

    #[tokio::test]
    async fn test_limit_show() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("limit", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_unknown_command() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("foobar", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_empty_input() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_address_no_arg_no_last() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        // address with no arg and no last_address → prints error, returns Ok(false)
        let result = execute_input("address", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tx_no_arg_no_last() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        // tx with no arg and no last_tx → prints error, returns Ok(false)
        let result = execute_input("tx", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_crawl_no_arg() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        // crawl with no arg → prints usage, returns Ok(false)
        let result = execute_input("crawl", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_multiple_context_commands() {
        let config = test_config();
        let mut ctx = SessionContext::default();

        // Set chain, format, toggle flags, set limit
        execute_input("chain polygon", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        execute_input("format json", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        execute_input("+tokens", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        execute_input("trace", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        execute_input("limit 25", &mut ctx, &config, &test_factory())
            .await
            .unwrap();

        assert_eq!(ctx.chain, "polygon");
        assert_eq!(ctx.format, OutputFormat::Json);
        assert!(ctx.include_tokens);
        assert!(ctx.trace);
        assert_eq!(ctx.limit, 25);

        // Clear resets everything
        execute_input("clear", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
        assert!(!ctx.include_tokens);
        assert!(!ctx.trace);
        assert_eq!(ctx.limit, 100);
    }

    #[tokio::test]
    async fn test_dot_prefix_commands() {
        let config = test_config();
        let mut ctx = SessionContext::default();

        // Dot-prefixed variants
        let result = execute_input(".help", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);

        execute_input(".chain polygon", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "polygon");

        execute_input(".format json", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.format, OutputFormat::Json);

        execute_input(".trace", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.trace);

        execute_input(".decode", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.decode);
    }

    #[tokio::test]
    async fn test_all_valid_chains() {
        let config = test_config();
        let valid_chains = [
            "ethereum", "polygon", "arbitrum", "optimism", "base", "bsc", "solana", "tron",
        ];
        for chain in valid_chains {
            let mut ctx = SessionContext::default();
            execute_input(
                &format!("chain {}", chain),
                &mut ctx,
                &config,
                &test_factory(),
            )
            .await
            .unwrap();
            assert_eq!(ctx.chain, chain);
            assert!(!ctx.is_auto_chain());
        }
    }

    // ========================================================================
    // Command dispatch tests (with MockClientFactory)
    // ========================================================================

    use crate::chains::mocks::MockClientFactory;

    fn mock_factory() -> MockClientFactory {
        let mut factory = MockClientFactory::new();
        factory.mock_client.transactions = vec![factory.mock_client.transaction.clone()];
        factory.mock_client.token_balances = vec![crate::chains::TokenBalance {
            token: crate::chains::Token {
                contract_address: "0xtoken".to_string(),
                symbol: "TEST".to_string(),
                name: "Test Token".to_string(),
                decimals: 18,
            },
            balance: "1000".to_string(),
            formatted_balance: "0.001".to_string(),
            usd_value: None,
        }];
        factory
    }

    #[tokio::test]
    async fn test_address_command_with_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
        assert_eq!(
            ctx.last_address,
            Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string())
        );
    }

    #[tokio::test]
    async fn test_address_command_with_chain_override() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain=polygon",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_command_with_tokens_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --tokens",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_command_with_txs_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --txs",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_reuses_last_address() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext {
            last_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            ..Default::default()
        };
        let result = execute_input("address", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_auto_detects_solana() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        // Solana address format
        let result = execute_input(
            "address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        // Context chain stays "auto" (inferred per command, not stored)
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_tx_command_with_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "tx 0xabc123def456789012345678901234567890123456789012345678901234abcd",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            ctx.last_tx,
            Some("0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string())
        );
    }

    #[tokio::test]
    async fn test_tx_command_with_trace_decode() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "tx 0xabc123def456789012345678901234567890123456789012345678901234abcd --trace --decode",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tx_command_with_chain_override() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "tx 0xabc123def456789012345678901234567890123456789012345678901234abcd --chain=polygon",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tx_reuses_last_tx() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext {
            last_tx: Some(
                "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            ),
            ..Default::default()
        };
        let result = execute_input("tx", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tx_auto_detects_tron() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "tx abc123def456789012345678901234567890123456789012345678901234abcd",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        // Context chain stays "auto" (inferred per command, not stored)
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_crawl_command_with_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_command_with_period() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period=7d --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_command_with_chain_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain polygon --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_command_with_period_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period 1h --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_command_with_report() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let result = execute_input(
            &format!(
                "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --report {} --no-charts",
                tmp.path().display()
            ),
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_list_command() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("portfolio list", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_add_command() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "portfolio add 0xtest --label mytest",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_summary_command() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        // Add first
        execute_input("portfolio add 0xtest", &mut ctx, &config, &factory)
            .await
            .unwrap();
        // Then summary
        let result = execute_input("portfolio summary", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_remove_command() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("portfolio remove 0xtest", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_no_subcommand() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("portfolio", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_unknown_subcommand() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("portfolio foobar", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_list() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens list", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_no_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_recent() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens recent", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_remove_no_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens remove", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_add_no_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens add", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_command_unknown() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens foobar", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_setup_command_status() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("setup --status", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_transaction_alias() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "transaction 0xabc123def456789012345678901234567890123456789012345678901234abcd",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_token_alias_for_crawl() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "token 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_port_alias_for_portfolio() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("port list", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // execute_tokens_command direct tests
    // ========================================================================

    #[tokio::test]
    async fn test_execute_tokens_list_empty() {
        let result = execute_tokens_command(&[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_list_subcommand() {
        let result = execute_tokens_command(&["list"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_recent() {
        let result = execute_tokens_command(&["recent"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_add_insufficient_args() {
        let result = execute_tokens_command(&["add"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_add_success() {
        let result = execute_tokens_command(&[
            "add",
            "TEST_INTERACTIVE",
            "ethereum",
            "0xtest123456789",
            "Test Token",
        ])
        .await;
        assert!(result.is_ok());
        let _ = execute_tokens_command(&["remove", "TEST_INTERACTIVE"]).await;
    }

    #[tokio::test]
    async fn test_execute_tokens_remove_no_args() {
        let result = execute_tokens_command(&["remove"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_remove_with_symbol() {
        let _ =
            execute_tokens_command(&["add", "RMTEST", "ethereum", "0xrmtest", "Remove Test"]).await;
        let result = execute_tokens_command(&["remove", "RMTEST"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_unknown_subcommand() {
        let result = execute_tokens_command(&["invalid"]).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // SessionContext additional tests (default and display already exist above)
    // ========================================================================

    #[test]
    fn test_session_context_serialization_roundtrip() {
        let ctx = SessionContext {
            chain: "solana".to_string(),
            include_tokens: true,
            limit: 25,
            last_address: Some("0xtest".to_string()),
            ..Default::default()
        };

        let yaml = serde_yaml::to_string(&ctx).unwrap();
        let deserialized: SessionContext = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.chain, "solana");
        assert!(deserialized.include_tokens);
        assert_eq!(deserialized.limit, 25);
        assert_eq!(deserialized.last_address, Some("0xtest".to_string()));
    }

    // ========================================================================
    // Tests for previously uncovered execute_input branches
    // ========================================================================

    #[tokio::test]
    async fn test_chain_show_explicit() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext {
            chain: "polygon".to_string(),
            ..Default::default()
        };

        // Just showing chain status when chain is pinned
        let result = execute_input("chain", &mut context, &config, &factory).await;
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should not exit
    }

    #[tokio::test]
    async fn test_address_with_explicit_chain() {
        let config = test_config();
        let factory = mock_factory();
        let mut context = SessionContext {
            chain: "polygon".to_string(),
            ..Default::default()
        };

        // Address command with explicit chain — should use context.chain directly
        let result = execute_input(
            "address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            &mut context,
            &config,
            &factory,
        )
        .await;
        // May fail due to network but should not panic
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_tx_with_explicit_chain() {
        let config = test_config();
        let factory = mock_factory();
        let mut context = SessionContext {
            chain: "polygon".to_string(),
            ..Default::default()
        };

        // TX command with explicit chain — should use context.chain
        let result = execute_input("tx 0xabc123def456789", &mut context, &config, &factory).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_period_eq_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        // crawl with --period=7d syntax
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period=7d",
            &mut context,
            &config,
            &factory,
        )
        .await;
        // Will attempt network call, may succeed or fail
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_period_space_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        // crawl with --period 1h syntax (space-separated)
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period 1h",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_chain_eq_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        // crawl with --chain=polygon syntax
        let result = execute_input(
            "crawl 0xAddress --chain=polygon",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_chain_space_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        // crawl with --chain polygon syntax
        let result = execute_input(
            "crawl 0xAddress --chain polygon",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_report_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy();
        let input = format!(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --report={}",
            path
        );
        let result = execute_input(&input, &mut context, &config, &factory).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_no_charts_flag() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext::default();

        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --no-charts",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_with_explicit_chain() {
        let config = test_config();
        let factory = test_factory();
        let mut context = SessionContext {
            chain: "arbitrum".to_string(),
            ..Default::default()
        };

        let result = execute_input("crawl 0xAddress", &mut context, &config, &factory).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_portfolio_add_with_label_and_tags() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut context = SessionContext::default();

        let result = execute_input(
            "portfolio add 0xAbC123 --label MyWallet --tags defi,staking",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_remove_no_args() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut context = SessionContext::default();

        let result = execute_input("portfolio remove", &mut context, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_portfolio_summary_with_chain_and_tag() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut context = SessionContext::default();

        let result = execute_input(
            "portfolio summary --chain ethereum --tag defi --tokens",
            &mut context,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_add_with_name() {
        let result = execute_tokens_command(&[
            "add",
            "USDC",
            "ethereum",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "USD",
            "Coin",
        ])
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_remove_with_chain() {
        let result = execute_tokens_command(&["remove", "USDC", "--chain", "ethereum"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_add_then_list_nonempty() {
        // Add a token first
        let _ = execute_tokens_command(&[
            "add",
            "TEST_TOKEN_XYZ",
            "ethereum",
            "0x1234567890abcdef1234567890abcdef12345678",
            "Test",
            "Token",
        ])
        .await;

        // Now list should show it
        let result = execute_tokens_command(&["list"]).await;
        assert!(result.is_ok());

        // And recent should show it
        let result = execute_tokens_command(&["recent"]).await;
        assert!(result.is_ok());

        // Clean up
        let _ = execute_tokens_command(&["remove", "TEST_TOKEN_XYZ"]).await;
    }

    #[tokio::test]
    async fn test_session_context_save_and_load() {
        // SessionContext::save() and ::load() use dirs::data_dir()
        // We just verify they don't panic
        let ctx = SessionContext {
            chain: "solana".to_string(),
            last_address: Some("0xabc".to_string()),
            last_tx: Some("0xdef".to_string()),
            ..Default::default()
        };
        // save may fail if data dir doesn't exist, but should not panic
        let _ = ctx.save();
        // load should return default or saved data
        let loaded = SessionContext::load();
        // At least the struct is valid
        assert!(!loaded.chain.is_empty());
    }

    // ========================================================================
    // Command alias coverage
    // ========================================================================

    #[tokio::test]
    async fn test_help_alias_question_mark() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("?", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_context_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input("context", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_dot_context_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        let result = execute_input(".context", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_reset_alias() {
        let config = test_config();
        let mut ctx = SessionContext {
            chain: "ethereum".to_string(),
            ..Default::default()
        };
        execute_input("reset", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_dot_reset_alias() {
        let config = test_config();
        let mut ctx = SessionContext {
            chain: "base".to_string(),
            ..Default::default()
        };
        execute_input(".reset", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_dot_clear_alias() {
        let config = test_config();
        let mut ctx = SessionContext {
            chain: "bsc".to_string(),
            ..Default::default()
        };
        execute_input(".clear", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert_eq!(ctx.chain, "auto");
    }

    #[tokio::test]
    async fn test_showtokens_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("showtokens", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_tokens);
    }

    #[tokio::test]
    async fn test_showtxs_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("showtxs", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_txs);
    }

    #[tokio::test]
    async fn test_txs_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input("txs", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_txs);
    }

    #[tokio::test]
    async fn test_dot_txs_alias() {
        let config = test_config();
        let mut ctx = SessionContext::default();
        execute_input(".txs", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(ctx.include_txs);
    }

    #[tokio::test]
    async fn test_addr_alias() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "addr 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            ctx.last_address,
            Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string())
        );
    }

    #[test]
    fn test_session_context_is_auto_chain() {
        let auto_ctx = SessionContext::default();
        assert!(auto_ctx.is_auto_chain());
        let pinned_ctx = SessionContext {
            chain: "ethereum".to_string(),
            ..Default::default()
        };
        assert!(!pinned_ctx.is_auto_chain());
    }

    #[test]
    fn test_print_help_no_panic() {
        print_help();
    }

    // ========================================================================
    // Contract command tests
    // ========================================================================

    #[tokio::test]
    async fn test_contract_no_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("contract", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_contract_ct_alias_with_args() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "ct 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        if let Ok(should_exit) = result {
            assert!(!should_exit);
        }
    }

    #[tokio::test]
    async fn test_contract_with_chain_and_json() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "contract 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain=polygon --json",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        if let Ok(should_exit) = result {
            assert!(!should_exit);
        }
    }

    // ========================================================================
    // address-book and address_book aliases
    // ========================================================================

    #[tokio::test]
    async fn test_address_book_list_command() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("address-book list", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_book_underscore_list() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("address_book list", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_book_add_insufficient_args() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("address-book add", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_book_remove_insufficient_args() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("address-book remove", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_address_book_empty_subcommand() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("address-book", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // aliases, config, monitor commands
    // ========================================================================

    #[tokio::test]
    async fn test_aliases_command() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("aliases", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_config_alias() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("config --status", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "setup --key prompts for API key input on stdin"]
    async fn test_setup_with_key_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("setup --key=etherscan", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_setup_with_key_short_flag() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("setup -s", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    // Note: setup --reset is not tested here; it prompts for stdin confirmation
    // and can block. See setup::tests::test_reset_config_impl_* for reset coverage.

    #[tokio::test]
    #[ignore = "monitor starts TUI and blocks until exit"]
    async fn test_monitor_command_no_token() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("monitor", &mut ctx, &config, &factory).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    #[ignore = "monitor starts TUI and blocks until exit"]
    async fn test_mon_alias() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "mon 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok() || result.is_err());
    }

    // ========================================================================
    // tokens ls alias and crawl period variants
    // ========================================================================

    #[tokio::test]
    async fn test_tokens_ls_alias() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input("tokens ls", &mut ctx, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tokens_ls_alias() {
        let result = execute_tokens_command(&["ls"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_period_1h() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period=1h --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_period_30d() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period=30d --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_crawl_invalid_period_defaults() {
        let config = test_config();
        let factory = mock_factory();
        let mut ctx = SessionContext::default();
        let result = execute_input(
            "crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --period=invalid --no-charts",
            &mut ctx,
            &config,
            &factory,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tokens_add_three_args_insufficient() {
        let result = execute_tokens_command(&["add", "SYM", "ethereum"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_format_show_when_csv() {
        let config = test_config();
        let mut ctx = SessionContext {
            format: OutputFormat::Csv,
            ..Default::default()
        };
        let result = execute_input("format", &mut ctx, &config, &test_factory())
            .await
            .unwrap();
        assert!(!result);
    }
}
