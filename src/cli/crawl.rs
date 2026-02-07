//! # Crawl Command
//!
//! This module implements the `crawl` command for retrieving comprehensive
//! token analytics data including holder information, volume statistics,
//! and price data.
//!
//! ## Usage
//!
//! ```bash
//! # Basic crawl by address
//! bca crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
//!
//! # Search by token name/symbol
//! bca crawl USDC
//! bca crawl "wrapped ether"
//!
//! # Specify chain and period
//! bca crawl USDC --chain ethereum --period 7d
//!
//! # Generate markdown report
//! bca crawl USDC --report report.md
//!
//! # Output as JSON
//! bca crawl USDC --format json
//! ```

use crate::chains::{
    DexClient, DexPair, EthereumClient, Token, TokenAnalytics, TokenHolder, TokenSearchResult,
    infer_chain_from_address,
};
use crate::config::{Config, OutputFormat};
use crate::display::{charts, report};
use crate::error::{BccError, Result};
use crate::tokens::TokenAliases;
use clap::Args;
use std::io::{self, Write};
use std::path::PathBuf;

/// Time period for analytics data.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum Period {
    /// 1 hour
    #[value(name = "1h")]
    Hour1,
    /// 24 hours (default)
    #[default]
    #[value(name = "24h")]
    Hour24,
    /// 7 days
    #[value(name = "7d")]
    Day7,
    /// 30 days
    #[value(name = "30d")]
    Day30,
}

impl Period {
    /// Returns the period duration in seconds.
    pub fn as_seconds(&self) -> i64 {
        match self {
            Period::Hour1 => 3600,
            Period::Hour24 => 86400,
            Period::Day7 => 604800,
            Period::Day30 => 2592000,
        }
    }

    /// Returns a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Period::Hour1 => "1 Hour",
            Period::Hour24 => "24 Hours",
            Period::Day7 => "7 Days",
            Period::Day30 => "30 Days",
        }
    }
}

/// Arguments for the crawl command.
#[derive(Debug, Args)]
pub struct CrawlArgs {
    /// Token address or name/symbol to analyze.
    ///
    /// Can be a contract address (0x...) or a token symbol/name.
    /// If a name is provided, matching tokens will be searched and
    /// you can select from the results.
    pub token: String,

    /// Target blockchain network.
    ///
    /// If not specified, the chain will be inferred from the address format
    /// or all chains will be searched for token names.
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Time period for volume and price data.
    #[arg(short, long, default_value = "24h")]
    pub period: Period,

    /// Maximum number of holders to display.
    #[arg(long, default_value = "10")]
    pub holders_limit: u32,

    /// Output format for the results.
    #[arg(short, long, default_value = "table")]
    pub format: OutputFormat,

    /// Disable ASCII chart output.
    #[arg(long)]
    pub no_charts: bool,

    /// Generate and save a markdown report to the specified path.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,

    /// Skip interactive prompts (use first match for token search).
    #[arg(long)]
    pub yes: bool,

    /// Save the selected token as an alias for future use.
    #[arg(long)]
    pub save: bool,
}

/// Token resolution result with optional alias info.
#[derive(Debug, Clone)]
struct ResolvedToken {
    address: String,
    chain: String,
    /// If resolved from an alias, contains (symbol, name)
    alias_info: Option<(String, String)>,
}

/// Resolves a token input (address, symbol, or name) to a concrete address and chain.
///
/// This function handles:
/// 1. Direct addresses (0x...) - used as-is
/// 2. Saved aliases - looked up from storage
/// 3. Token names/symbols - searched via DEX API with interactive selection
async fn resolve_token_input(
    args: &CrawlArgs,
    aliases: &mut TokenAliases,
) -> Result<ResolvedToken> {
    let input = args.token.trim();

    // Check if it's a direct address
    if TokenAliases::is_address(input) {
        let chain = if args.chain == "ethereum" {
            infer_chain_from_address(input)
                .unwrap_or("ethereum")
                .to_string()
        } else {
            args.chain.clone()
        };
        return Ok(ResolvedToken {
            address: input.to_string(),
            chain,
            alias_info: None,
        });
    }

    // Check if it's a saved alias
    let chain_filter = if args.chain != "ethereum" {
        Some(args.chain.as_str())
    } else {
        None
    };

    if let Some(token_info) = aliases.get(input, chain_filter) {
        println!(
            "Using saved token: {} ({}) on {}",
            token_info.symbol, token_info.name, token_info.chain
        );
        return Ok(ResolvedToken {
            address: token_info.address.clone(),
            chain: token_info.chain.clone(),
            alias_info: Some((token_info.symbol.clone(), token_info.name.clone())),
        });
    }

    // Search for tokens by name/symbol
    println!("Searching for '{}'...", input);

    let dex_client = DexClient::new();
    let search_results = dex_client.search_tokens(input, chain_filter).await?;

    if search_results.is_empty() {
        return Err(BccError::NotFound(format!(
            "No tokens found matching '{}'. Try a different name or use the contract address.",
            input
        )));
    }

    // Display results and let user select
    let selected = select_token(&search_results, args.yes)?;

    // Offer to save the alias
    if args.save || (!args.yes && prompt_save_alias()) {
        aliases.add(
            &selected.symbol,
            &selected.chain,
            &selected.address,
            &selected.name,
        );
        if let Err(e) = aliases.save() {
            tracing::warn!("Failed to save token alias: {}", e);
        } else {
            println!("Saved {} as alias for future use.", selected.symbol);
        }
    }

    Ok(ResolvedToken {
        address: selected.address.clone(),
        chain: selected.chain.clone(),
        alias_info: Some((selected.symbol.clone(), selected.name.clone())),
    })
}

/// Displays token search results and prompts user to select one.
fn select_token(results: &[TokenSearchResult], auto_select: bool) -> Result<&TokenSearchResult> {
    if results.len() == 1 || auto_select {
        let selected = &results[0];
        println!(
            "Selected: {} ({}) on {} - ${:.6}",
            selected.symbol,
            selected.name,
            selected.chain,
            selected.price_usd.unwrap_or(0.0)
        );
        return Ok(selected);
    }

    println!("\nFound {} matching tokens:\n", results.len());
    println!(
        "{:>3}  {:>8}  {:<20}  {:<12}  {:>12}  {:>12}",
        "#", "Symbol", "Name", "Chain", "Price", "Liquidity"
    );
    println!("{}", "-".repeat(80));

    for (i, token) in results.iter().enumerate() {
        let price = token
            .price_usd
            .map(|p| format!("${:.6}", p))
            .unwrap_or_else(|| "N/A".to_string());

        let liquidity = format_large_number(token.liquidity_usd);

        // Truncate name if too long
        let name = if token.name.len() > 18 {
            format!("{}...", &token.name[..15])
        } else {
            token.name.clone()
        };

        println!(
            "{:>3}  {:>8}  {:<20}  {:<12}  {:>12}  {:>12}",
            i + 1,
            token.symbol,
            name,
            token.chain,
            price,
            liquidity
        );
    }

    println!();
    print!("Select token (1-{}): ", results.len());
    io::stdout()
        .flush()
        .map_err(|e| BccError::Io(e.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BccError::Io(e.to_string()))?;

    let selection: usize = input
        .trim()
        .parse()
        .map_err(|_| BccError::Api("Invalid selection".to_string()))?;

    if selection < 1 || selection > results.len() {
        return Err(BccError::Api(format!(
            "Selection must be between 1 and {}",
            results.len()
        )));
    }

    Ok(&results[selection - 1])
}

/// Prompts the user to save the token alias.
fn prompt_save_alias() -> bool {
    print!("Save this token for future use? [y/N]: ");
    if io::stdout().flush().is_err() {
        return false;
    }

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Runs the crawl command.
///
/// Fetches comprehensive token analytics and displays them with ASCII charts
/// or generates a markdown report.
pub async fn run(args: CrawlArgs, config: &Config) -> Result<()> {
    // Load token aliases
    let mut aliases = TokenAliases::load();

    // Resolve the token input to an address
    let resolved = resolve_token_input(&args, &mut aliases).await?;

    tracing::info!(
        token = %resolved.address,
        chain = %resolved.chain,
        period = ?args.period,
        "Starting token crawl"
    );

    println!(
        "Crawling token {} on {}...",
        resolved.address, resolved.chain
    );

    // Fetch token analytics from multiple sources
    let mut analytics =
        fetch_token_analytics(&resolved.address, &resolved.chain, &args, config).await?;

    // If we have alias info and the fetched token info is unknown, use alias info
    if (analytics.token.symbol == "UNKNOWN" || analytics.token.name == "Unknown Token")
        && let Some((symbol, name)) = &resolved.alias_info
    {
        analytics.token.symbol = symbol.clone();
        analytics.token.name = name.clone();
    }

    // Output based on format
    match args.format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&analytics)?;
            println!("{}", json);
        }
        OutputFormat::Csv => {
            output_csv(&analytics)?;
        }
        OutputFormat::Table => {
            output_table(&analytics, &args)?;
        }
    }

    // Generate report if requested
    if let Some(ref report_path) = args.report {
        let markdown_report = report::generate_report(&analytics);
        report::save_report(&markdown_report, report_path)?;
        println!("\nReport saved to: {}", report_path.display());
    }

    Ok(())
}

/// Fetches comprehensive token analytics from multiple data sources.
async fn fetch_token_analytics(
    token_address: &str,
    chain: &str,
    args: &CrawlArgs,
    config: &Config,
) -> Result<TokenAnalytics> {
    // Initialize clients
    let dex_client = DexClient::new();

    // Try to fetch DEX data (price, volume, liquidity)
    println!("  Fetching DEX data...");
    let dex_result = dex_client.get_token_data(chain, token_address).await;

    // Handle DEX data - either use it or fall back to block explorer only
    match dex_result {
        Ok(dex_data) => {
            // We have DEX data - proceed with full analytics
            fetch_analytics_with_dex(token_address, chain, args, config, dex_data).await
        }
        Err(BccError::NotFound(_)) => {
            // No DEX data - fall back to block explorer only
            println!("  No DEX data found, fetching from block explorer...");
            fetch_analytics_from_explorer(token_address, chain, args, config).await
        }
        Err(e) => Err(e),
    }
}

/// Fetches analytics when DEX data is available.
async fn fetch_analytics_with_dex(
    token_address: &str,
    chain: &str,
    args: &CrawlArgs,
    config: &Config,
    dex_data: crate::chains::dex::DexTokenData,
) -> Result<TokenAnalytics> {
    // Fetch holder data from block explorer (if available)
    println!("  Fetching holder data...");
    let holders = fetch_holders(token_address, chain, args.holders_limit, config).await?;

    // Get token info
    let token = Token {
        contract_address: token_address.to_string(),
        symbol: dex_data.symbol.clone(),
        name: dex_data.name.clone(),
        decimals: 18, // Default, could be fetched from contract
    };

    // Calculate concentration metrics
    let top_10_pct: f64 = holders.iter().take(10).map(|h| h.percentage).sum();
    let top_50_pct: f64 = holders.iter().take(50).map(|h| h.percentage).sum();
    let top_100_pct: f64 = holders.iter().take(100).map(|h| h.percentage).sum();

    // Convert DEX pairs
    let dex_pairs: Vec<DexPair> = dex_data.pairs;

    // Calculate 7d volume estimate
    let volume_7d = DexClient::estimate_7d_volume(dex_data.volume_24h);

    // Get current timestamp
    let fetched_at = chrono::Utc::now().timestamp();

    // Calculate token age in hours from earliest pair creation
    // DexScreener returns pairCreatedAt in milliseconds, so convert to seconds
    let token_age_hours = dex_data.earliest_pair_created_at.map(|created_at| {
        let now = chrono::Utc::now().timestamp();
        // If timestamp is in milliseconds (> year 3000 in seconds), convert to seconds
        let created_at_secs = if created_at > 32503680000 {
            created_at / 1000
        } else {
            created_at
        };
        let age_secs = now - created_at_secs;
        if age_secs > 0 {
            (age_secs as f64) / 3600.0
        } else {
            0.0 // Fallback for invalid timestamps
        }
    });

    // Map social links to the TokenSocial type used in TokenAnalytics
    let socials: Vec<crate::chains::TokenSocial> = dex_data
        .socials
        .iter()
        .map(|s| crate::chains::TokenSocial {
            platform: s.platform.clone(),
            url: s.url.clone(),
        })
        .collect();

    Ok(TokenAnalytics {
        token,
        chain: chain.to_string(),
        holders,
        total_holders: 0, // Would need a separate API call
        volume_24h: dex_data.volume_24h,
        volume_7d,
        price_usd: dex_data.price_usd,
        price_change_24h: dex_data.price_change_24h,
        price_change_7d: 0.0, // Not available from DexScreener directly
        liquidity_usd: dex_data.liquidity_usd,
        market_cap: dex_data.market_cap,
        fdv: dex_data.fdv,
        total_supply: None,
        circulating_supply: None,
        price_history: dex_data.price_history,
        volume_history: dex_data.volume_history,
        holder_history: Vec::new(), // Would need historical data
        dex_pairs,
        fetched_at,
        top_10_concentration: if top_10_pct > 0.0 {
            Some(top_10_pct)
        } else {
            None
        },
        top_50_concentration: if top_50_pct > 0.0 {
            Some(top_50_pct)
        } else {
            None
        },
        top_100_concentration: if top_100_pct > 0.0 {
            Some(top_100_pct)
        } else {
            None
        },
        price_change_6h: dex_data.price_change_6h,
        price_change_1h: dex_data.price_change_1h,
        total_buys_24h: dex_data.total_buys_24h,
        total_sells_24h: dex_data.total_sells_24h,
        total_buys_6h: dex_data.total_buys_6h,
        total_sells_6h: dex_data.total_sells_6h,
        total_buys_1h: dex_data.total_buys_1h,
        total_sells_1h: dex_data.total_sells_1h,
        token_age_hours,
        image_url: dex_data.image_url.clone(),
        websites: dex_data.websites.clone(),
        socials,
        dexscreener_url: dex_data.dexscreener_url.clone(),
    })
}

/// Fetches basic token analytics from block explorer when DEX data is unavailable.
async fn fetch_analytics_from_explorer(
    token_address: &str,
    chain: &str,
    args: &CrawlArgs,
    config: &Config,
) -> Result<TokenAnalytics> {
    // Only EVM chains support block explorer data
    let is_evm = matches!(
        chain,
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc"
    );

    if !is_evm {
        return Err(BccError::NotFound(format!(
            "No DEX data found for token {} on {} and block explorer fallback not supported for this chain",
            token_address, chain
        )));
    }

    // Create block explorer client
    let client = EthereumClient::for_chain(chain, &config.chains)?;

    // Fetch token info
    println!("  Fetching token info...");
    let token = match client.get_token_info(token_address).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to fetch token info: {}", e);
            // Use placeholder token info
            Token {
                contract_address: token_address.to_string(),
                symbol: "UNKNOWN".to_string(),
                name: "Unknown Token".to_string(),
                decimals: 18,
            }
        }
    };

    // Fetch holder data
    println!("  Fetching holder data...");
    let holders = fetch_holders(token_address, chain, args.holders_limit, config).await?;

    // Fetch holder count
    println!("  Fetching holder count...");
    let total_holders = match client.get_token_holder_count(token_address).await {
        Ok(count) => count,
        Err(e) => {
            tracing::warn!("Failed to fetch holder count: {}", e);
            0
        }
    };

    // Calculate concentration metrics
    let top_10_pct: f64 = holders.iter().take(10).map(|h| h.percentage).sum();
    let top_50_pct: f64 = holders.iter().take(50).map(|h| h.percentage).sum();
    let top_100_pct: f64 = holders.iter().take(100).map(|h| h.percentage).sum();

    // Get current timestamp
    let fetched_at = chrono::Utc::now().timestamp();

    Ok(TokenAnalytics {
        token,
        chain: chain.to_string(),
        holders,
        total_holders,
        volume_24h: 0.0,
        volume_7d: 0.0,
        price_usd: 0.0,
        price_change_24h: 0.0,
        price_change_7d: 0.0,
        liquidity_usd: 0.0,
        market_cap: None,
        fdv: None,
        total_supply: None,
        circulating_supply: None,
        price_history: Vec::new(),
        volume_history: Vec::new(),
        holder_history: Vec::new(),
        dex_pairs: Vec::new(),
        fetched_at,
        top_10_concentration: if top_10_pct > 0.0 {
            Some(top_10_pct)
        } else {
            None
        },
        top_50_concentration: if top_50_pct > 0.0 {
            Some(top_50_pct)
        } else {
            None
        },
        top_100_concentration: if top_100_pct > 0.0 {
            Some(top_100_pct)
        } else {
            None
        },
        price_change_6h: 0.0,
        price_change_1h: 0.0,
        total_buys_24h: 0,
        total_sells_24h: 0,
        total_buys_6h: 0,
        total_sells_6h: 0,
        total_buys_1h: 0,
        total_sells_1h: 0,
        token_age_hours: None,
        image_url: None,
        websites: Vec::new(),
        socials: Vec::new(),
        dexscreener_url: None,
    })
}

/// Fetches token holder data from block explorer APIs.
async fn fetch_holders(
    token_address: &str,
    chain: &str,
    limit: u32,
    config: &Config,
) -> Result<Vec<TokenHolder>> {
    // Only EVM chains support holder data via block explorers
    match chain {
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" => {
            let client = EthereumClient::for_chain(chain, &config.chains)?;
            match client.get_token_holders(token_address, limit).await {
                Ok(holders) => Ok(holders),
                Err(e) => {
                    tracing::warn!("Failed to fetch holders: {}", e);
                    Ok(Vec::new())
                }
            }
        }
        _ => {
            tracing::info!("Holder data not available for chain: {}", chain);
            Ok(Vec::new())
        }
    }
}

/// Outputs analytics in table format with optional charts.
fn output_table(analytics: &TokenAnalytics, args: &CrawlArgs) -> Result<()> {
    println!();

    // Check if we have DEX data (price > 0 indicates DEX data)
    let has_dex_data = analytics.price_usd > 0.0;

    if has_dex_data {
        // Full output with DEX data
        output_table_with_dex(analytics, args)
    } else {
        // Explorer-only output
        output_table_explorer_only(analytics)
    }
}

/// Outputs full analytics table with DEX data.
fn output_table_with_dex(analytics: &TokenAnalytics, args: &CrawlArgs) -> Result<()> {
    // Display charts if not disabled
    if !args.no_charts {
        let dashboard = charts::render_analytics_dashboard(
            &analytics.price_history,
            &analytics.volume_history,
            &analytics.holders,
            &analytics.token.symbol,
            &analytics.chain,
        );
        println!("{}", dashboard);
    } else {
        // Display text-only summary
        println!(
            "Token: {} ({})",
            analytics.token.name, analytics.token.symbol
        );
        println!("Chain: {}", analytics.chain);
        println!("Contract: {}", analytics.token.contract_address);
        println!();
    }

    // Key metrics
    println!("Key Metrics");
    println!("{}", "=".repeat(50));
    println!("Price:           ${:.6}", analytics.price_usd);
    println!("24h Change:      {:+.2}%", analytics.price_change_24h);
    println!(
        "24h Volume:      ${}",
        format_large_number(analytics.volume_24h)
    );
    println!(
        "Liquidity:       ${}",
        format_large_number(analytics.liquidity_usd)
    );

    if let Some(mc) = analytics.market_cap {
        println!("Market Cap:      ${}", format_large_number(mc));
    }

    if let Some(fdv) = analytics.fdv {
        println!("FDV:             ${}", format_large_number(fdv));
    }

    // Trading pairs
    if !analytics.dex_pairs.is_empty() {
        println!();
        println!("Top Trading Pairs");
        println!("{}", "=".repeat(50));

        for (i, pair) in analytics.dex_pairs.iter().take(5).enumerate() {
            println!(
                "{}. {} {}/{} - ${} (${} liq)",
                i + 1,
                pair.dex_name,
                pair.base_token,
                pair.quote_token,
                format_large_number(pair.volume_24h),
                format_large_number(pair.liquidity_usd)
            );
        }
    }

    // Concentration summary
    if let Some(top_10) = analytics.top_10_concentration {
        println!();
        println!("Holder Concentration");
        println!("{}", "=".repeat(50));
        println!("Top 10 holders:  {:.1}% of supply", top_10);

        if let Some(top_50) = analytics.top_50_concentration {
            println!("Top 50 holders:  {:.1}% of supply", top_50);
        }
    }

    Ok(())
}

/// Outputs basic token info from block explorer (no DEX data).
fn output_table_explorer_only(analytics: &TokenAnalytics) -> Result<()> {
    println!("Token Info (Block Explorer Data)");
    println!("{}", "=".repeat(60));
    println!();

    // Basic token info
    println!("Name:            {}", analytics.token.name);
    println!("Symbol:          {}", analytics.token.symbol);
    println!("Contract:        {}", analytics.token.contract_address);
    println!("Chain:           {}", analytics.chain);
    println!("Decimals:        {}", analytics.token.decimals);

    if analytics.total_holders > 0 {
        println!("Total Holders:   {}", analytics.total_holders);
    }

    if let Some(supply) = &analytics.total_supply {
        println!("Total Supply:    {}", supply);
    }

    // Note about missing DEX data
    println!();
    println!("Note: No DEX trading data available for this token.");
    println!("      Price, volume, and liquidity data require active DEX pairs.");

    // Top holders if available
    if !analytics.holders.is_empty() {
        println!();
        println!("Top Holders");
        println!("{}", "=".repeat(60));
        println!(
            "{:>4}  {:>10}  {:>20}  Address",
            "Rank", "Percent", "Balance"
        );
        println!("{}", "-".repeat(80));

        for holder in analytics.holders.iter().take(10) {
            // Truncate address for display
            let addr_display = if holder.address.len() > 20 {
                format!(
                    "{}...{}",
                    &holder.address[..10],
                    &holder.address[holder.address.len() - 8..]
                )
            } else {
                holder.address.clone()
            };

            println!(
                "{:>4}  {:>9.2}%  {:>20}  {}",
                holder.rank, holder.percentage, holder.formatted_balance, addr_display
            );
        }
    }

    // Concentration summary
    if let Some(top_10) = analytics.top_10_concentration {
        println!();
        println!("Holder Concentration");
        println!("{}", "=".repeat(60));
        println!("Top 10 holders:  {:.1}% of supply", top_10);

        if let Some(top_50) = analytics.top_50_concentration {
            println!("Top 50 holders:  {:.1}% of supply", top_50);
        }
    }

    Ok(())
}

/// Outputs analytics in CSV format.
fn output_csv(analytics: &TokenAnalytics) -> Result<()> {
    // Header
    println!("metric,value");

    // Basic info
    println!("symbol,{}", analytics.token.symbol);
    println!("name,{}", analytics.token.name);
    println!("chain,{}", analytics.chain);
    println!("contract,{}", analytics.token.contract_address);

    // Metrics
    println!("price_usd,{}", analytics.price_usd);
    println!("price_change_24h,{}", analytics.price_change_24h);
    println!("volume_24h,{}", analytics.volume_24h);
    println!("volume_7d,{}", analytics.volume_7d);
    println!("liquidity_usd,{}", analytics.liquidity_usd);

    if let Some(mc) = analytics.market_cap {
        println!("market_cap,{}", mc);
    }

    if let Some(fdv) = analytics.fdv {
        println!("fdv,{}", fdv);
    }

    println!("total_holders,{}", analytics.total_holders);

    if let Some(top_10) = analytics.top_10_concentration {
        println!("top_10_concentration,{}", top_10);
    }

    // Holders section
    if !analytics.holders.is_empty() {
        println!();
        println!("rank,address,balance,percentage");
        for holder in &analytics.holders {
            println!(
                "{},{},{},{}",
                holder.rank, holder.address, holder.balance, holder.percentage
            );
        }
    }

    Ok(())
}

/// Formats a large number with K, M, B suffixes.
fn format_large_number(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{:.2}", value)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_as_seconds() {
        assert_eq!(Period::Hour1.as_seconds(), 3600);
        assert_eq!(Period::Hour24.as_seconds(), 86400);
        assert_eq!(Period::Day7.as_seconds(), 604800);
        assert_eq!(Period::Day30.as_seconds(), 2592000);
    }

    #[test]
    fn test_period_label() {
        assert_eq!(Period::Hour1.label(), "1 Hour");
        assert_eq!(Period::Hour24.label(), "24 Hours");
        assert_eq!(Period::Day7.label(), "7 Days");
        assert_eq!(Period::Day30.label(), "30 Days");
    }

    #[test]
    fn test_format_large_number() {
        assert_eq!(format_large_number(500.0), "500.00");
        assert_eq!(format_large_number(1500.0), "1.50K");
        assert_eq!(format_large_number(1500000.0), "1.50M");
        assert_eq!(format_large_number(1500000000.0), "1.50B");
    }

    #[test]
    fn test_period_default() {
        let period = Period::default();
        assert!(matches!(period, Period::Hour24));
    }

    #[test]
    fn test_crawl_args_defaults() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            crawl: CrawlArgs,
        }

        let cli = TestCli::try_parse_from(["test", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"])
            .unwrap();

        assert_eq!(cli.crawl.chain, "ethereum");
        assert!(matches!(cli.crawl.period, Period::Hour24));
        assert_eq!(cli.crawl.holders_limit, 10);
        assert!(!cli.crawl.no_charts);
        assert!(cli.crawl.report.is_none());
    }
}
