//! # TUI Command Execution
//!
//! Turns one line of TUI input into a [`Plan`]. Session commands (`chain`,
//! `format`, `ctx`, `tokens`, …) run here and change the [`SessionContext`].
//! Every other line is parsed by the CLI's own clap definition ([`Cli`]), so
//! commands, aliases and flags are the same as on the command line.
//!
//! The session context fills in flags the user did not type, for example
//! `--chain` when a chain is pinned. A flag the user types always wins.

use crate::cli::interactive::SessionContext;
use crate::cli::output::Output;
use crate::cli::{Cli, Commands};
use crate::{err, errln, out, outln};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};
use scope::config::{Config, OutputFormat};
use scope::tokens::TokenAliases;
use std::io;

/// Where a parsed command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// In the TUI: the output goes to the output pane.
    Pane,
    /// On the plain terminal: the TUI steps aside while the command runs,
    /// because the command reads from stdin or draws its own screen.
    Suspend,
    /// Not inside the TUI. The reason tells the user what to do instead.
    Refuse(&'static str),
}

/// Decides where a command runs.
///
/// The match has no wildcard arm on purpose: a new CLI command does not
/// compile until it has a TUI route (issue #47, requirement 5).
pub fn route(command: &Commands) -> Route {
    match command {
        Commands::Address(_)
        | Commands::Tx(_)
        | Commands::Insights(_)
        | Commands::Contract(_)
        | Commands::TokenHealth(_)
        | Commands::Discover(_)
        | Commands::Market(_)
        | Commands::Venues(_)
        | Commands::Compliance(_)
        | Commands::AddressBook(_)
        | Commands::Export(_)
        | Commands::Report(_)
        | Commands::Completions(_) => Route::Pane,
        // A name or symbol search asks the user to pick a token and offers
        // to save an alias. An address or --yes never prompts.
        Commands::Crawl(args) => {
            if args.yes || TokenAliases::is_address(args.token.trim()) {
                Route::Pane
            } else {
                Route::Suspend
            }
        }
        // Only --status is free of prompts.
        Commands::Setup(args) => {
            if args.status {
                Route::Pane
            } else {
                Route::Suspend
            }
        }
        // The live dashboard draws its own screen (until phase 6 of #47).
        Commands::Monitor(_) => Route::Suspend,
        Commands::Interactive(_) => Route::Refuse("you are already in the TUI"),
        Commands::Web(_) => Route::Refuse(
            "the web server runs in the foreground; start it from a shell with `scope web`",
        ),
    }
}

/// A CLI command that the TUI will run.
#[derive(Debug)]
pub struct Invocation {
    /// The parsed command.
    pub command: Commands,
    /// Where it runs.
    pub route: Route,
    /// `--ai` was given: use markdown output for this command.
    pub ai: bool,
    /// The argument vector that clap parsed, after context injection.
    /// Recorded in the event log.
    pub argv: Vec<String>,
}

/// What the TUI must do with one line of input.
#[derive(Debug)]
pub enum Plan {
    /// Nothing to run: the line was empty, a session command, or an error
    /// that is already written to the output.
    Done,
    /// Leave the TUI.
    Exit,
    /// Run a CLI command. Boxed: `Commands` is large.
    Run(Box<Invocation>),
}

/// Plans one line of input.
///
/// Session commands run here and write to `out`. Parse errors and `--help`
/// text are written to `out` too.
///
/// # Errors
///
/// Returns the output's error when a message cannot be written.
pub fn plan(
    line: &str,
    ctx: &mut SessionContext,
    config: &Config,
    out: &Output,
) -> io::Result<Plan> {
    let Some(words) = shlex::split(line) else {
        errln!(out, "Unbalanced quotes in: {}", line)?;
        return Ok(Plan::Done);
    };
    let Some(first) = words.first() else {
        return Ok(Plan::Done);
    };
    let rest: Vec<&str> = words[1..].iter().map(String::as_str).collect();

    match first.to_lowercase().as_str() {
        "exit" | "quit" | "q" | ".exit" | ".quit" => return Ok(Plan::Exit),
        "help" | "?" | ".help" => {
            write_help(out)?;
            return Ok(Plan::Done);
        }
        "ctx" | "context" | ".ctx" | ".context" => {
            out!(out, "{}", ctx)?;
            return Ok(Plan::Done);
        }
        "clear" | "reset" | ".clear" | ".reset" => {
            *ctx = SessionContext::default();
            outln!(out, "Context reset to defaults.")?;
            return Ok(Plan::Done);
        }
        "chain" => {
            set_chain(ctx, rest.first().copied(), out)?;
            return Ok(Plan::Done);
        }
        "format" => {
            set_format(ctx, rest.first().copied(), out)?;
            return Ok(Plan::Done);
        }
        "+tokens" | "showtokens" => {
            ctx.include_tokens = !ctx.include_tokens;
            outln!(out, "Include tokens: {}", on_off(ctx.include_tokens))?;
            return Ok(Plan::Done);
        }
        "+txs" | "showtxs" | "txs" => {
            ctx.include_txs = !ctx.include_txs;
            outln!(out, "Include transactions: {}", on_off(ctx.include_txs))?;
            return Ok(Plan::Done);
        }
        "trace" => {
            ctx.trace = !ctx.trace;
            outln!(out, "Trace: {}", on_off(ctx.trace))?;
            return Ok(Plan::Done);
        }
        "decode" => {
            ctx.decode = !ctx.decode;
            outln!(out, "Decode: {}", on_off(ctx.decode))?;
            return Ok(Plan::Done);
        }
        "limit" => {
            set_limit(ctx, rest.first().copied(), out)?;
            return Ok(Plan::Done);
        }
        "tokens" | "aliases" => {
            tokens_command(&rest, &mut TokenAliases::load(), &TokenAliases::save, out)?;
            return Ok(Plan::Done);
        }
        _ => {}
    }

    parse_cli(words, ctx, config, out)
}

/// Parses a CLI command line with the clap definition, after it adds the
/// session context.
fn parse_cli(
    words: Vec<String>,
    ctx: &mut SessionContext,
    config: &Config,
    out: &Output,
) -> io::Result<Plan> {
    let mut argv = vec!["scope".to_string()];
    argv.extend(inject_context(words, ctx, config));

    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        // `address` / `tx` with no target reuse the last one, as in the REPL.
        Err(e) if e.kind() == ErrorKind::MissingRequiredArgument => match last_target(&argv, ctx) {
            Some(target) => {
                argv.push(target);
                match Cli::try_parse_from(&argv) {
                    Ok(cli) => cli,
                    Err(e) => return write_clap_error(&e, out),
                }
            }
            None => return write_clap_error(&e, out),
        },
        Err(e) => return write_clap_error(&e, out),
    };

    if let Some(path) = &cli.config {
        errln!(
            out,
            "--config {} has no effect inside the TUI. Restart with `scope --config {} tui`.",
            path.display(),
            path.display()
        )?;
        return Ok(Plan::Done);
    }

    let route = route(&cli.command);
    if let Route::Refuse(reason) = route {
        errln!(out, "Not available here: {}.", reason)?;
        return Ok(Plan::Done);
    }

    remember_target(&cli.command, ctx);
    Ok(Plan::Run(Box::new(Invocation {
        command: cli.command,
        route,
        ai: cli.ai,
        argv,
    })))
}

/// Writes a clap error. `--help` and `--version` go to the data channel.
fn write_clap_error(e: &clap::Error, out: &Output) -> io::Result<Plan> {
    let text = e.render().ansi().to_string();
    match e.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => out!(out, "{}", text)?,
        _ => err!(out, "{}", text)?,
    }
    Ok(Plan::Done)
}

/// One session-context value that becomes a CLI flag.
struct ContextFlag {
    /// The long flag name.
    long: &'static str,
    /// The value, or `None` for a boolean flag.
    value: Option<String>,
    /// Commands the flag applies to. `None` means every command that has it.
    only: Option<&'static [&'static str]>,
}

/// Adds the session context as flags, for the flags the user did not type.
///
/// A flag is added only when the target command defines it and accepts the
/// value. The command is found by walking the clap subcommand tree, so
/// aliases (`addr`, `token`) work.
fn inject_context(mut words: Vec<String>, ctx: &SessionContext, config: &Config) -> Vec<String> {
    let mut flags = Vec::new();
    if !ctx.is_auto_chain() {
        flags.push(ContextFlag {
            long: "chain",
            value: Some(ctx.chain.clone()),
            only: None,
        });
    }
    // Only a format the user chose differs from the configured default.
    if ctx.format != config.output.format
        && let Some(v) = ctx.format.to_possible_value()
    {
        flags.push(ContextFlag {
            long: "format",
            value: Some(v.get_name().to_string()),
            only: None,
        });
    }
    for (on, long) in [
        (ctx.include_tokens, "include-tokens"),
        (ctx.include_txs, "include-txs"),
        (ctx.trace, "trace"),
        (ctx.decode, "decode"),
    ] {
        if on {
            flags.push(ContextFlag {
                long,
                value: None,
                only: None,
            });
        }
    }
    // Other commands also have --limit, with other meanings and defaults.
    if ctx.limit != SessionContext::default().limit {
        flags.push(ContextFlag {
            long: "limit",
            value: Some(ctx.limit.to_string()),
            only: Some(&["address"]),
        });
    }
    if flags.is_empty() {
        return words;
    }

    let root = Cli::command();
    let mut cmd = &root;
    for w in &words {
        if w.starts_with('-') {
            continue;
        }
        match cmd.find_subcommand(w) {
            Some(sub) => cmd = sub,
            None => break,
        }
    }
    if std::ptr::eq(cmd, &root) {
        return words;
    }

    for flag in flags {
        if flag
            .only
            .is_some_and(|names| !names.contains(&cmd.get_name()))
        {
            continue;
        }
        let Some(arg) = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some(flag.long))
        else {
            continue;
        };
        let typed = words.iter().any(|w| {
            w == &format!("--{}", flag.long)
                || w.starts_with(&format!("--{}=", flag.long))
                || arg
                    .get_short()
                    .is_some_and(|s| w.starts_with(&format!("-{}", s)) && !w.starts_with("--"))
        });
        if typed {
            continue;
        }
        if let Some(value) = &flag.value {
            let values = arg.get_possible_values();
            if !values.is_empty() && !values.iter().any(|p| p.matches(value, true)) {
                continue;
            }
            words.push(format!("--{}", flag.long));
            words.push(value.clone());
        } else {
            words.push(format!("--{}", flag.long));
        }
    }
    words
}

/// The last address or transaction to reuse when `address` / `tx` has no
/// target.
fn last_target(argv: &[String], ctx: &SessionContext) -> Option<String> {
    let root = Cli::command();
    let name = argv.get(1)?;
    let sub = root.find_subcommand(name)?;
    match sub.get_name() {
        "address" => ctx.last_address.clone(),
        "tx" => ctx.last_tx.clone(),
        _ => None,
    }
}

/// Records the target of `address` / `tx` for later reuse.
fn remember_target(command: &Commands, ctx: &mut SessionContext) {
    match command {
        Commands::Address(args) => ctx.last_address = Some(args.address.clone()),
        Commands::Tx(args) => ctx.last_tx = Some(args.hash.clone()),
        _ => {}
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// Chains the `chain` session command accepts.
const CHAINS: &[&str] = &[
    "ethereum", "polygon", "arbitrum", "optimism", "base", "bsc", "aegis", "solana", "tron",
];

fn set_chain(ctx: &mut SessionContext, arg: Option<&str>, out: &Output) -> io::Result<()> {
    match arg.map(str::to_lowercase) {
        None if ctx.is_auto_chain() => outln!(out, "Current chain: auto (inferred from input)"),
        None => outln!(out, "Current chain: {} (pinned)", ctx.chain),
        Some(c) if c == "auto" => {
            ctx.chain = "auto".into();
            outln!(out, "Chain set to auto — will infer from each input")
        }
        Some(c) if CHAINS.contains(&c.as_str()) => {
            outln!(out, "Chain pinned to: {}", c)?;
            ctx.chain = c;
            Ok(())
        }
        Some(c) => errln!(
            out,
            "Unknown chain: {}. Valid chains: auto, {}",
            c,
            CHAINS.join(", ")
        ),
    }
}

fn set_format(ctx: &mut SessionContext, arg: Option<&str>, out: &Output) -> io::Result<()> {
    let Some(arg) = arg else {
        return outln!(out, "Current format: {:?}", ctx.format);
    };
    match OutputFormat::from_str(arg, true) {
        Ok(f) => {
            ctx.format = f;
            outln!(out, "Format set to: {}", arg.to_lowercase())
        }
        Err(_) => {
            let names: Vec<String> = OutputFormat::value_variants()
                .iter()
                .filter_map(|v| v.to_possible_value().map(|p| p.get_name().to_string()))
                .collect();
            errln!(
                out,
                "Unknown format: {}. Valid formats: {}",
                arg,
                names.join(", ")
            )
        }
    }
}

fn set_limit(ctx: &mut SessionContext, arg: Option<&str>, out: &Output) -> io::Result<()> {
    match arg.map(str::parse::<u32>) {
        None => outln!(out, "Current limit: {}", ctx.limit),
        Some(Ok(n)) if n > 0 => {
            ctx.limit = n;
            outln!(out, "Limit set to: {}", n)
        }
        Some(_) => errln!(
            out,
            "Invalid limit: {}. Must be a positive integer.",
            arg.unwrap_or_default()
        ),
    }
}

/// Saves the token aliases. Injected so tests never write the user's file.
type SaveAliases<'a> = dyn Fn(&TokenAliases) -> scope::error::Result<()> + 'a;

/// The `tokens` session command: list, add and remove saved token aliases.
fn tokens_command(
    args: &[&str],
    aliases: &mut TokenAliases,
    save: &SaveAliases<'_>,
    out: &Output,
) -> io::Result<()> {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("list") | Some("ls") => {
            let tokens = aliases.list();
            if tokens.is_empty() {
                outln!(
                    out,
                    "{}",
                    scope::display::terminal::info_row("No saved token aliases.")
                )?;
                return outln!(
                    out,
                    "{}",
                    scope::display::terminal::info_row(
                        "Use 'crawl <token_name> --save' to save a token alias."
                    )
                );
            }
            write_token_table(out, "Saved Token Aliases", &tokens)
        }
        Some("recent") => {
            let recent = aliases.recent();
            if recent.is_empty() {
                return outln!(
                    out,
                    "{}",
                    scope::display::terminal::info_row("No recently used tokens.")
                );
            }
            write_token_table(out, "Recently Used Tokens", recent)
        }
        Some("remove") | Some("rm") | Some("delete") => {
            let Some(symbol) = args.get(1) else {
                return errln!(out, "Usage: tokens remove <symbol> [--chain <chain>]");
            };
            let chain = match (args.get(2), args.get(3)) {
                (Some(&"--chain"), Some(c)) => Some(*c),
                _ => None,
            };
            aliases.remove(symbol, chain);
            match save(aliases) {
                Ok(()) => outln!(out, "Removed alias: {}", symbol),
                Err(e) => errln!(out, "Failed to save the token aliases: {}", e),
            }
        }
        Some("add") => {
            let (Some(symbol), Some(chain), Some(address)) =
                (args.get(1), args.get(2), args.get(3))
            else {
                return errln!(out, "Usage: tokens add <symbol> <chain> <address> [name]");
            };
            let name = if args.len() > 4 {
                args[4..].join(" ")
            } else {
                symbol.to_string()
            };
            aliases.add(symbol, chain, address, &name);
            match save(aliases) {
                Ok(()) => outln!(out, "Added alias: {} -> {} on {}", symbol, address, chain),
                Err(e) => errln!(out, "Failed to save the token aliases: {}", e),
            }
        }
        Some(other) => {
            errln!(out, "Unknown tokens subcommand: {}", other)?;
            errln!(out, "Available: list, recent, add, remove")
        }
    }
}

fn write_token_table<T>(out: &Output, title: &str, tokens: &[T]) -> io::Result<()>
where
    T: std::borrow::Borrow<scope::tokens::TokenInfo>,
{
    use scope::display::terminal as t;
    let col = |label, width| t::Col {
        label,
        width,
        align: '<',
    };
    let cols = &[
        col("Symbol", 10),
        col("Chain", 12),
        col("Name", 20),
        col("Address", 42),
    ];
    outln!(out, "{}", t::section_header(title))?;
    outln!(out, "{}", t::table_header(cols))?;
    for token in tokens.iter().map(std::borrow::Borrow::borrow) {
        outln!(
            out,
            "{}",
            t::table_row(
                cols,
                &[&token.symbol, &token.chain, &token.name, &token.address]
            )
        )?;
    }
    outln!(out, "{}", t::section_footer())
}

/// Writes the TUI help: the session commands, then every CLI command.
fn write_help(out: &Output) -> io::Result<()> {
    outln!(out, "Session commands:")?;
    for (cmd, what) in [
        ("chain [name|auto]", "Show, pin or unpin the chain"),
        (
            "format [table|json|csv|markdown]",
            "Show or set the output format",
        ),
        (
            "+tokens, +txs",
            "Toggle token balances / transactions for address",
        ),
        ("trace, decode", "Toggle trace / decode for tx"),
        ("limit [n]", "Show or set the transaction limit for address"),
        (
            "tokens [list|recent|add|remove]",
            "Manage saved token aliases",
        ),
        ("ctx", "Show the session context"),
        ("clear", "Reset the session context"),
        ("help", "Show this help"),
        ("exit", "Leave the TUI (also Ctrl-D)"),
    ] {
        outln!(out, "  {:<34} {}", cmd, what)?;
    }
    outln!(out)?;
    outln!(
        out,
        "CLI commands (same flags as on the command line; add --help for details):"
    )?;
    for sub in Cli::command().get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        outln!(out, "  {:<34} {}", sub.get_name(), about)?;
    }
    outln!(out)?;
    outln!(
        out,
        "Keys: Enter run · ↑/↓ history · PgUp/PgDn scroll · Esc or Ctrl-C cancel a running command"
    )
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const ADDR: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";
    const TX: &str = "0xabc123def456789012345678901234567890123456789012345678901234abcd";

    fn plan_line(line: &str, ctx: &mut SessionContext) -> (Plan, crate::cli::output::Captured) {
        let (o, cap) = Output::capture();
        let p = plan(line, ctx, &Config::default(), &o).unwrap();
        (p, cap)
    }

    fn run_of(p: Plan) -> Invocation {
        match p {
            Plan::Run(inv) => *inv,
            other => panic!("expected Plan::Run, got {:?}", other),
        }
    }

    // ---- Parity (issue #47, requirement 5) ----

    #[test]
    fn test_every_cli_command_parses_and_has_a_tui_route() {
        // Every CLI subcommand must reach a handler from the TUI, except
        // the two that cannot run inside it. A new command fails here (and
        // in `route`, at compile time) until it has a TUI path.
        let refused = ["interactive", "web"];
        for sub in Cli::command().get_subcommands() {
            let name = sub.get_name();
            let (o, cap) = Output::capture();
            let mut ctx = SessionContext::default();
            // `--help` parses for every command without its required args.
            let p = plan(
                &format!("{} --help", name),
                &mut ctx,
                &Config::default(),
                &o,
            )
            .unwrap();
            assert!(matches!(p, Plan::Done), "{}", name);
            assert!(cap.out().contains("Usage"), "no help for {}", name);

            let mut subcmds = sub.get_subcommands().map(|s| s.get_name()).peekable();
            let leaf_names: Vec<String> = if subcmds.peek().is_some() {
                subcmds.map(|s| format!("{} {}", name, s)).collect()
            } else {
                vec![name.to_string()]
            };
            for leaf in leaf_names {
                let line = minimal_line(&leaf);
                let (o, cap) = Output::capture();
                let p = plan(&line, &mut ctx, &Config::default(), &o).unwrap();
                if refused.contains(&name) {
                    assert!(matches!(p, Plan::Done), "{} must be refused", leaf);
                    assert!(cap.err().contains("Not available here"), "{}", cap.err());
                } else {
                    // A refused route never becomes Plan::Run.
                    assert!(
                        matches!(p, Plan::Run(_)),
                        "`{}` did not parse: {}",
                        line,
                        cap.err()
                    );
                }
            }
        }
    }

    /// The smallest valid command line for a leaf command.
    fn minimal_line(leaf: &str) -> String {
        match leaf {
            "address" | "insights" | "contract" => format!("{} {}", leaf, ADDR),
            "tx" => format!("tx {}", TX),
            "crawl" | "token-health" | "monitor" => format!("{} USDC", leaf),
            "compliance risk" | "compliance trace" | "compliance analyze" => {
                format!("{} {}", leaf, ADDR)
            }
            "compliance compliance-report" => {
                format!("{} {} --output r.md --jurisdiction us", leaf, ADDR)
            }
            "address-book add" | "address-book remove" => format!("{} {}", leaf, ADDR),
            "venues validate" => format!("{} venue.yaml", leaf),
            "market summary" | "market ohlc" | "market trades" => format!("{} USDC", leaf),
            "report batch" => format!("{} --addresses {} --output r.md", leaf, ADDR),
            "export" => format!("export --address {} --output out.json", ADDR),
            "completions" => "completions bash".into(),
            other => other.to_string(),
        }
    }

    // ---- Session commands ----

    #[test]
    fn test_exit_words() {
        for w in ["exit", "quit", "q", ".exit", ".quit", "EXIT"] {
            assert!(matches!(
                plan_line(w, &mut SessionContext::default()).0,
                Plan::Exit
            ));
        }
    }

    #[test]
    fn test_empty_line_does_nothing() {
        let (p, cap) = plan_line("   ", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert_eq!(cap.out() + &cap.err(), "");
    }

    #[test]
    fn test_chain_pin_and_unpin() {
        let mut ctx = SessionContext::default();
        plan_line("chain solana", &mut ctx);
        assert_eq!(ctx.chain, "solana");
        plan_line("chain auto", &mut ctx);
        assert!(ctx.is_auto_chain());
        let (_, cap) = plan_line("chain notachain", &mut ctx);
        assert!(ctx.is_auto_chain());
        assert!(cap.err().contains("Unknown chain"));
    }

    #[test]
    fn test_format_set_and_reject() {
        let mut ctx = SessionContext::default();
        plan_line("format json", &mut ctx);
        assert_eq!(ctx.format, OutputFormat::Json);
        let (_, cap) = plan_line("format xml", &mut ctx);
        assert_eq!(ctx.format, OutputFormat::Json);
        assert!(cap.err().contains("Unknown format"));
    }

    #[test]
    fn test_toggles_limit_and_clear() {
        let mut ctx = SessionContext::default();
        for w in ["+tokens", "+txs", "trace", "decode"] {
            plan_line(w, &mut ctx);
        }
        assert!(ctx.include_tokens && ctx.include_txs && ctx.trace && ctx.decode);
        plan_line("limit 25", &mut ctx);
        assert_eq!(ctx.limit, 25);
        let (_, cap) = plan_line("limit 0", &mut ctx);
        assert_eq!(ctx.limit, 25);
        assert!(cap.err().contains("Invalid limit"));
        plan_line("clear", &mut ctx);
        assert_eq!(ctx.limit, SessionContext::default().limit);
        assert!(!ctx.include_tokens);
    }

    #[test]
    fn test_ctx_prints_context() {
        let mut ctx = SessionContext::default();
        plan_line("chain base", &mut ctx);
        let (_, cap) = plan_line("ctx", &mut ctx);
        assert!(cap.out().contains("base (pinned)"));
    }

    #[test]
    fn test_help_lists_session_and_cli_commands() {
        let (_, cap) = plan_line("help", &mut SessionContext::default());
        for word in [
            "chain [name|auto]",
            "address",
            "market",
            "compliance",
            "venues",
        ] {
            assert!(cap.out().contains(word), "help is missing {}", word);
        }
    }

    // ---- tokens ----

    /// Runs `tokens …` against an in-memory store; returns output and how
    /// many times it saved.
    fn tokens(line: &str, aliases: &mut TokenAliases, fail_save: bool) -> (String, String, usize) {
        let saves = std::cell::Cell::new(0);
        let save = |_: &TokenAliases| {
            saves.set(saves.get() + 1);
            if fail_save {
                Err(scope::error::ScopeError::Io("disk full".into()))
            } else {
                Ok(())
            }
        };
        let (o, cap) = Output::capture();
        let args: Vec<&str> = line.split_whitespace().collect();
        tokens_command(&args, aliases, &save, &o).unwrap();
        (cap.out(), cap.err(), saves.get())
    }

    #[test]
    fn test_tokens_add_list_recent_remove() {
        let mut a = TokenAliases::default();
        let (out, _, saves) = tokens(
            &format!("add USDN ethereum {} Nova USD", ADDR),
            &mut a,
            false,
        );
        assert!(out.contains("Added alias: USDN"));
        assert_eq!(saves, 1);
        assert_eq!(a.get("USDN", None).unwrap().name, "Nova USD");

        for cmd in ["", "list", "ls"] {
            let (out, _, _) = tokens(cmd, &mut a, false);
            assert!(
                out.contains("Saved Token Aliases") && out.contains("USDN"),
                "{}",
                cmd
            );
        }
        let (out, _, _) = tokens("recent", &mut a, false);
        assert!(out.contains("No recently used tokens") || out.contains("USDN"));

        let (out, _, saves) = tokens("rm USDN --chain ethereum", &mut a, false);
        assert!(out.contains("Removed alias: USDN"));
        assert_eq!(saves, 1);
        assert!(a.get("USDN", None).is_none());
        let (out, _, _) = tokens("list", &mut a, false);
        assert!(out.contains("No saved token aliases"));
    }

    #[test]
    fn test_tokens_usage_errors_do_not_save() {
        let mut a = TokenAliases::default();
        for line in ["add USDN ethereum", "remove", "frobnicate"] {
            let (out, err, saves) = tokens(line, &mut a, false);
            assert_eq!(out, "", "{}", line);
            assert!(!err.is_empty(), "{}", line);
            assert_eq!(saves, 0, "{}", line);
        }
    }

    #[test]
    fn test_tokens_save_failure_is_reported() {
        let mut a = TokenAliases::default();
        let (out, err, _) = tokens(&format!("add USDN ethereum {}", ADDR), &mut a, true);
        assert!(!out.contains("Added"));
        assert!(
            err.contains("Failed to save the token aliases: "),
            "{}",
            err
        );
        assert!(err.contains("disk full"));
    }

    // ---- CLI parsing and context injection ----

    #[test]
    fn test_cli_command_parses_with_aliases() {
        let inv = run_of(plan_line(&format!("addr {}", ADDR), &mut SessionContext::default()).0);
        assert!(matches!(inv.command, Commands::Address(_)));
        assert_eq!(inv.route, Route::Pane);
    }

    #[test]
    fn test_pinned_chain_is_injected_but_typed_flag_wins() {
        let mut ctx = SessionContext {
            chain: "polygon".into(),
            ..Default::default()
        };
        let inv = run_of(plan_line(&format!("address {}", ADDR), &mut ctx).0);
        let Commands::Address(a) = inv.command else {
            panic!()
        };
        assert_eq!(a.chain, "polygon");

        let inv = run_of(plan_line(&format!("address {} -c base", ADDR), &mut ctx).0);
        let Commands::Address(a) = inv.command else {
            panic!()
        };
        assert_eq!(a.chain, "base");
    }

    #[test]
    fn test_auto_chain_injects_nothing() {
        // The CLI infers the chain from the address; the TUI must not
        // override that with a default.
        let inv = run_of(plan_line(&format!("address {}", ADDR), &mut SessionContext::default()).0);
        assert_eq!(inv.argv, vec!["scope", "address", ADDR]);
    }

    #[test]
    fn test_context_flags_apply_only_where_defined() {
        let mut ctx = SessionContext {
            include_tokens: true,
            trace: true,
            limit: 7,
            ..Default::default()
        };
        let inv = run_of(plan_line(&format!("address {}", ADDR), &mut ctx).0);
        let Commands::Address(a) = inv.command else {
            panic!()
        };
        assert!(a.include_tokens);
        assert_eq!(a.limit, 7);
        // `address` has no --trace; it must not be injected there.
        assert!(!inv.argv.contains(&"--trace".to_string()));

        let inv = run_of(plan_line(&format!("tx {}", TX), &mut ctx).0);
        let Commands::Tx(t) = inv.command else {
            panic!()
        };
        assert!(t.trace);
        assert!(!inv.argv.contains(&"--limit".to_string()));
    }

    #[test]
    fn test_format_injected_only_where_value_is_accepted() {
        let mut ctx = SessionContext {
            format: OutputFormat::Csv,
            ..Default::default()
        };
        let inv = run_of(plan_line(&format!("address {}", ADDR), &mut ctx).0);
        let Commands::Address(a) = inv.command else {
            panic!()
        };
        assert_eq!(a.format, Some(OutputFormat::Csv));
        // market summary accepts text|json only: csv must not break it.
        let inv = run_of(plan_line("market summary USDC", &mut ctx).0);
        assert!(!inv.argv.contains(&"csv".to_string()));
    }

    #[test]
    fn test_last_address_and_tx_are_reused() {
        let mut ctx = SessionContext::default();
        run_of(plan_line(&format!("address {}", ADDR), &mut ctx).0);
        let inv = run_of(plan_line("address", &mut ctx).0);
        let Commands::Address(a) = inv.command else {
            panic!()
        };
        assert_eq!(a.address, ADDR);

        run_of(plan_line(&format!("tx {}", TX), &mut ctx).0);
        let inv = run_of(plan_line("tx", &mut ctx).0);
        let Commands::Tx(t) = inv.command else {
            panic!()
        };
        assert_eq!(t.hash, TX);
    }

    #[test]
    fn test_missing_target_without_history_is_a_clap_error() {
        let (p, cap) = plan_line("address", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("required"), "{}", cap.err());
    }

    #[test]
    fn test_unknown_command_suggests_like_the_cli() {
        let (p, cap) = plan_line("adress 0x1", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("address"), "{}", cap.err());
    }

    #[test]
    fn test_quoted_arguments_and_unbalanced_quotes() {
        let inv = run_of(
            plan_line(
                &format!("address-book add {} --label \"cold wallet\"", ADDR),
                &mut SessionContext::default(),
            )
            .0,
        );
        assert!(inv.argv.contains(&"cold wallet".to_string()));
        let (p, cap) = plan_line("address \"0x1", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("Unbalanced quotes"));
    }

    #[test]
    fn test_ai_flag_and_config_flag() {
        let inv = run_of(
            plan_line(
                &format!("--ai address {}", ADDR),
                &mut SessionContext::default(),
            )
            .0,
        );
        assert!(inv.ai);
        let (p, cap) = plan_line(
            &format!("--config x.yaml address {}", ADDR),
            &mut SessionContext::default(),
        );
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("no effect inside the TUI"));
    }

    // ---- Routing ----

    #[test]
    fn test_routes_for_prompting_commands() {
        let mut ctx = SessionContext::default();
        assert_eq!(
            run_of(plan_line("crawl USDC", &mut ctx).0).route,
            Route::Suspend
        );
        assert_eq!(
            run_of(plan_line("crawl USDC --yes", &mut ctx).0).route,
            Route::Pane
        );
        assert_eq!(
            run_of(plan_line(&format!("crawl {}", ADDR), &mut ctx).0).route,
            Route::Pane
        );
        assert_eq!(run_of(plan_line("setup", &mut ctx).0).route, Route::Suspend);
        assert_eq!(
            run_of(plan_line("setup --status", &mut ctx).0).route,
            Route::Pane
        );
        assert_eq!(
            run_of(plan_line("monitor USDC", &mut ctx).0).route,
            Route::Suspend
        );
    }

    #[test]
    fn test_nested_tui_and_web_are_refused() {
        for line in ["interactive", "tui", "shell", "web"] {
            let (p, cap) = plan_line(line, &mut SessionContext::default());
            assert!(matches!(p, Plan::Done), "{}", line);
            assert!(cap.err().contains("Not available here"), "{}", line);
        }
    }
}
