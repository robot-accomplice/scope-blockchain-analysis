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
//! scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
//!
//! # Search by token name/symbol
//! scope crawl USDC
//! scope crawl "wrapped ether"
//!
//! # Specify chain and period
//! scope crawl USDC --chain ethereum --period 7d
//!
//! # Generate markdown report
//! scope crawl USDC --report report.md
//!
//! # Output as JSON
//! scope crawl USDC --format json
//! ```

use crate::chains::{
    ChainClientFactory, DexClient, DexDataSource, DexPair, Token, TokenAnalytics, TokenHolder,
    TokenSearchResult, infer_chain_from_address,
};
use crate::config::{Config, OutputFormat};
use crate::display::{charts, report};
use crate::error::{Result, ScopeError};
use crate::market::{ExchangeClient, VenueRegistry};
use crate::tokens::TokenAliases;
use clap::Args;
use std::io::{self, BufRead, Write};
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
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope crawl USDC
  scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum
  scope crawl USDC --period 7d --report usdc_report.md
  scope crawl PEPE --format json --no-charts",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope crawl USDC\x1b[0m

  Key Metrics
  ==================================================
  Price:           $0.999900
  24h Change:      -0.01%
  24h Volume:      $5.00M
  Liquidity:       $100.00M
  Market Cap:      $30.00B
  FDV:             $30.00B

  Top Trading Pairs
  ==================================================
  1. Uniswap V3 USDC/WETH - $5.00M ($50.00M liq)
  2. Uniswap V2 USDC/USDT - $2.50M ($25.00M liq)
  ...

  \x1b[1m$ scope crawl PEPE --period 7d --no-charts\x1b[0m

  Key Metrics
  ==================================================
  Price:           $0.000012
  24h Change:      +8.50%
  24h Volume:      $120.00M
  Liquidity:       $45.00M
  Market Cap:      $5.00B
  ...

  \x1b[1m$ scope crawl USDC --report usdc.md\x1b[0m

  Key Metrics
  ==================================================
  Price:           $0.999900
  ...
  Report saved to usdc.md"
)]
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
///
/// Attempts to find a token on a CEX venue when DexScreener has no results.
///
/// Tries the default venue (binance) first, checking `{SYMBOL}USDT`.
/// Returns a synthetic [`TokenSearchResult`] with price/volume from the ticker.
async fn try_cex_fallback(symbol: &str, chain: &str) -> Option<TokenSearchResult> {
    let registry = VenueRegistry::load().ok()?;
    let venue_id = "binance";
    let descriptor = registry.get(venue_id)?;
    let client = ExchangeClient::from_descriptor(&descriptor.clone());
    let pair = client.format_pair(&format!("{}USDT", symbol.to_uppercase()));
    let ticker = client.fetch_ticker(&pair).await.ok()?;
    let price = ticker.last_price.unwrap_or(0.0);
    Some(TokenSearchResult {
        address: String::new(), // no on-chain address from CEX
        symbol: symbol.to_uppercase(),
        name: symbol.to_uppercase(),
        chain: chain.to_string(),
        price_usd: Some(price),
        volume_24h: ticker.volume_24h.unwrap_or(0.0),
        liquidity_usd: 0.0,
        market_cap: None,
    })
}

/// Uses `dex_client` for search to enable dependency injection and testing.
///
/// When an optional `spinner` is provided, progress messages are routed through
/// it instead of printing directly to stderr/stdout, keeping output on a single
/// updating line.
async fn resolve_token_input(
    args: &CrawlArgs,
    aliases: &mut TokenAliases,
    dex_client: &dyn DexDataSource,
    spinner: Option<&crate::cli::progress::Spinner>,
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
        let msg = format!(
            "Using saved token: {} ({}) on {}",
            token_info.symbol, token_info.name, token_info.chain
        );
        if let Some(sp) = spinner {
            sp.set_message(msg);
        } else {
            eprintln!("  {}", msg);
        }
        return Ok(ResolvedToken {
            address: token_info.address.clone(),
            chain: token_info.chain.clone(),
            alias_info: Some((token_info.symbol.clone(), token_info.name.clone())),
        });
    }

    // Search for tokens by name/symbol
    let search_msg = format!("Searching for '{}'...", input);
    if let Some(sp) = spinner {
        sp.set_message(search_msg);
    } else {
        eprintln!("  {}", search_msg);
    }

    let mut search_results = dex_client.search_tokens(input, chain_filter).await?;

    // Fallback: if DexScreener has no results, try CEX venue ticker
    if search_results.is_empty()
        && let Some(fallback) = try_cex_fallback(input, &args.chain).await
    {
        let msg = format!(
            "Not found on DexScreener; found {} on {} (CEX)",
            fallback.symbol, fallback.chain
        );
        if let Some(sp) = spinner {
            sp.println(&msg);
        } else {
            eprintln!("  {}", msg);
        }
        search_results.push(fallback);
    }

    if search_results.is_empty() {
        return Err(ScopeError::NotFound(format!(
            "No token found matching '{}' on {} (checked DexScreener and CEX venues)",
            input, args.chain
        )));
    }

    // Display results and let user select
    // When a spinner is active, suspend it for interactive selection (multi-result)
    // or route auto-select messages through it (single-result / --yes).
    let selected = if let Some(sp) = spinner {
        if search_results.len() == 1 || args.yes {
            // Auto-select: route "Selected:" through the spinner
            let sel = &search_results[0];
            sp.set_message(format!(
                "Selected: {} ({}) on {} - ${:.6}",
                sel.symbol,
                sel.name,
                sel.chain,
                sel.price_usd.unwrap_or(0.0)
            ));
            sel
        } else {
            // Interactive: suspend spinner for the prompt
            let result = sp.suspend(|| select_token(&search_results, args.yes));
            result?
        }
    } else {
        select_token(&search_results, args.yes)?
    };

    // Offer to save the alias
    if args.save || (!args.yes && prompt_save_alias()) {
        aliases.add(
            &selected.symbol,
            &selected.chain,
            &selected.address,
            &selected.name,
        );
        if let Err(e) = aliases.save() {
            tracing::debug!("Failed to save token alias: {}", e);
        } else if let Some(sp) = spinner {
            sp.println(&format!(
                "Saved {} as alias for future use.",
                selected.symbol
            ));
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
    let stdin = io::stdin();
    let stdout = io::stdout();
    select_token_impl(results, auto_select, &mut stdin.lock(), &mut stdout.lock())
}

/// Testable implementation of select_token with injected I/O.
fn select_token_impl<'a>(
    results: &'a [TokenSearchResult],
    auto_select: bool,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<&'a TokenSearchResult> {
    if results.len() == 1 || auto_select {
        let selected = &results[0];
        writeln!(
            writer,
            "Selected: {} ({}) on {} - ${:.6}",
            selected.symbol,
            selected.name,
            selected.chain,
            selected.price_usd.unwrap_or(0.0)
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
        return Ok(selected);
    }

    writeln!(writer, "\nFound {} matching tokens:\n", results.len())
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(
        writer,
        "{:>3}  {:>8}  {:<22}  {:<16}  {:<12}  {:>12}  {:>12}",
        "#", "Symbol", "Name", "Address", "Chain", "Price", "Liquidity"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;
    writeln!(writer, "{}", "─".repeat(98)).map_err(|e| ScopeError::Io(e.to_string()))?;

    for (i, token) in results.iter().enumerate() {
        let price = token
            .price_usd
            .map(|p| format!("${:.6}", p))
            .unwrap_or_else(|| "N/A".to_string());

        let liquidity = crate::display::format_large_number(token.liquidity_usd);
        let addr = abbreviate_address(&token.address);

        // Truncate name if too long
        let name = if token.name.len() > 20 {
            format!("{}...", &token.name[..17])
        } else {
            token.name.clone()
        };

        writeln!(
            writer,
            "{:>3}  {:>8}  {:<22}  {:<16}  {:<12}  {:>12}  {:>12}",
            i + 1,
            token.symbol,
            name,
            addr,
            token.chain,
            price,
            liquidity
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    write!(writer, "Select token (1-{}): ", results.len())
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

    let selection: usize = input
        .trim()
        .parse()
        .map_err(|_| ScopeError::Api("Invalid selection".to_string()))?;

    if selection < 1 || selection > results.len() {
        return Err(ScopeError::Api(format!(
            "Selection must be between 1 and {}",
            results.len()
        )));
    }

    Ok(&results[selection - 1])
}

/// Prompts the user to save the token alias.
fn prompt_save_alias() -> bool {
    let stdin = io::stdin();
    let stdout = io::stdout();
    prompt_save_alias_impl(&mut stdin.lock(), &mut stdout.lock())
}

/// Testable implementation of prompt_save_alias with injected I/O.
fn prompt_save_alias_impl(reader: &mut impl BufRead, writer: &mut impl Write) -> bool {
    if write!(writer, "Save this token for future use? [y/N]: ").is_err() {
        return false;
    }
    if writer.flush().is_err() {
        return false;
    }

    let mut input = String::new();
    if reader.read_line(&mut input).is_err() {
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Fetches token analytics for composite commands (e.g., token-health).
/// Resolves token input (address or symbol) and returns full analytics.
///
/// When an optional `spinner` is provided, progress messages (searching,
/// selecting) are routed through it for single-line in-place updates.
pub async fn fetch_analytics_for_input(
    token_input: &str,
    chain: &str,
    period: Period,
    holders_limit: u32,
    clients: &dyn ChainClientFactory,
    spinner: Option<&crate::cli::progress::Spinner>,
) -> Result<TokenAnalytics> {
    let args = CrawlArgs {
        token: token_input.to_string(),
        chain: chain.to_string(),
        period,
        holders_limit,
        format: OutputFormat::Table,
        no_charts: true,
        report: None,
        yes: true,
        save: false,
    };
    let mut aliases = TokenAliases::load();
    let dex_client = clients.create_dex_client();
    let resolved = resolve_token_input(&args, &mut aliases, dex_client.as_ref(), spinner).await?;
    if let Some(sp) = spinner {
        sp.set_message(format!(
            "Fetching analytics for {} on {}...",
            resolved.address, resolved.chain
        ));
    }
    let mut analytics =
        fetch_token_analytics(&resolved.address, &resolved.chain, &args, clients).await?;
    if let Some((symbol, name)) = &resolved.alias_info
        && (analytics.token.symbol == "UNKNOWN" || analytics.token.name == "Unknown Token")
    {
        analytics.token.symbol = symbol.clone();
        analytics.token.name = name.clone();
    }
    Ok(analytics)
}

/// Runs the crawl command.
///
/// Fetches comprehensive token analytics and displays them with ASCII charts
/// or generates a markdown report.
pub async fn run(
    mut args: CrawlArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Resolve address book label → address + chain before token resolution
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.token, config)?
    {
        args.token = address;
        if args.chain == "ethereum" {
            args.chain = chain;
        }
    }

    // Load token aliases
    let mut aliases = TokenAliases::load();

    // Start spinner early so resolution messages route through it
    let sp = crate::cli::progress::Spinner::new(&format!(
        "Crawling token {} on {}...",
        args.token, args.chain
    ));

    // Resolve the token input to an address (uses factory's dex client for search)
    let dex_client = clients.create_dex_client();
    let resolved = resolve_token_input(&args, &mut aliases, dex_client.as_ref(), Some(&sp)).await?;

    tracing::info!(
        token = %resolved.address,
        chain = %resolved.chain,
        period = ?args.period,
        "Starting token crawl"
    );

    sp.set_message(format!(
        "Fetching analytics for {} on {}...",
        resolved.address, resolved.chain
    ));

    // Fetch token analytics from multiple sources
    let mut analytics =
        fetch_token_analytics(&resolved.address, &resolved.chain, &args, clients).await?;

    sp.finish("Token data loaded.");

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
        OutputFormat::Markdown => {
            let md = report::generate_report(&analytics);
            println!("{}", md);
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
    clients: &dyn ChainClientFactory,
) -> Result<TokenAnalytics> {
    // Initialize DEX client via factory
    let dex_client = clients.create_dex_client();

    // Try to fetch DEX data (price, volume, liquidity)
    let dex_result = dex_client.get_token_data(chain, token_address).await;

    // Handle DEX data - either use it or fall back to block explorer only
    match dex_result {
        Ok(dex_data) => {
            // We have DEX data - proceed with full analytics
            fetch_analytics_with_dex(token_address, chain, args, clients, dex_data).await
        }
        Err(ScopeError::NotFound(_)) => {
            // No DEX data - fall back to block explorer only
            tracing::debug!("No DEX data, falling back to block explorer");
            fetch_analytics_from_explorer(token_address, chain, args, clients).await
        }
        Err(e) => Err(e),
    }
}

/// Fetches analytics when DEX data is available.
async fn fetch_analytics_with_dex(
    token_address: &str,
    chain: &str,
    args: &CrawlArgs,
    clients: &dyn ChainClientFactory,
    dex_data: crate::chains::dex::DexTokenData,
) -> Result<TokenAnalytics> {
    // Fetch holder data from block explorer (if available)
    let holders = fetch_holders(token_address, chain, args.holders_limit, clients).await?;

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
    clients: &dyn ChainClientFactory,
) -> Result<TokenAnalytics> {
    // EVM chains, Solana (token info via RPC), and Tron support block explorer data
    let has_explorer = matches!(
        chain,
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "solana" | "tron"
    );

    if !has_explorer {
        return Err(ScopeError::NotFound(format!(
            "No DEX data found for token {} on {} and block explorer fallback not supported for this chain",
            token_address, chain
        )));
    }

    // Create chain client via factory
    let client = clients.create_chain_client(chain)?;

    // Fetch token info
    let token = match client.get_token_info(token_address).await {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!("Failed to fetch token info: {}", e);
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
    let holders = fetch_holders(token_address, chain, args.holders_limit, clients).await?;

    // Fetch holder count
    let total_holders = match client.get_token_holder_count(token_address).await {
        Ok(count) => count,
        Err(e) => {
            tracing::debug!("Failed to fetch holder count: {}", e);
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
    clients: &dyn ChainClientFactory,
) -> Result<Vec<TokenHolder>> {
    // EVM chains and Tron support holder data; Solana uses default (empty) until Solscan Pro
    match chain {
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "solana" | "tron" => {
            let client = clients.create_chain_client(chain)?;
            match client.get_token_holders(token_address, limit).await {
                Ok(holders) => Ok(holders),
                Err(e) => {
                    tracing::debug!("Failed to fetch holders: {}", e);
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
        crate::display::format_large_number(analytics.volume_24h)
    );
    println!(
        "Liquidity:       ${}",
        crate::display::format_large_number(analytics.liquidity_usd)
    );

    if let Some(mc) = analytics.market_cap {
        println!(
            "Market Cap:      ${}",
            crate::display::format_large_number(mc)
        );
    }

    if let Some(fdv) = analytics.fdv {
        println!(
            "FDV:             ${}",
            crate::display::format_large_number(fdv)
        );
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
                crate::display::format_large_number(pair.volume_24h),
                crate::display::format_large_number(pair.liquidity_usd)
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

/// Abbreviates a blockchain address for display (e.g. "0x1234...abcd").
fn abbreviate_address(addr: &str) -> String {
    if addr.len() > 16 {
        format!("{}...{}", &addr[..8], &addr[addr.len() - 6..])
    } else {
        addr.to_string()
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
        assert_eq!(crate::display::format_large_number(500.0), "500.00");
        assert_eq!(crate::display::format_large_number(1500.0), "1.50K");
        assert_eq!(crate::display::format_large_number(1500000.0), "1.50M");
        assert_eq!(crate::display::format_large_number(1500000000.0), "1.50B");
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

    // ========================================================================
    // format_large_number edge cases
    // ========================================================================

    #[test]
    fn test_format_large_number_zero() {
        assert_eq!(crate::display::format_large_number(0.0), "0.00");
    }

    #[test]
    fn test_format_large_number_small() {
        assert_eq!(crate::display::format_large_number(0.12), "0.12");
    }

    #[test]
    fn test_format_large_number_boundary_k() {
        assert_eq!(crate::display::format_large_number(999.99), "999.99");
        assert_eq!(crate::display::format_large_number(1000.0), "1.00K");
    }

    #[test]
    fn test_format_large_number_boundary_m() {
        assert_eq!(crate::display::format_large_number(999_999.0), "1000.00K");
        assert_eq!(crate::display::format_large_number(1_000_000.0), "1.00M");
    }

    #[test]
    fn test_format_large_number_boundary_b() {
        assert_eq!(
            crate::display::format_large_number(999_999_999.0),
            "1000.00M"
        );
        assert_eq!(
            crate::display::format_large_number(1_000_000_000.0),
            "1.00B"
        );
    }

    #[test]
    fn test_format_large_number_very_large() {
        let result = crate::display::format_large_number(1_500_000_000_000.0);
        assert!(result.contains("B"));
    }

    // ========================================================================
    // Period tests
    // ========================================================================

    #[test]
    fn test_period_seconds_all() {
        assert_eq!(Period::Hour1.as_seconds(), 3600);
        assert_eq!(Period::Hour24.as_seconds(), 86400);
        assert_eq!(Period::Day7.as_seconds(), 604800);
        assert_eq!(Period::Day30.as_seconds(), 2592000);
    }

    #[test]
    fn test_period_labels_all() {
        assert_eq!(Period::Hour1.label(), "1 Hour");
        assert_eq!(Period::Hour24.label(), "24 Hours");
        assert_eq!(Period::Day7.label(), "7 Days");
        assert_eq!(Period::Day30.label(), "30 Days");
    }

    // ========================================================================
    // Output formatting tests
    // ========================================================================

    use crate::chains::{
        DexPair, PricePoint, Token, TokenAnalytics, TokenHolder, TokenSearchResult, TokenSocial,
    };

    fn make_test_analytics(with_dex: bool) -> TokenAnalytics {
        TokenAnalytics {
            token: Token {
                contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
            },
            chain: "ethereum".to_string(),
            holders: vec![
                TokenHolder {
                    address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                    balance: "1000000000000".to_string(),
                    formatted_balance: "1,000,000".to_string(),
                    percentage: 12.5,
                    rank: 1,
                },
                TokenHolder {
                    address: "0xabcdef1234567890abcdef1234567890abcdef12".to_string(),
                    balance: "500000000000".to_string(),
                    formatted_balance: "500,000".to_string(),
                    percentage: 6.25,
                    rank: 2,
                },
            ],
            total_holders: 150_000,
            volume_24h: if with_dex { 5_000_000.0 } else { 0.0 },
            volume_7d: if with_dex { 25_000_000.0 } else { 0.0 },
            price_usd: if with_dex { 0.9999 } else { 0.0 },
            price_change_24h: if with_dex { -0.01 } else { 0.0 },
            price_change_7d: if with_dex { 0.02 } else { 0.0 },
            liquidity_usd: if with_dex { 100_000_000.0 } else { 0.0 },
            market_cap: if with_dex {
                Some(30_000_000_000.0)
            } else {
                None
            },
            fdv: if with_dex {
                Some(30_000_000_000.0)
            } else {
                None
            },
            total_supply: Some("30000000000".to_string()),
            circulating_supply: Some("28000000000".to_string()),
            price_history: vec![
                PricePoint {
                    timestamp: 1700000000,
                    price: 0.9998,
                },
                PricePoint {
                    timestamp: 1700003600,
                    price: 0.9999,
                },
            ],
            volume_history: vec![],
            holder_history: vec![],
            dex_pairs: if with_dex {
                vec![DexPair {
                    dex_name: "Uniswap V3".to_string(),
                    pair_address: "0xpair".to_string(),
                    base_token: "USDC".to_string(),
                    quote_token: "WETH".to_string(),
                    price_usd: 0.9999,
                    volume_24h: 5_000_000.0,
                    liquidity_usd: 50_000_000.0,
                    price_change_24h: -0.01,
                    buys_24h: 1000,
                    sells_24h: 900,
                    buys_6h: 300,
                    sells_6h: 250,
                    buys_1h: 50,
                    sells_1h: 45,
                    pair_created_at: Some(1600000000),
                    url: Some("https://dexscreener.com/ethereum/0xpair".to_string()),
                }]
            } else {
                vec![]
            },
            fetched_at: 1700003600,
            top_10_concentration: Some(35.5),
            top_50_concentration: Some(55.0),
            top_100_concentration: Some(65.0),
            price_change_6h: 0.01,
            price_change_1h: -0.005,
            total_buys_24h: 1000,
            total_sells_24h: 900,
            total_buys_6h: 300,
            total_sells_6h: 250,
            total_buys_1h: 50,
            total_sells_1h: 45,
            token_age_hours: Some(25000.0),
            image_url: None,
            websites: vec!["https://www.centre.io/usdc".to_string()],
            socials: vec![TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/circle".to_string(),
            }],
            dexscreener_url: Some("https://dexscreener.com/ethereum/0xpair".to_string()),
        }
    }

    fn make_test_crawl_args() -> CrawlArgs {
        CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 10,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: false,
            save: false,
        }
    }

    #[test]
    fn test_output_table_with_dex_data() {
        let analytics = make_test_analytics(true);
        let args = make_test_crawl_args();
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_explorer_only() {
        let analytics = make_test_analytics(false);
        let args = make_test_crawl_args();
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_no_holders() {
        let mut analytics = make_test_analytics(false);
        analytics.holders = vec![];
        analytics.total_holders = 0;
        analytics.top_10_concentration = None;
        analytics.top_50_concentration = None;
        let args = make_test_crawl_args();
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_csv() {
        let analytics = make_test_analytics(true);
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_csv_no_market_cap() {
        let mut analytics = make_test_analytics(true);
        analytics.market_cap = None;
        analytics.fdv = None;
        analytics.top_10_concentration = None;
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_csv_no_holders() {
        let mut analytics = make_test_analytics(true);
        analytics.holders = vec![];
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_no_charts() {
        let analytics = make_test_analytics(true);
        let mut args = make_test_crawl_args();
        args.no_charts = true;
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_no_market_cap() {
        let mut analytics = make_test_analytics(true);
        analytics.market_cap = None;
        analytics.fdv = None;
        analytics.top_10_concentration = None;
        let args = make_test_crawl_args();
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_explorer_with_concentration() {
        let mut analytics = make_test_analytics(false);
        analytics.top_10_concentration = Some(40.0);
        analytics.top_50_concentration = Some(60.0);
        let result = output_table_explorer_only(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_explorer_no_supply() {
        let mut analytics = make_test_analytics(false);
        analytics.total_supply = None;
        let result = output_table_explorer_only(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_multiple_pairs() {
        let mut analytics = make_test_analytics(true);
        for i in 0..8 {
            analytics.dex_pairs.push(DexPair {
                dex_name: format!("DEX {}", i),
                pair_address: format!("0xpair{}", i),
                base_token: "USDC".to_string(),
                quote_token: "WETH".to_string(),
                price_usd: 0.9999,
                volume_24h: 1_000_000.0 - (i as f64 * 100_000.0),
                liquidity_usd: 10_000_000.0 - (i as f64 * 1_000_000.0),
                price_change_24h: 0.0,
                buys_24h: 100,
                sells_24h: 90,
                buys_6h: 30,
                sells_6h: 25,
                buys_1h: 5,
                sells_1h: 4,
                pair_created_at: None,
                url: None,
            });
        }
        let args = make_test_crawl_args();
        // Should only show top 5
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    // ========================================================================
    // CrawlArgs CLI parsing tests
    // ========================================================================

    #[test]
    fn test_crawl_args_with_report() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            crawl: CrawlArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "--report",
            "output.md",
        ])
        .unwrap();

        assert_eq!(
            cli.crawl.report,
            Some(std::path::PathBuf::from("output.md"))
        );
    }

    #[test]
    fn test_crawl_args_with_chain_and_period() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            crawl: CrawlArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "--chain",
            "polygon",
            "--period",
            "7d",
            "--no-charts",
            "--yes",
            "--save",
        ])
        .unwrap();

        assert_eq!(cli.crawl.chain, "polygon");
        assert!(matches!(cli.crawl.period, Period::Day7));
        assert!(cli.crawl.no_charts);
        assert!(cli.crawl.yes);
        assert!(cli.crawl.save);
    }

    #[test]
    fn test_crawl_args_all_periods() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            crawl: CrawlArgs,
        }

        for (period_str, expected) in [
            ("1h", Period::Hour1),
            ("24h", Period::Hour24),
            ("7d", Period::Day7),
            ("30d", Period::Day30),
        ] {
            let cli = TestCli::try_parse_from(["test", "token", "--period", period_str]).unwrap();
            assert_eq!(cli.crawl.period.as_seconds(), expected.as_seconds());
        }
    }

    // ========================================================================
    // JSON serialization test for TokenAnalytics
    // ========================================================================

    #[test]
    fn test_analytics_json_serialization() {
        let analytics = make_test_analytics(true);
        let json = serde_json::to_string(&analytics).unwrap();
        assert!(json.contains("USDC"));
        assert!(json.contains("USD Coin"));
        assert!(json.contains("ethereum"));
        assert!(json.contains("0.9999"));
    }

    #[test]
    fn test_analytics_json_no_optional_fields() {
        let mut analytics = make_test_analytics(false);
        analytics.market_cap = None;
        analytics.fdv = None;
        analytics.total_supply = None;
        analytics.top_10_concentration = None;
        analytics.top_50_concentration = None;
        analytics.top_100_concentration = None;
        analytics.token_age_hours = None;
        analytics.dexscreener_url = None;
        let json = serde_json::to_string(&analytics).unwrap();
        assert!(!json.contains("market_cap"));
        assert!(!json.contains("fdv"));
    }

    // ========================================================================
    // End-to-end tests using MockClientFactory
    // ========================================================================

    use crate::chains::mocks::{MockClientFactory, MockDexSource};

    fn mock_factory_for_crawl() -> MockClientFactory {
        let mut factory = MockClientFactory::new();
        // Provide complete DexTokenData so crawl succeeds
        factory.mock_dex = MockDexSource::new();
        factory
    }

    #[tokio::test]
    async fn test_run_crawl_json_output() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Json,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_table_output() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_csv_output() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Csv,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_symbol_resolution_via_factory_dex() {
        // Verifies resolve_token_input uses DexDataSource from factory (not DexClient::new)
        let mut factory = MockClientFactory::new();
        factory.mock_dex.search_results = vec![TokenSearchResult {
            address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            symbol: "MOCK".to_string(),
            name: "Mock Token".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(1.0),
            volume_24h: 1_000_000.0,
            liquidity_usd: 5_000_000.0,
            market_cap: Some(100_000_000.0),
        }];
        // Make token_data use the same address so fetch succeeds
        if let Some(ref mut td) = factory.mock_dex.token_data {
            td.address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string();
        }

        let config = Config::default();
        let args = CrawlArgs {
            token: "MOCK".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Json,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_no_dex_data_evm() {
        let config = Config::default();
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = None; // No DEX data → falls back to explorer
        factory.mock_client.token_info = Some(Token {
            contract_address: "0xtoken".to_string(),
            symbol: "TEST".to_string(),
            name: "Test Token".to_string(),
            decimals: 18,
        });
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Json,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_table_no_charts() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_with_charts() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour1,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: false, // Charts enabled
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_day7_period() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Day7,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_day30_period() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Day30,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_with_charts() {
        let analytics = make_test_analytics(true);
        let mut args = make_test_crawl_args();
        args.no_charts = false; // Enable charts
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_explorer_short_addresses() {
        let mut analytics = make_test_analytics(false);
        analytics.holders = vec![TokenHolder {
            address: "0xshort".to_string(), // Short address
            balance: "100".to_string(),
            formatted_balance: "100".to_string(),
            percentage: 1.0,
            rank: 1,
        }];
        let result = output_table_explorer_only(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_csv_with_all_fields() {
        let analytics = make_test_analytics(true);
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_crawl_with_report() {
        let config = Config::default();
        let factory = mock_factory_for_crawl();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = CrawlArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 5,
            format: OutputFormat::Table,
            no_charts: true,
            report: Some(tmp.path().to_path_buf()),
            yes: true,
            save: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
        // Report file should exist and contain markdown
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("Token Analysis Report"));
    }

    // ========================================================================
    // Additional output formatting coverage
    // ========================================================================

    #[test]
    fn test_output_table_explorer_long_address_truncation() {
        let mut analytics = make_test_analytics(false);
        analytics.holders = vec![TokenHolder {
            address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            balance: "1000000".to_string(),
            formatted_balance: "1,000,000".to_string(),
            percentage: 50.0,
            rank: 1,
        }];
        let result = output_table_explorer_only(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_empty_pairs() {
        let mut analytics = make_test_analytics(true);
        analytics.dex_pairs = vec![];
        let args = make_test_crawl_args();
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_explorer_no_concentration() {
        let mut analytics = make_test_analytics(false);
        analytics.top_10_concentration = None;
        analytics.top_50_concentration = None;
        analytics.top_100_concentration = None;
        let result = output_table_explorer_only(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_top_10_only() {
        let mut analytics = make_test_analytics(true);
        analytics.top_10_concentration = Some(25.0);
        analytics.top_50_concentration = None;
        analytics.top_100_concentration = None;
        let args = make_test_crawl_args();
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_top_100_concentration() {
        let mut analytics = make_test_analytics(true);
        analytics.top_10_concentration = Some(20.0);
        analytics.top_50_concentration = Some(45.0);
        analytics.top_100_concentration = Some(65.0);
        let args = make_test_crawl_args();
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_csv_with_market_cap_and_fdv() {
        let mut analytics = make_test_analytics(true);
        analytics.market_cap = Some(1_000_000_000.0);
        analytics.fdv = Some(1_500_000_000.0);
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_routing_has_dex_data() {
        let analytics = make_test_analytics(true);
        assert!(analytics.price_usd > 0.0);
        let args = make_test_crawl_args();
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_routing_no_dex_data() {
        let analytics = make_test_analytics(false);
        assert_eq!(analytics.price_usd, 0.0);
        let args = make_test_crawl_args();
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_large_number_negative() {
        let result = crate::display::format_large_number(-1_000_000.0);
        assert!(result.contains("M") || result.contains("-"));
    }

    #[test]
    fn test_select_token_auto_select() {
        let results = vec![TokenSearchResult {
            address: "0xtoken".to_string(),
            symbol: "TKN".to_string(),
            name: "Test Token".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(10.0),
            volume_24h: 100000.0,
            liquidity_usd: 500000.0,
            market_cap: Some(1000000.0),
        }];
        let selected = select_token(&results, true).unwrap();
        assert_eq!(selected.symbol, "TKN");
    }

    #[test]
    fn test_select_token_single_result() {
        let results = vec![TokenSearchResult {
            address: "0xtoken".to_string(),
            symbol: "SINGLE".to_string(),
            name: "Single Token".to_string(),
            chain: "ethereum".to_string(),
            price_usd: None,
            volume_24h: 0.0,
            liquidity_usd: 0.0,
            market_cap: None,
        }];
        // Single result auto-selects even without auto_select flag
        let selected = select_token(&results, false).unwrap();
        assert_eq!(selected.symbol, "SINGLE");
    }

    #[test]
    fn test_output_table_with_dex_with_holders() {
        let mut analytics = make_test_analytics(true);
        analytics.holders = vec![
            TokenHolder {
                address: "0xholder1".to_string(),
                balance: "1000000".to_string(),
                formatted_balance: "1,000,000".to_string(),
                percentage: 30.0,
                rank: 1,
            },
            TokenHolder {
                address: "0xholder2".to_string(),
                balance: "500000".to_string(),
                formatted_balance: "500,000".to_string(),
                percentage: 15.0,
                rank: 2,
            },
        ];
        let args = make_test_crawl_args();
        let result = output_table_with_dex(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_json() {
        let analytics = make_test_analytics(true);
        let result = serde_json::to_string_pretty(&analytics);
        assert!(result.is_ok());
    }

    // ========================================================================
    // select_token_impl tests
    // ========================================================================

    fn make_search_results() -> Vec<TokenSearchResult> {
        vec![
            TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 1_000_000.0,
                liquidity_usd: 500_000_000.0,
                market_cap: Some(30_000_000_000.0),
            },
            TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "USD Coin on Polygon".to_string(),
                address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
                chain: "polygon".to_string(),
                price_usd: Some(0.9999),
                volume_24h: 500_000.0,
                liquidity_usd: 100_000_000.0,
                market_cap: None,
            },
            TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "Very Long Token Name That Should Be Truncated To Fit".to_string(),
                address: "0x1234567890abcdef".to_string(),
                chain: "arbitrum".to_string(),
                price_usd: None,
                volume_24h: 0.0,
                liquidity_usd: 50_000.0,
                market_cap: None,
            },
        ]
    }

    #[test]
    fn test_select_token_impl_auto_select_multi() {
        let results = make_search_results();
        let mut writer = Vec::new();
        let mut reader = std::io::Cursor::new(b"" as &[u8]);

        let selected = select_token_impl(&results, true, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "USDC");
        assert_eq!(selected.chain, "ethereum");
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Selected:"));
    }

    #[test]
    fn test_select_token_impl_single_result() {
        let results = vec![make_search_results().remove(0)];
        let mut writer = Vec::new();
        let mut reader = std::io::Cursor::new(b"" as &[u8]);

        let selected = select_token_impl(&results, false, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "USDC");
    }

    #[test]
    fn test_select_token_user_selects_second() {
        let results = make_search_results();
        let input = b"2\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let selected = select_token_impl(&results, false, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.chain, "polygon");
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Found 3 matching tokens"));
        assert!(output.contains("USDC"));
    }

    #[test]
    fn test_select_token_shows_address_column() {
        let results = make_search_results();
        let input = b"1\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        select_token_impl(&results, false, &mut reader, &mut writer).unwrap();
        let output = String::from_utf8(writer).unwrap();

        // Table header should include Address column
        assert!(output.contains("Address"));
        // Abbreviated addresses should appear
        assert!(output.contains("0xA0b869...06eB48"));
        assert!(output.contains("0x2791Bc...a84174"));
    }

    #[test]
    fn test_abbreviate_address() {
        assert_eq!(
            abbreviate_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            "0xA0b869...06eB48"
        );
        // Short address is not abbreviated
        assert_eq!(abbreviate_address("0x1234abcd"), "0x1234abcd");
    }

    #[test]
    fn test_select_token_user_selects_third() {
        let results = make_search_results();
        let input = b"3\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let selected = select_token_impl(&results, false, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.chain, "arbitrum");
        // Long name should be truncated in output
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("..."));
    }

    #[test]
    fn test_select_token_invalid_input() {
        let results = make_search_results();
        let input = b"abc\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, false, &mut reader, &mut writer);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid selection")
        );
    }

    #[test]
    fn test_select_token_out_of_range_zero() {
        let results = make_search_results();
        let input = b"0\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, false, &mut reader, &mut writer);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Selection must be between")
        );
    }

    #[test]
    fn test_select_token_out_of_range_high() {
        let results = make_search_results();
        let input = b"99\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, false, &mut reader, &mut writer);
        assert!(result.is_err());
    }

    // ========================================================================
    // prompt_save_alias_impl tests
    // ========================================================================

    #[test]
    fn test_prompt_save_alias_yes() {
        let input = b"y\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        assert!(prompt_save_alias_impl(&mut reader, &mut writer));
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Save this token"));
    }

    #[test]
    fn test_prompt_save_alias_yes_full() {
        let input = b"yes\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        assert!(prompt_save_alias_impl(&mut reader, &mut writer));
    }

    #[test]
    fn test_prompt_save_alias_no() {
        let input = b"n\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        assert!(!prompt_save_alias_impl(&mut reader, &mut writer));
    }

    #[test]
    fn test_prompt_save_alias_empty() {
        let input = b"\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        assert!(!prompt_save_alias_impl(&mut reader, &mut writer));
    }

    // ========================================================================
    // output_table and output_csv tests
    // ========================================================================

    #[test]
    fn test_output_csv_no_panic() {
        let analytics = create_test_analytics_minimal();
        let result = output_csv(&analytics);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_no_dex_data() {
        // analytics with price_usd=0 → explorer-only output
        let analytics = create_test_analytics_minimal();
        let args = CrawlArgs {
            token: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 10,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: false,
            save: false,
        };
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_data_no_charts() {
        let mut analytics = create_test_analytics_minimal();
        analytics.price_usd = 1.0;
        analytics.volume_24h = 1_000_000.0;
        analytics.liquidity_usd = 500_000.0;
        analytics.market_cap = Some(1_000_000_000.0);
        analytics.fdv = Some(2_000_000_000.0);

        let args = CrawlArgs {
            token: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 10,
            format: OutputFormat::Table,
            no_charts: true,
            report: None,
            yes: false,
            save: false,
        };
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_table_with_dex_data_and_charts() {
        let mut analytics = create_test_analytics_minimal();
        analytics.price_usd = 1.0;
        analytics.volume_24h = 1_000_000.0;
        analytics.liquidity_usd = 500_000.0;
        analytics.price_history = vec![
            crate::chains::PricePoint {
                timestamp: 1,
                price: 0.99,
            },
            crate::chains::PricePoint {
                timestamp: 2,
                price: 1.01,
            },
        ];
        analytics.volume_history = vec![
            crate::chains::VolumePoint {
                timestamp: 1,
                volume: 50000.0,
            },
            crate::chains::VolumePoint {
                timestamp: 2,
                volume: 60000.0,
            },
        ];

        let args = CrawlArgs {
            token: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            period: Period::Hour24,
            holders_limit: 10,
            format: OutputFormat::Table,
            no_charts: false,
            report: None,
            yes: false,
            save: false,
        };
        let result = output_table(&analytics, &args);
        assert!(result.is_ok());
    }

    fn create_test_analytics_minimal() -> TokenAnalytics {
        TokenAnalytics {
            token: Token {
                contract_address: "0xtest".to_string(),
                symbol: "TEST".to_string(),
                name: "Test Token".to_string(),
                decimals: 18,
            },
            chain: "ethereum".to_string(),
            holders: Vec::new(),
            total_holders: 0,
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
            fetched_at: 0,
            top_10_concentration: None,
            top_50_concentration: None,
            top_100_concentration: None,
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
        }
    }
}
