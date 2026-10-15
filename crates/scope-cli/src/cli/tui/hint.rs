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

    // Walk the words after the command (#66): a flag's value is not a
    // positional, an unknown flag gets its own hint, and a flag that still
    // waits for its value names it.
    let ends_in_space = line.ends_with(char::is_whitespace);
    let mut typed = 0;
    let mut pending: Option<&clap::Arg> = None;
    for (i, w) in words[rest..].iter().enumerate() {
        let last = rest + i + 1 == words.len();
        if pending.take().is_some() {
            continue;
        }
        if *w == "--" || !w.starts_with('-') || w.len() == 1 {
            typed += 1;
            continue;
        }
        let (flag, has_value) = match w.split_once('=') {
            Some((f, _)) => (f, true),
            None => (*w, false),
        };
        let arg = cmd.get_arguments().find(|a| {
            flag.strip_prefix("--")
                .is_some_and(|l| a.get_long() == Some(l))
                || flag
                    .strip_prefix('-')
                    .and_then(|s| s.chars().next())
                    .is_some_and(|c| !flag.starts_with("--") && a.get_short() == Some(c))
        });
        let Some(arg) = arg else {
            // A flag still being typed is not unknown yet.
            if last && !ends_in_space {
                continue;
            }
            return Some(Hint {
                text: format!(
                    "unknown flag `{flag}`; see `{} --help`",
                    path[1..].join(" ")
                ),
                next: None,
            });
        };
        if arg.get_action().takes_values() && !has_value {
            pending = Some(arg);
        }
    }
    if let Some(arg) = pending {
        return Some(Hint {
            text,
            next: Some(value_hint(arg)),
        });
    }
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
/// Most allowed values a hint lists.
const MAX_LISTED_VALUES: usize = 6;

/// `<NAME>` for a flag's value, and its allowed values when clap knows them.
fn value_hint(arg: &clap::Arg) -> String {
    let name = arg
        .get_value_names()
        .and_then(|v| v.first())
        .map(|n| n.to_string())
        .unwrap_or_else(|| arg.get_id().to_string().to_uppercase());
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    match values.len() {
        0 => format!("<{name}>"),
        n if n <= MAX_LISTED_VALUES => format!("<{name}>: {}", values.join(" | ")),
        _ => format!("<{name}>: {} | …", values[..MAX_LISTED_VALUES].join(" | ")),
    }
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

    // #66: a flag that waits for its value names it, with its values.
    #[test]
    fn test_a_flag_waiting_for_its_value_names_it() {
        let h = hint("market summary USDC --venue ").unwrap();
        assert_eq!(h.next.as_deref(), Some("<VENUE>"), "{h:?}");
        let h = hint("market summary USDC --format ").unwrap();
        assert!(h.next.as_deref().unwrap().contains("json"), "{h:?}");
    }

    #[test]
    fn test_a_flag_value_is_not_a_positional() {
        // `--chain ethereum` before the address: the address is still next.
        let h = hint("address --chain ethereum ").unwrap();
        assert_eq!(h.next.as_deref(), Some("<ADDRESS>"), "{h:?}");
    }

    #[test]
    fn test_an_unknown_flag_is_named() {
        let h = hint("address 0x1 --nosuch ").unwrap();
        assert!(h.text.contains("unknown flag `--nosuch`"), "{}", h.text);
        // While it is still being typed, it is not unknown yet.
        assert!(!hint("address 0x1 --ch").unwrap().text.contains("unknown"));
    }
}
