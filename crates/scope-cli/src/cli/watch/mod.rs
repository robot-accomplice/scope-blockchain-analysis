//! # Watchlist runs (#47)
//!
//! The I/O side of [`scope::watch`]: [`fetch`] reads each target's metrics
//! through the compute functions the CLI commands use (never a printing
//! handler), and [`run`](mod@run) repeats the ticks on the watchlist's schedule
//! with the one repeat runner, writes the CSV trail and the report.
//!
//! The TUI's monitor wizard and grid drive these functions.

pub mod deviation;
pub mod fetch;
pub mod headless;
pub mod inline;
#[cfg(test)]
mod inline_e2e_tests;
pub mod monitor;
pub mod run;
pub mod state;
#[cfg(test)]
mod state_e2e_tests;

use crate::cli::output::Output;
use crate::cli::resolve::{DexCandidates, ResolveContext, VenueKind, picker_for, resolve};
use scope::chains::{ChainClientFactory, infer_chain_from_address};
use scope::config::Config;
use scope::error::Result;
use scope::metrics::MetricGroup;
use scope::tokens::TokenAliases;
use scope::watch::{Target, TargetKind, Watchlist};

pub use fetch::{TargetReading, Tick, tick};
pub use run::{csv_header, csv_rows, report_text, run, tick_csv_rows};

/// What the fetchers read with.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub config: &'a Config,
    pub clients: &'a dyn ChainClientFactory,
}

/// The venue IDs a watchlist may name: the built-in and the user's.
pub fn venue_ids() -> Vec<String> {
    scope::market::VenueRegistry::load()
        .map(|r| r.list().into_iter().map(str::to_string).collect())
        .unwrap_or_default()
}

/// True when `arg` names a watchlist file: it ends in `.yaml` or has a path
/// separator. Else it is the name of a saved watchlist.
pub fn is_watchlist_path(arg: &str) -> bool {
    arg.ends_with(&format!(".{}", scope::watch::file::EXTENSION))
        || arg.contains(['/', std::path::MAIN_SEPARATOR])
}

/// The file of a watchlist `arg`: itself when it is a path (see
/// [`is_watchlist_path`]), else the saved list of that name in `dir`. The
/// TUI session command and `scope watch` both read the argument this way.
pub fn watchlist_path(arg: &str, dir: &std::path::Path) -> std::path::PathBuf {
    if is_watchlist_path(arg) {
        std::path::PathBuf::from(arg)
    } else {
        scope::watch::file::path_in(dir, arg)
    }
}

/// A watchlist ready to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    /// The watchlist, with `@label` targets replaced by their address.
    pub list: Watchlist,
    /// For each target, the token contract its token metrics read. The
    /// target's id stays the symbol, which market metrics use on a venue.
    pub contracts: Vec<Option<String>>,
}

/// Makes the watchlist ready to run, once before the first tick:
/// `@label` targets become their address and chain, and a token symbol
/// that token metrics read is resolved to its contract. A symbol on many
/// chains asks the user (in the TUI, as a list); the answer holds for the
/// run.
///
/// # Errors
///
/// The address book cannot be read, or a symbol cannot be resolved.
pub async fn prepare(w: &Watchlist, src: Sources<'_>, out: &Output) -> Result<Ready> {
    let mut list = w.clone();
    let mut contracts = Vec::with_capacity(list.targets.len());
    for t in &mut list.targets {
        if let Some((address, chain)) =
            crate::cli::address_book::resolve_address_book_input(&t.id, src.config)?
        {
            t.id = address;
            t.chain.get_or_insert(chain);
        }
        if t.chain.is_none() && t.kind != TargetKind::Pair {
            t.chain = infer_chain_from_address(&t.id).map(str::to_string);
        }
        let contract = if t.kind == TargetKind::Token && t.needs(MetricGroup::Token) {
            Some(resolve_token(t, src, out).await?)
        } else {
            None
        };
        contracts.push(contract);
    }
    Ok(Ready { list, contracts })
}

/// The contract of a token target, and its chain when the target has none.
async fn resolve_token(t: &mut Target, src: Sources<'_>, out: &Output) -> Result<String> {
    if TokenAliases::is_address(t.id.trim()) {
        return Ok(t.id.trim().to_string());
    }
    let dex = src.clients.create_dex_client();
    let ctx = ResolveContext {
        chain: t.chain.clone(),
        venue: VenueKind::Dex,
        token_flag: None,
        yes: false,
        save: false,
    };
    let source = DexCandidates {
        dex: dex.as_ref(),
        out,
        fallback: Some(src.clients.http()),
    };
    let picker = picker_for(out, false);
    let mut aliases = TokenAliases::load();
    let r = resolve(&t.id, &ctx, &source, picker.as_ref(), &mut aliases, out).await?;
    if t.chain.is_none() && !r.chain.is_empty() {
        t.chain = Some(r.chain);
    }
    Ok(r.address.unwrap_or_else(|| t.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // The TUI session command and `scope watch` read the argument with the
    // same function, so a name or a path means the same in both.
    #[test]
    fn a_name_is_in_the_folder_and_a_yaml_or_separator_is_a_path() {
        let dir = Path::new("lists");
        assert_eq!(
            watchlist_path("pegs", dir),
            scope::watch::file::path_in(dir, "pegs")
        );
        for path in ["pegs.yaml", "./pegs", "sub/pegs", "C:/x/pegs"] {
            assert_eq!(watchlist_path(path, dir), Path::new(path), "{path}");
        }
        assert!(!is_watchlist_path("pegs"));
        assert!(is_watchlist_path(&format!(
            "a{}b",
            std::path::MAIN_SEPARATOR
        )));
    }
}
