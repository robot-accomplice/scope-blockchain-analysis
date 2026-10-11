//! # Command Palette
//!
//! `Ctrl-K` opens a fuzzy list of every command (from the clap definition),
//! the session commands, and recent history. `Enter` puts the selection in
//! the input line; it does not run it.

use super::exec::SESSION_COMMANDS;
use crate::cli::Cli;
use clap::CommandFactory;

/// The most history entries offered.
const HISTORY_ITEMS: usize = 50;

/// One palette entry.
#[derive(Debug, Clone)]
pub struct PaletteItem {
    /// What the list shows and the query matches.
    pub label: String,
    /// The one-line description.
    pub description: String,
    /// What goes in the input line.
    pub fill: String,
}

/// All palette entries: history (newest first), then CLI commands, then
/// session commands.
pub fn items(history: &[String]) -> Vec<PaletteItem> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in history.iter().rev() {
        if seen.insert(line.clone()) && seen.len() <= HISTORY_ITEMS {
            out.push(PaletteItem {
                label: line.clone(),
                description: "history".into(),
                fill: line.clone(),
            });
        }
    }
    for sub in Cli::command()
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
    {
        let leaves: Vec<(String, String)> = if sub.has_subcommands() {
            sub.get_subcommands()
                .filter(|s| !s.is_hide_set())
                .map(|l| {
                    let about = l.get_about().map(|a| a.to_string()).unwrap_or_default();
                    (format!("{} {}", sub.get_name(), l.get_name()), about)
                })
                .collect()
        } else {
            let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
            vec![(sub.get_name().to_string(), about)]
        };
        for (label, description) in leaves {
            out.push(PaletteItem {
                fill: format!("{} ", label),
                label,
                description,
            });
        }
    }
    for (word, usage, what) in SESSION_COMMANDS {
        out.push(PaletteItem {
            label: word.to_string(),
            description: format!("{} — {}", usage, what),
            fill: format!("{} ", word),
        });
    }
    out
}

/// Fuzzy score of `query` against `text`, case-insensitive. `None` when the
/// letters of `query` are not all in `text` in order. Higher is better.
pub fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        // Everything matches an empty query, equally.
        return Some(0);
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut total = 0;
    let mut pos = 0;
    let mut prev: Option<usize> = None;
    for q in query.to_lowercase().chars() {
        let found = (pos..text.len()).find(|&i| text[i] == q)?;
        if found == 0 {
            total += 20;
        } else if matches!(text[found - 1], ' ' | '-') {
            total += 5;
        }
        match prev {
            Some(p) if p + 1 == found => total += 10,
            Some(p) => total -= (found - p - 1) as i32,
            None => total -= found as i32,
        }
        prev = Some(found);
        pos = found + 1;
    }
    Some(total - (text.len() as i32 / 8))
}

/// The open palette.
pub struct PaletteState {
    /// The typed query.
    pub query: String,
    /// All entries.
    pub items: Vec<PaletteItem>,
    /// Index into `matches()`.
    pub selected: usize,
}

impl PaletteState {
    /// A palette over `items` with an empty query.
    pub fn new(items: Vec<PaletteItem>) -> Self {
        Self {
            query: String::new(),
            items,
            selected: 0,
        }
    }

    /// Indexes of the matching items, best first. An empty query keeps the
    /// original order.
    pub fn matches(&self) -> Vec<usize> {
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| score(&self.query, &it.label).map(|s| (s, i)))
            .collect();
        if !self.query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        scored.into_iter().map(|(_, i)| i).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_visible_leaf_command_is_an_item() {
        // The same guard as the parity test: a new command shows up here.
        let labels: Vec<String> = items(&[]).into_iter().map(|i| i.label).collect();
        for sub in Cli::command()
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
        {
            if sub.has_subcommands() {
                for leaf in sub.get_subcommands().filter(|s| !s.is_hide_set()) {
                    let l = format!("{} {}", sub.get_name(), leaf.get_name());
                    assert!(labels.contains(&l), "{}", l);
                }
            } else {
                assert!(
                    labels.contains(&sub.get_name().to_string()),
                    "{}",
                    sub.get_name()
                );
            }
        }
        assert!(labels.contains(&"chain".to_string()));
    }

    #[test]
    fn test_history_comes_first_newest_first_and_unique() {
        let h = vec![
            "venues list".to_string(),
            "tx 0x1".to_string(),
            "venues list".to_string(),
        ];
        let it = items(&h);
        assert_eq!(it[0].label, "venues list");
        assert_eq!(it[1].label, "tx 0x1");
        assert_eq!(
            it.iter().filter(|i| i.label == "venues list").count(),
            2,
            "history + command"
        );
    }

    #[test]
    fn test_score_prefers_prefix_and_consecutive_letters() {
        assert!(score("msum", "market summary").is_some());
        assert!(score("xyz", "market summary").is_none());
        assert!(score("mark", "market summary") > score("mark", "compliance risk mark"));
        assert!(score("addr", "address") > score("addr", "address-book add"));
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn test_matches_filters_and_ranks() {
        let mut p = PaletteState::new(items(&[]));
        p.query = "msum".into();
        let m = p.matches();
        assert_eq!(p.items[m[0]].label, "market summary");
    }
}
