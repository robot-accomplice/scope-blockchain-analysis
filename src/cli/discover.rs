//! # Discover Command
//!
//! Browse trending and boosted tokens from DexScreener.

use crate::chains::{DexClient, DiscoverToken};
use crate::config::OutputFormat;
use crate::error::Result;
use clap::{Args, ValueEnum};
use serde::Serialize;

/// Source for token discovery.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum DiscoverSource {
    /// Featured token profiles
    #[default]
    Profiles,

    /// Recently boosted tokens
    Boosts,

    /// Top boosted tokens (most active)
    TopBoosts,
}

/// Arguments for the discover command.
#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Discovery source: profiles (featured), boosts (recent), top-boosts (most active)
    #[arg(short, long, default_value = "profiles")]
    pub source: DiscoverSource,

    /// Filter by chain (e.g., ethereum, solana). Omit for all chains.
    #[arg(short, long)]
    pub chain: Option<String>,

    /// Maximum number of tokens to show
    #[arg(short, long, default_value = "15")]
    pub limit: u32,

    /// Output format
    #[arg(short, long)]
    pub format: Option<OutputFormat>,
}

#[derive(Serialize)]
struct DiscoverRow {
    chain: String,
    address: String,
    description: Option<String>,
    url: String,
}

/// Run the discover command.
pub async fn run(args: DiscoverArgs, format: OutputFormat) -> Result<()> {
    let client = DexClient::new();

    let tokens = match args.source {
        DiscoverSource::Profiles => client.get_token_profiles().await?,
        DiscoverSource::Boosts => client.get_token_boosts().await?,
        DiscoverSource::TopBoosts => client.get_token_boosts_top().await?,
    };

    let filtered: Vec<DiscoverToken> = if let Some(ref chain) = args.chain {
        let c = chain.to_lowercase();
        tokens
            .into_iter()
            .filter(|t| t.chain_id.to_lowercase() == c)
            .take(args.limit as usize)
            .collect()
    } else {
        tokens.into_iter().take(args.limit as usize).collect()
    };

    if filtered.is_empty() {
        println!("No tokens found.");
        return Ok(());
    }

    let output_format = args.format.unwrap_or(format);

    match output_format {
        OutputFormat::Json => {
            let rows: Vec<DiscoverRow> = filtered
                .iter()
                .map(|t| DiscoverRow {
                    chain: t.chain_id.clone(),
                    address: t.token_address.clone(),
                    description: t.description.clone(),
                    url: t.url.clone(),
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        OutputFormat::Table | OutputFormat::Markdown => {
            println!(
                "\n{} ({}) — limit {}",
                match args.source {
                    DiscoverSource::Profiles => "Featured Token Profiles",
                    DiscoverSource::Boosts => "Recently Boosted Tokens",
                    DiscoverSource::TopBoosts => "Top Boosted Tokens",
                },
                filtered.len(),
                args.limit
            );
            println!("{}", "-".repeat(80));
            for (i, t) in filtered.iter().enumerate() {
                let desc = t
                    .description
                    .as_deref()
                    .map(|d| {
                        let truncated = if d.len() > 60 { &d[..57] } else { d };
                        format!("{}...", truncated)
                    })
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "{:3}. {} | {} | {}",
                    i + 1,
                    t.chain_id,
                    truncate_address(&t.token_address),
                    desc
                );
                println!("     {}", t.url);
            }
        }
        OutputFormat::Csv => {
            println!("chain,address,description,url");
            for t in &filtered {
                let desc = t
                    .description
                    .as_ref()
                    .map(|d| d.replace(',', ";").replace('\n', " "))
                    .unwrap_or_else(|| "-".to_string());
                println!("{},{},\"{}\",{}", t.chain_id, t.token_address, desc, t.url);
            }
        }
    }

    Ok(())
}

fn truncate_address(addr: &str) -> String {
    if addr.len() > 20 {
        format!("{}...{}", &addr[..10], &addr[addr.len() - 8..])
    } else {
        addr.to_string()
    }
}
