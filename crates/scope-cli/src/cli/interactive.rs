//! # Interactive Mode
//!
//! `scope interactive` (also `scope tui` and `scope shell`) opens the
//! unified TUI in [`crate::cli::tui`]. This module keeps its arguments and
//! the [`SessionContext`] that persists between commands. The chain
//! defaults to `auto`, meaning each command infers the chain from its
//! input (e.g., `0x…` → Ethereum/EVM, `T…` → Tron, base58 → Solana).
//! Users can pin a chain with `chain solana` and unlock with `chain auto`.
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

use clap::Args;
use scope::chains::ChainClientFactory;
use scope::config::{Config, OutputFormat};
use scope::error::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

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
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    /// Loads session context from `path`. A missing or unreadable file is
    /// the default context.
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_yaml::from_str(&contents).ok())
            .unwrap_or_default()
    }

    /// Saves session context to file.
    pub fn save(&self) -> Result<()> {
        match Self::context_path() {
            Some(path) => self.save_to(&path),
            None => Ok(()),
        }
    }

    /// Saves session context to `path`, creating its directory.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_yaml::to_string(self)
            .map_err(|e| scope::error::ScopeError::Export(e.to_string()))?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}

/// Opens the unified TUI.
///
/// # Errors
///
/// Returns the TUI's terminal or save error.
pub async fn run(
    args: InteractiveArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    crate::cli::tui::run(args.no_banner, config, clients).await
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
    fn test_session_context_save_to_and_load_from() {
        // The TUI restores the pinned chain and toggles on the next start.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("session.yaml");
        let ctx = SessionContext {
            chain: "solana".into(),
            include_txs: true,
            last_address: Some("addr".into()),
            ..Default::default()
        };
        ctx.save_to(&path).unwrap();
        let back = SessionContext::load_from(&path);
        assert_eq!(back.chain, "solana");
        assert!(back.include_txs);
        assert_eq!(back.last_address.as_deref(), Some("addr"));
    }

    #[test]
    fn test_session_context_load_from_missing_or_corrupt_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SessionContext::load_from(&dir.path().join("none.yaml")).is_auto_chain());
        let bad = dir.path().join("bad.yaml");
        std::fs::write(&bad, "chain: [unclosed").unwrap();
        assert!(SessionContext::load_from(&bad).is_auto_chain());
    }
}
