//! # Inline Hints
//!
//! The dim line under the input: the usage of the command being typed, with
//! the next expected argument, or clap's suggestion for an unknown command.

use super::exec::SESSION_COMMANDS;
use crate::cli::Cli;
use clap::{Command, CommandFactory, Parser};

/// One hint line.
#[derive(Debug, PartialEq, Eq)]
pub struct Hint {
    /// The usage text or the suggestion.
    pub text: String,
    /// The next positional argument the command expects, as `<NAME>`.
    pub next: Option<String>,
}

/// The hint for `line`, or `None` for an empty line.
pub fn hint(line: &str) -> Option<Hint> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let first = *words.first()?;
    if let Some((_, usage, _)) = SESSION_COMMANDS.iter().find(|(w, _, _)| *w == first) {
        return Some(Hint {
            text: usage.to_string(),
            next: None,
        });
    }

    let mut root = Cli::command();
    root.build();
    let mut path = vec!["scope".to_string()];
    let mut cmd: &Command = &root;
    let mut rest = 0;
    for (i, w) in words.iter().enumerate() {
        match cmd.find_subcommand(w) {
            Some(sub) if !w.starts_with('-') => {
                path.push(sub.get_name().to_string());
                cmd = sub;
                rest = i + 1;
            }
            _ => break,
        }
    }
    if path.len() == 1 {
        let err = Cli::try_parse_from(["scope", first]).err()?;
        let tip = err
            .render()
            .to_string()
            .lines()
            .find(|l| l.contains("similar"))
            .map(|l| l.trim().trim_start_matches("tip:").trim().to_string())
            .unwrap_or_else(|| format!("unknown command `{}`; type `help`", first));
        return Some(Hint {
            text: tip,
            next: None,
        });
    }

    let usage = cmd
        .clone()
        .bin_name(path.join(" "))
        .render_usage()
        .to_string();
    let text = usage.trim_start_matches("Usage:").trim().to_string();
    let typed = words[rest..].iter().filter(|w| !w.starts_with('-')).count();
    let next = cmd.get_positionals().nth(typed).map(|a| {
        let name = a
            .get_value_names()
            .and_then(|v| v.first())
            .map(|n| n.to_string())
            .unwrap_or_else(|| a.get_id().to_string().to_uppercase());
        format!("<{}>", name)
    });
    Some(Hint { text, next })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_of_the_matched_command() {
        let h = hint("market summary").unwrap();
        assert!(h.text.starts_with("scope market summary"), "{}", h.text);
        assert_eq!(h.next.as_deref(), Some("<SYMBOL>"));
    }

    #[test]
    fn test_next_moves_past_typed_positionals() {
        let h = hint("address 0x1 ").unwrap();
        assert_eq!(h.next, None, "address has one positional");
    }

    #[test]
    fn test_session_command_usage() {
        assert_eq!(hint("chain").unwrap().text, "chain [name|auto]");
    }

    #[test]
    fn test_unknown_command_suggests_like_clap() {
        let h = hint("adress 0x1").unwrap();
        assert!(h.text.contains("address"), "{}", h.text);
    }

    #[test]
    fn test_empty_line_has_no_hint() {
        assert!(hint("").is_none());
        assert!(hint("   ").is_none());
    }
}
