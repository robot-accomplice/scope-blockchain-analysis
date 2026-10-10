//! # Tab Completion
//!
//! Completes the last word of the input line from the clap definition and
//! the [`Vocab`]. Commands, flags and enum values come from `Cli::command()`,
//! so a new command or flag completes with no extra code.

use super::exec::SESSION_COMMANDS;
use super::vocab::Vocab;
use crate::cli::Cli;
use clap::{Arg, Command, CommandFactory};

/// The candidates for the word that ends the line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// Byte index where the word starts. A candidate replaces `line[start..]`.
    pub start: usize,
    /// Sorted, unique candidates that start with the word.
    pub candidates: Vec<String>,
}

/// Completes the last word of `line`.
pub fn complete(line: &str, vocab: &Vocab) -> Completion {
    let start = line.rfind(' ').map_or(0, |i| i + 1);
    let word = &line[start..];
    // Inside an open quote the word boundaries are not known.
    if line[..start].matches('"').count() % 2 == 1 || word.starts_with('"') {
        return Completion {
            start,
            candidates: Vec::new(),
        };
    }
    let before: Vec<&str> = line[..start].split_whitespace().collect();

    let mut root = Cli::command();
    // build() copies global flags (--ai, address-book's --format) down to
    // the subcommands, so they complete there too.
    root.build();
    let mut cmd: &Command = &root;
    let mut positional_seen = false;
    for w in &before {
        if w.starts_with('-') {
            continue;
        }
        match (positional_seen, cmd.find_subcommand(w)) {
            (false, Some(sub)) => cmd = sub,
            _ => positional_seen = true,
        }
    }
    let at_root = std::ptr::eq(cmd, &root);
    if at_root && positional_seen {
        // The first word is not a known command.
        return Completion {
            start,
            candidates: Vec::new(),
        };
    }

    let pool: Vec<String> = if let Some(arg) = before
        .last()
        .filter(|p| p.starts_with('-'))
        .and_then(|p| find_arg(cmd, p))
        .filter(|a| a.get_action().takes_values())
    {
        values_for(arg, vocab)
    } else if word.starts_with('@') {
        vocab.labels.iter().map(|l| format!("@{}", l)).collect()
    } else if word.starts_with('-') {
        cmd.get_arguments()
            .filter(|a| !a.is_hide_set())
            .filter_map(|a| a.get_long().map(|l| format!("--{}", l)))
            .chain(std::iter::once("--help".to_string()))
            .collect()
    } else if cmd.has_subcommands() && !positional_seen {
        let mut names: Vec<String> = cmd
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
            .map(|s| s.get_name().to_string())
            .collect();
        if at_root {
            names.extend(SESSION_COMMANDS.iter().map(|(w, _, _)| w.to_string()));
        }
        names
    } else {
        vocab.aliases.clone()
    };

    let mut candidates: Vec<String> = pool.into_iter().filter(|c| c.starts_with(word)).collect();
    candidates.sort();
    candidates.dedup();
    Completion { start, candidates }
}

/// Finds the argument that a flag word names (`--venue`, `--venue=x`, `-c`).
fn find_arg<'a>(cmd: &'a Command, flag: &str) -> Option<&'a Arg> {
    if let Some(long) = flag.strip_prefix("--") {
        let long = long.split('=').next().unwrap_or(long);
        cmd.get_arguments().find(|a| a.get_long() == Some(long))
    } else {
        let short = flag.strip_prefix('-')?.chars().next()?;
        cmd.get_arguments().find(|a| a.get_short() == Some(short))
    }
}

/// The values a flag accepts: its enum values, or a store from the vocab.
fn values_for(arg: &Arg, vocab: &Vocab) -> Vec<String> {
    let enums: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_string())
        .collect();
    if !enums.is_empty() {
        return enums;
    }
    match arg.get_long() {
        Some("venue") => vocab.venues.clone(),
        Some("chain") => vocab.chains.clone(),
        _ => Vec::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocab {
        Vocab {
            labels: vec!["cold".into(), "hot".into()],
            aliases: vec!["USDC".into(), "USDT".into()],
            venues: vec!["binance".into(), "bitget".into(), "okx".into()],
            chains: vec!["ethereum".into(), "solana".into()],
        }
    }

    fn cands(line: &str) -> Vec<String> {
        complete(line, &vocab()).candidates
    }

    #[test]
    fn test_root_offers_cli_and_session_commands() {
        let c = cands("ad");
        assert!(c.contains(&"address".to_string()));
        assert!(c.contains(&"address-book".to_string()));
        assert_eq!(cands("cha"), vec!["chain"]);
    }

    #[test]
    fn test_subcommands_flags_and_enum_values() {
        assert_eq!(cands("market su"), vec!["summary"]);
        assert!(cands("market summary USDC --ve").contains(&"--venue".to_string()));
        assert_eq!(cands("address 0x1 --format c"), vec!["csv"]);
    }

    #[test]
    fn test_dynamic_values() {
        assert_eq!(
            cands("market summary USDC --venue bi"),
            vec!["binance", "bitget"]
        );
        assert_eq!(cands("address 0x1 --chain so"), vec!["solana"]);
        assert_eq!(cands("address @co"), vec!["@cold"]);
        assert_eq!(cands("crawl US"), vec!["USDC", "USDT"]);
    }

    #[test]
    fn test_start_points_at_the_word() {
        let c = complete("market su", &vocab());
        assert_eq!(c.start, "market ".len());
    }

    #[test]
    fn test_global_flags_complete_under_subcommands() {
        // Review I2: `--format` on address-book is `global = true`, and the
        // root flags (--ai, --config) are global too. Spec 5.2's example.
        assert_eq!(
            cands("address-book list --format "),
            vec!["csv", "json", "markdown", "table"]
        );
        assert!(cands("market summary USDC --a").contains(&"--ai".to_string()));
    }

    #[test]
    fn test_inside_open_quote_offers_nothing() {
        // Review Focus 2.
        assert!(cands("address-book add 0x1 --label \"cold wal").is_empty());
    }

    #[test]
    fn test_empty_line_offers_commands_and_unknown_command_offers_nothing() {
        assert!(cands("").contains(&"address".to_string()));
        assert!(cands("nosuchcommand x").is_empty());
    }

    #[test]
    fn test_every_visible_subcommand_is_offered_at_the_root() {
        // One source of truth: a new CLI command is completable with no
        // extra code.
        let all = cands("");
        for sub in crate::cli::Cli::command().get_subcommands() {
            if !sub.is_hide_set() {
                assert!(
                    all.contains(&sub.get_name().to_string()),
                    "{}",
                    sub.get_name()
                );
            }
        }
    }
}
