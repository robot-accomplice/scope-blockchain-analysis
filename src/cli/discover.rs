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
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope discover
  scope discover --source boosts --chain solana
  scope discover --source top-boosts --limit 30 --format json")]
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
    run_with_client(args, format, &DexClient::new()).await
}

/// Run the discover command with a provided DEX client (for testing).
pub async fn run_with_client(
    args: DiscoverArgs,
    format: OutputFormat,
    client: &DexClient,
) -> Result<()> {
    let sp = crate::cli::progress::Spinner::new(&format!(
        "Discovering {} tokens...",
        match args.source {
            DiscoverSource::Profiles => "featured",
            DiscoverSource::Boosts => "boosted",
            DiscoverSource::TopBoosts => "top boosted",
        }
    ));

    let tokens = match args.source {
        DiscoverSource::Profiles => client.get_token_profiles().await?,
        DiscoverSource::Boosts => client.get_token_boosts().await?,
        DiscoverSource::TopBoosts => client.get_token_boosts_top().await?,
    };

    sp.finish("Tokens loaded.");

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
            use crate::display::terminal as t;

            let title = format!(
                "{} ({})",
                match args.source {
                    DiscoverSource::Profiles => "Featured Token Profiles",
                    DiscoverSource::Boosts => "Recently Boosted Tokens",
                    DiscoverSource::TopBoosts => "Top Boosted Tokens",
                },
                filtered.len()
            );
            println!("{}", t::section_header(&title));
            println!("{}", t::kv_row("Results", &filtered.len().to_string()));

            for (i, t) in filtered.iter().enumerate() {
                let desc = t.description.as_deref().unwrap_or("-");
                let row_text = format!(
                    "{} | {} | {}",
                    t.chain_id,
                    truncate_address(&t.token_address),
                    desc
                );
                println!("{}", t::numbered_row(i + 1, &row_text));
                println!("{}", t::detail_row(&t.url));
            }

            println!("{}", t::section_footer());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::DexClient;
    use crate::config::OutputFormat;

    fn discover_json_body() -> String {
        r#"[
            {
                "chainId": "ethereum",
                "tokenAddress": "0x1234567890123456789012345678901234567890",
                "url": "https://dexscreener.com/ethereum/0x1234",
                "description": "A test token"
            },
            {
                "chainId": "solana",
                "tokenAddress": "So11111111111111111111111111111111111111112",
                "url": "https://dexscreener.com/solana/So11",
                "description": null
            }
        ]"#
        .to_string()
    }

    #[tokio::test]
    async fn test_discover_profiles_table() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_json_body())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::Profiles,
            chain: None,
            limit: 15,
            format: None,
        };
        let result = run_with_client(args, OutputFormat::Table, &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_discover_boosts_with_chain_filter() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-boosts/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_json_body())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::Boosts,
            chain: Some("ethereum".to_string()),
            limit: 5,
            format: None,
        };
        let result = run_with_client(args, OutputFormat::Table, &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_discover_top_boosts_json() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-boosts/top/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_json_body())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::TopBoosts,
            chain: None,
            limit: 10,
            format: Some(OutputFormat::Json),
        };
        let result = run_with_client(args, OutputFormat::Json, &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_discover_empty_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[]")
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::Profiles,
            chain: None,
            limit: 15,
            format: None,
        };
        let result = run_with_client(args, OutputFormat::Table, &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_discover_csv_format() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_json_body())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::Profiles,
            chain: None,
            limit: 15,
            format: Some(OutputFormat::Csv),
        };
        let result = run_with_client(args, OutputFormat::Csv, &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_discover_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(500)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let args = DiscoverArgs {
            source: DiscoverSource::Profiles,
            chain: None,
            limit: 15,
            format: None,
        };
        let result = run_with_client(args, OutputFormat::Table, &client).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate_address_short() {
        assert_eq!(truncate_address("0x1234"), "0x1234");
    }

    #[test]
    fn test_truncate_address_long() {
        let addr = "0x1234567890123456789012345678901234567890";
        assert_eq!(truncate_address(addr), "0x12345678...34567890");
    }
}
