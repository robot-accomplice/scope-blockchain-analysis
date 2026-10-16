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
use crate::{errln, out, outln};
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

/// The commands the TUI does not run, by name, and what to do instead.
/// `route` and the TUI help both read this list (#66).
pub(crate) const TUI_REFUSED: &[(&str, &str)] = &[
    ("interactive", "you are already in the TUI"),
    (
        "web",
        "the web server runs in the foreground; start it from a shell with `scope web`",
    ),
    (
        "completions",
        "it prints a script for your shell; run `scope completions <shell>` in a shell",
    ),
];

/// The refusal reason for the command `name`, from [`TUI_REFUSED`].
fn refused(name: &str) -> Route {
    let reason = TUI_REFUSED
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("it does not run in the TUI", |(_, why)| why);
    Route::Refuse(reason)
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
        | Commands::Report(_) => Route::Pane,
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
        Commands::Interactive(_) => refused("interactive"),
        Commands::Web(_) => refused("web"),
        Commands::Completions(_) => refused("completions"),
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
    let Some(words) = split_line(line) else {
        errln!(out, "Unbalanced quotes in: {}", line)?;
        return Ok(Plan::Done);
    };
    let Some(first) = words.first() else {
        return Ok(Plan::Done);
    };
    let rest: Vec<&str> = words[1..].iter().map(String::as_str).collect();

    match session_command_name(first).as_str() {
        "exit" => return Ok(Plan::Exit),
        // Hidden debug command for the manual crash-log check. It is not in
        // SESSION_COMMANDS, so help, completion and the palette skip it.
        #[cfg(debug_assertions)]
        "__panic" => panic!("__panic: manual crash-log check"),
        "help" => {
            if rest.is_empty() {
                write_help(out)?;
                return Ok(Plan::Done);
            }
            // A session command has no clap help: show its own line.
            let name = session_command_name(&words[1]);
            if let Some((_, usage, what)) = SESSION_COMMANDS.iter().find(|(n, _, _)| *n == name) {
                outln!(out, "  {:<34} {}", usage, what)?;
                return Ok(Plan::Done);
            }
            // `help market summary` is `market summary --help`: the same
            // text as `scope help market summary`.
            let mut words: Vec<String> = words[1..].to_vec();
            words.push("--help".to_string());
            return parse_cli(words, ctx, config, out);
        }
        "ctx" => {
            out!(out, "{}", ctx)?;
            return Ok(Plan::Done);
        }
        "reset" => {
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
        "+tokens" => {
            ctx.include_tokens = !ctx.include_tokens;
            outln!(out, "Include tokens: {}", on_off(ctx.include_tokens))?;
            return Ok(Plan::Done);
        }
        "+txs" => {
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
        "tokens" => {
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
                    Err(e) => return write_clap_error(&e, &argv, out),
                }
            }
            None => return write_clap_error(&e, &argv, out),
        },
        Err(e) => return write_clap_error(&e, &argv, out),
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
///
/// For a usage error, clap's `--help` hint is replaced by the TUI form,
/// `run 'help <command>' for its options`.
fn write_clap_error(e: &clap::Error, argv: &[String], out: &Output) -> io::Result<Plan> {
    let text = e.render().ansi().to_string();
    match e.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => out!(out, "{}", text)?,
        _ => {
            for line in text.lines().filter(|l| !l.contains(CLAP_HELP_HINT)) {
                errln!(out, "{}", line)?;
            }
            errln!(out, "{}", tui_help_hint(argv))?;
        }
    }
    Ok(Plan::Done)
}

/// The start of clap's own hint line, which names the CLI `--help` flag.
const CLAP_HELP_HINT: &str = "For more information";

/// The TUI next step for a usage error: `help` for the command the user
/// typed. The command is the longest run of subcommand words in `argv`.
fn tui_help_hint(argv: &[String]) -> String {
    let root = Cli::command();
    let mut cmd = &root;
    let mut words = Vec::new();
    for w in argv.iter().skip(1).filter(|w| !w.starts_with('-')) {
        match cmd.find_subcommand(w) {
            Some(sub) => {
                words.push(w.as_str());
                cmd = sub;
            }
            None => break,
        }
    }
    if words.is_empty() {
        "run 'help <command>' for its options".to_string()
    } else {
        format!("run 'help {}' for its options", words.join(" "))
    }
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

fn set_chain(ctx: &mut SessionContext, arg: Option<&str>, out: &Output) -> io::Result<()> {
    match arg.map(str::to_lowercase) {
        None if ctx.is_auto_chain() => outln!(out, "Current chain: auto (inferred from input)"),
        None => outln!(out, "Current chain: {} (pinned)", ctx.chain),
        Some(c) if c == "auto" => {
            ctx.chain = "auto".into();
            outln!(out, "Chain set to auto — will infer from each input")
        }
        Some(c) if scope::chains::registry::client_chain_ids().contains(&c.as_str()) => {
            outln!(out, "Chain pinned to: {}", c)?;
            ctx.chain = c;
            Ok(())
        }
        Some(c) => errln!(
            out,
            "Unknown chain: {}. Valid chains: auto, {}",
            c,
            scope::chains::registry::client_chain_list()
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

/// The session commands: (word, usage, description). Help, completion and
/// the palette read this list.
pub(crate) const SESSION_COMMANDS: &[(&str, &str, &str)] = &[
    ("chain", "chain [name|auto]", "Show, pin or unpin the chain"),
    (
        "format",
        "format [table|json|csv|markdown]",
        "Show or set the output format",
    ),
    ("+tokens", "+tokens", "Toggle token balances for address"),
    ("+txs", "+txs", "Toggle transactions for address"),
    ("trace", "trace", "Toggle trace for tx"),
    ("decode", "decode", "Toggle decode for tx"),
    (
        "limit",
        "limit [n]",
        "Show or set the transaction limit for address",
    ),
    (
        "tokens",
        "tokens [list|recent|add|remove]",
        "Manage saved token aliases",
    ),
    (
        "save",
        "save <file> [--force]",
        "Write the last output to a file",
    ),
    ("ctx", "ctx", "Show the session context"),
    ("clear", "clear", "Clear the output pane"),
    ("reset", "reset", "Reset the session context"),
    ("help", "help", "Show this help"),
    ("exit", "exit", "Leave the TUI (also Ctrl-D)"),
];

/// Other words for the session commands: `(alias, SESSION_COMMANDS name)`.
/// The one list: dispatch, help and the exit check all read it.
const SESSION_ALIASES: &[(&str, &str)] = &[
    ("quit", "exit"),
    ("q", "exit"),
    (".exit", "exit"),
    (".quit", "exit"),
    ("context", "ctx"),
    (".ctx", "ctx"),
    (".context", "ctx"),
    (".clear", "clear"),
    (".reset", "reset"),
    ("?", "help"),
    (".help", "help"),
    ("showtokens", "+tokens"),
    ("showtxs", "+txs"),
    ("txs", "+txs"),
    ("aliases", "tokens"),
];

/// The `SESSION_COMMANDS` name for a typed word (any case), or the word
/// itself when it is not an alias.
/// Splits a line of input into words; `None` when a quote is not closed.
///
/// On unix the rules are the shell's (shlex). On Windows a backslash is
/// a path separator, so `save C:\Users\me\out.json` keeps its path; see
/// [`split_backslash_literal`].
pub(crate) fn split_line(line: &str) -> Option<Vec<String>> {
    if cfg!(windows) {
        split_backslash_literal(line)
    } else {
        shlex::split(line)
    }
}

/// Splits `line` at whitespace outside quotes. Single and double quotes
/// group words and are removed; a backslash is an ordinary character, so
/// a quote cannot be escaped (quote it with the other kind). `None` when a
/// quote is not closed.
fn split_backslash_literal(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.get_or_insert_default().push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            None if c.is_whitespace() => words.extend(word.take()),
            None => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    words.extend(word);
    Some(words)
}

pub(crate) fn session_command_name(word: &str) -> String {
    let word = word.to_lowercase();
    SESSION_ALIASES
        .iter()
        .find(|(alias, _)| *alias == word)
        .map_or(word, |(_, name)| (*name).to_string())
}

/// Writes the TUI help: the session commands, then every CLI command.
fn write_help(out: &Output) -> io::Result<()> {
    outln!(out, "Session commands:")?;
    for (name, usage, what) in SESSION_COMMANDS {
        // Name the other words for the command too (#66: `quit`, `q`).
        let aliases: Vec<&str> = SESSION_ALIASES
            .iter()
            .filter(|(_, n)| n == name)
            .map(|(alias, _)| *alias)
            .filter(|alias| !alias.starts_with('.'))
            .collect();
        if aliases.is_empty() {
            outln!(out, "  {:<34} {}", usage, what)?;
        } else {
            outln!(
                out,
                "  {:<34} {} (also: {})",
                usage,
                what,
                aliases.join(", ")
            )?;
        }
    }
    outln!(out)?;
    outln!(
        out,
        "CLI commands (same flags as on the command line; add --help for details):"
    )?;
    for sub in Cli::command().get_subcommands() {
        let refused = TUI_REFUSED.iter().any(|(n, _)| *n == sub.get_name());
        if sub.is_hide_set() || refused {
            continue;
        }
        let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        outln!(out, "  {:<34} {}", sub.get_name(), about)?;
    }
    outln!(out)?;
    // Key names in the key color, what they do dim. The ANSI becomes styles
    // in the pane; the copy and the log strip it.
    let painter = scope::display::theme::Painter::new(true);
    let key = |k: &str| painter.paint(scope::display::theme::Role::Key, k);
    let what = |w: &str| painter.paint(scope::display::theme::Role::Secondary, w);
    outln!(
        out,
        "Keys: {} {} · {} {} · {} {} · {} {} · {} {} · {} {}",
        key("Enter"),
        what("run"),
        key("Tab"),
        what("complete"),
        key("Ctrl-K"),
        what("palette"),
        key("↑/↓"),
        what("history"),
        key("PgUp/PgDn"),
        what("scroll"),
        format_args!("{} or {}", key("Esc"), key("Ctrl-C")),
        what("cancel"),
    )?;
    outln!(
        out,
        "      {} {} · {} or {} {} · {} {} · {} {}",
        key("Ctrl-Y"),
        what("copy last output"),
        key("Ctrl-F"),
        key("/"),
        what("search"),
        key("Ctrl-R"),
        what("history search"),
        key("Shift-drag"),
        what("selects text"),
    )?;
    outln!(
        out,
        "      {}",
        what("Copy needs a terminal that allows OSC 52; save <file> works everywhere")
    )?;
    outln!(
        out,
        "      {}",
        what(
            "A command with --every shows one live view: the latest report and its trends. After it ends, any key leaves the view; save <file>.csv writes the series."
        )
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

    #[test]
    fn test_backslash_literal_split_keeps_windows_paths() {
        let split = |s: &str| split_backslash_literal(s).unwrap();
        assert_eq!(
            split(r"save C:\Users\x\f.json"),
            ["save", r"C:\Users\x\f.json"]
        );
        assert_eq!(
            split(r#"save "C:\My Files\f.json""#),
            ["save", r"C:\My Files\f.json"]
        );
        assert_eq!(split(r"a 'b c'd  e"), ["a", "b cd", "e"]);
        assert_eq!(split(r#"x "" y"#), ["x", "", "y"]);
        assert_eq!(split("   "), Vec::<String>::new());
        assert_eq!(split_backslash_literal(r#"save "C:\x"#), None);
    }

    #[test]
    fn test_split_line_uses_the_platform_rules() {
        let words = split_line(r"save C:\x\f.json").unwrap();
        if cfg!(windows) {
            assert_eq!(words, ["save", r"C:\x\f.json"]);
        } else {
            assert_eq!(words, ["save", "C:xf.json"]);
        }
    }

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
        // the ones TUI_REFUSED lists. A new command fails here (and in
        // `route`, at compile time) until it has a TUI path.
        let refused: Vec<&str> = TUI_REFUSED.iter().map(|(n, _)| *n).collect();
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

    #[test]
    fn test_clap_error_gives_tui_next_steps_not_the_cli_hint() {
        let (_, cap) = plan_line("market summary --bogus", &mut SessionContext::default());
        let text = cap.err();
        assert!(!text.contains("try '--help'"), "{text}");
        assert!(text.contains("help market summary"), "{text}");
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
        plan_line("reset", &mut ctx);
        assert_eq!(ctx.limit, SessionContext::default().limit);
        assert!(!ctx.include_tokens);
    }

    #[test]
    fn test_bare_session_commands_show_current_values() {
        let mut ctx = SessionContext::default();
        let (_, cap) = plan_line("chain", &mut ctx);
        assert!(cap.out().contains("Current chain: auto"));
        plan_line("chain tron", &mut ctx);
        let (_, cap) = plan_line("chain", &mut ctx);
        assert!(cap.out().contains("Current chain: tron (pinned)"));
        let (_, cap) = plan_line("format", &mut ctx);
        assert!(cap.out().contains("Current format: Table"));
        let (_, cap) = plan_line("limit", &mut ctx);
        assert!(cap.out().contains("Current limit: 100"));
    }

    #[test]
    fn test_tokens_session_command_is_routed_not_parsed_by_clap() {
        // `tokens` is not a CLI command; it must reach the alias manager.
        // An unknown subcommand neither reads nor writes anything to save.
        let (p, cap) = plan_line("tokens frobnicate", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("Unknown tokens subcommand: frobnicate"));
    }

    #[test]
    fn test_missing_target_on_other_commands_is_not_filled_from_context() {
        // Only address and tx reuse the last target; crawl must still fail.
        let mut ctx = SessionContext {
            last_address: Some(ADDR.into()),
            ..Default::default()
        };
        let (p, cap) = plan_line("crawl", &mut ctx);
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("required"));
    }

    #[test]
    fn test_context_is_not_injected_before_a_subcommand_is_named() {
        // `--ai` alone names no command: nothing may be appended to it.
        let mut ctx = SessionContext {
            chain: "base".into(),
            ..Default::default()
        };
        let (p, cap) = plan_line("--ai", &mut ctx);
        assert!(matches!(p, Plan::Done));
        assert!(!cap.err().contains("--chain"), "{}", cap.err());
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
        for word in ["+tokens", "address", "market", "compliance", "venues"] {
            assert!(cap.out().contains(word), "help is missing {}", word);
        }
    }

    #[test]
    fn test_help_lists_the_search_keys() {
        let (_, cap) = plan_line("help", &mut SessionContext::default());
        let help = crate::cli::ansi::strip(&cap.out());
        for keys in ["Ctrl-F or / search", "Ctrl-R history search"] {
            assert!(help.contains(keys), "help is missing {}", keys);
        }
    }

    #[test]
    fn test_help_with_a_command_shows_that_commands_help() {
        let (p, cap) = plan_line("help market summary", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.out().contains("Usage:"), "{}", cap.out());
        assert!(cap.out().contains("--venue"), "{}", cap.out());
        assert!(!cap.out().contains("Session commands"), "{}", cap.out());
    }

    #[test]
    fn test_help_with_a_session_command_shows_its_line() {
        for (typed, usage) in [
            ("help ctx", "ctx"),
            ("help tokens", "tokens [list|recent|add|remove]"),
            ("help clear", "Clear the output pane"),
            ("help reset", "Reset the session context"),
            ("help save", "save <file> [--force]"),
            ("help quit", "exit"),
        ] {
            let (p, cap) = plan_line(typed, &mut SessionContext::default());
            assert!(matches!(p, Plan::Done));
            assert!(cap.out().contains(usage), "{typed}: {}", cap.out());
            assert!(cap.err().is_empty(), "{typed}: {}", cap.err());
            assert!(!cap.out().contains("Usage:"), "{typed}");
        }
    }

    #[test]
    fn test_help_with_an_unknown_command_is_a_clap_error() {
        let (p, cap) = plan_line("help nosuchcmd", &mut SessionContext::default());
        assert!(matches!(p, Plan::Done));
        assert!(cap.err().contains("nosuchcmd"), "{}", cap.err());
        assert!(cap.out().is_empty(), "{}", cap.out());
    }

    // ---- tokens ----

    /// Runs `tokens …` against an in-memory store; returns output and how
    /// many times it saved.
    fn tokens(line: &str, aliases: &mut TokenAliases, fail_save: bool) -> (String, String, usize) {
        let saves = std::cell::Cell::new(0);
        let save = |_: &TokenAliases| {
            saves.set(saves.get() + 1);
            if fail_save {
                Err(scope::error::ScopeError::IoError(std::io::Error::other(
                    "disk full",
                )))
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

    #[test]
    fn test_tokens_recent_and_remove_report_empty_and_failed_saves() {
        let mut a = TokenAliases::default();
        let (out, _, saves) = tokens("recent", &mut a, false);
        assert!(out.contains("No recently used tokens"), "{out}");
        assert_eq!(saves, 0, "listing never writes the file");
        let (out, _, _) = tokens("list", &mut a, false);
        assert!(out.contains("Use 'crawl <token_name> --save'"), "{out}");
        // A failed save after a remove must be loud, not "Removed".
        tokens(&format!("add USDN ethereum {}", ADDR), &mut a, false);
        let (out, err, _) = tokens("remove USDN", &mut a, true);
        assert!(!out.contains("Removed"), "{out}");
        assert!(err.contains("Failed to save the token aliases: "), "{err}");
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
    fn test_monitor_never_gets_the_auto_chain() {
        // The REPL passed SessionContext.chain ("auto") straight to the
        // monitor, which has no "auto" handling. The TUI parses `monitor`
        // with clap: an auto context adds no --chain, so clap's default
        // applies; a pinned chain is passed as given.
        let inv = run_of(plan_line("monitor USDC", &mut SessionContext::default()).0);
        let Commands::Monitor(m) = inv.command else {
            panic!("expected monitor")
        };
        assert_eq!(m.chain, "ethereum");

        let mut ctx = SessionContext {
            chain: "solana".into(),
            ..Default::default()
        };
        let inv = run_of(plan_line("monitor USDC", &mut ctx).0);
        let Commands::Monitor(m) = inv.command else {
            panic!("expected monitor")
        };
        assert_eq!(m.chain, "solana");
    }

    #[test]
    fn test_hidden_panic_command_is_not_listed() {
        assert!(
            SESSION_COMMANDS
                .iter()
                .all(|(n, u, _)| *n != "__panic" && !u.contains("__panic"))
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
