//! `scope venues` subcommands for managing venue descriptors.
//!
//! Provides venue discovery, schema documentation, initialisation of user
//! venue directories, and YAML validation.

use crate::cli::output::Output;
use crate::outln;
use clap::{Args, Subcommand};
use scope::config::Config;
use scope::display::terminal::{
    check_fail, check_pass, info_row, kv_row, section_footer, section_header, separator,
};
use scope::error::Result;
use scope::http::HttpClient;
use scope::market::catalog;
use scope::market::registry::VenueSource;
use scope::market::{MarketVenue, VenueRegistry};
use std::sync::Arc;

/// Venue management subcommands.
///
/// List available exchange venues, view the YAML schema, initialise the
/// user venues directory, or validate a custom descriptor file.
///
/// # Examples
///
/// ```text
/// scope venues list
/// scope venues list --format json
/// scope venues schema
/// scope venues init
/// scope venues validate my-exchange.yaml
/// ```
#[derive(Debug, Subcommand)]
pub enum VenuesCommands {
    /// List all available exchange venues and their capabilities.
    ///
    /// Shows the catalog's and your own CEX venues with their supported
    /// API capabilities (order_book, ticker, trades, ohlc), then the DEX
    /// venues.
    ///
    /// # Examples
    ///
    /// ```text
    /// scope venues list
    /// scope venues list --format json
    /// ```
    List(ListArgs),

    /// Display the YAML schema for venue descriptors.
    ///
    /// Prints the expected structure, field descriptions, and an annotated
    /// example you can copy to create your own venue descriptor.
    ///
    /// # Examples
    ///
    /// ```text
    /// scope venues schema
    /// scope venues schema --format json
    /// ```
    Schema(SchemaArgs),

    /// Fetch the venue catalog: new and fixed venues without a new scope.
    ///
    /// Downloads the catalog index and every venue file, checks each
    /// against its SHA-256, and replaces ~/.scope-bca/venues/catalog/.
    /// Your own files in ~/.scope-bca/venues/ are not touched and still
    /// override a catalog venue with the same id.
    ///
    /// # Examples
    ///
    /// ```text
    /// scope venues update
    /// scope venues update --url https://example.com/my-catalog
    /// ```
    Update(UpdateArgs),

    /// Copy the catalog's venue files into your venues folder to customise.
    ///
    /// Copies each catalog YAML file as it is to ~/.scope-bca/venues/,
    /// where your copy overrides the catalog's. Fetches the catalog first
    /// if it is not there.
    ///
    /// # Examples
    ///
    /// ```text
    /// scope venues init
    /// scope venues init --force
    /// ```
    Init(InitArgs),

    /// Validate a venue descriptor YAML file.
    ///
    /// Parses the file against the VenueDescriptor schema and reports
    /// any errors or warnings. Exits with code 0 on success, 1 on failure.
    ///
    /// # Examples
    ///
    /// ```text
    /// scope venues validate my-exchange.yaml
    /// scope venues validate ~/.scope-bca/venues/custom.yaml
    /// ```
    Validate(ValidateArgs),
}

/// Arguments for `scope venues update`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope venues update
  scope venues update --url https://example.com/my-catalog")]
pub struct UpdateArgs {
    /// Fetch from this catalog folder URL (holding index.json) instead of
    /// the configured one (`market.venue_catalog_url`).
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
}

/// Arguments for `scope venues list`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope venues list
  scope venues list --format json")]
pub struct ListArgs {
    /// Output format.
    #[arg(short, long, default_value = "table")]
    pub format: ListFormat,
}

/// Output format for venue listing.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ListFormat {
    /// Human-readable table (default).
    #[default]
    Table,
    /// JSON for programmatic consumption.
    Json,
}

/// Arguments for `scope venues schema`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope venues schema
  scope venues schema --format json")]
pub struct SchemaArgs {
    /// Output format.
    #[arg(short, long, default_value = "text")]
    pub format: SchemaFormat,
}

/// Output format for schema display.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum SchemaFormat {
    /// Human-readable annotated text (default).
    #[default]
    Text,
    /// JSON Schema representation.
    Json,
}

/// Arguments for `scope venues init`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope venues init
  scope venues init --force")]
pub struct InitArgs {
    /// Overwrite existing files in the user venues directory.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `scope venues validate`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope venues validate my-exchange.yaml
  scope venues validate ~/.scope-bca/venues/custom.yaml")]
pub struct ValidateArgs {
    /// Path to the YAML file to validate.
    pub file: std::path::PathBuf,
}

/// Run the venues command.
pub async fn run(
    cmd: VenuesCommands,
    config: &Config,
    http: Arc<dyn HttpClient>,
    out: &Output,
) -> Result<()> {
    match cmd {
        VenuesCommands::List(args) => run_list(args, out),
        VenuesCommands::Schema(args) => run_schema(args, out),
        VenuesCommands::Update(args) => {
            let url = args.url.as_deref().unwrap_or(config.market.catalog_url());
            run_update(url, http.as_ref(), &VenueRegistry::catalog_dir(), out).await?;
            // A newer catalog may now be hidden by the user's own copies.
            let scripted =
                crate::cli::ghola_offer::scripted(&crate::cli::ghola_offer::process_args(), config);
            offer_catalog_updates(scripted, out).await;
            Ok(())
        }
        VenuesCommands::Init(args) => {
            if catalog::installed(&VenueRegistry::catalog_dir()).is_empty() {
                run_update(
                    config.market.catalog_url(),
                    http.as_ref(),
                    &VenueRegistry::catalog_dir(),
                    out,
                )
                .await?;
            }
            run_init(args, out)
        }
        VenuesCommands::Validate(args) => run_validate(args, out),
    }
}

// =============================================================================
// Update
// =============================================================================

/// Syncs the catalog from `url` into `dest` and reports what changed. The
/// example file goes next to the user's venues either way.
async fn run_update(
    url: &str,
    http: &dyn HttpClient,
    dest: &std::path::Path,
    out: &Output,
) -> Result<()> {
    let example = catalog::example_path();
    let _ = catalog::write_example(&example);
    let report = catalog::sync(http, url, dest).await?;
    outln!(out, "{}", section_header("Venue Catalog"))?;
    outln!(out, "{}", kv_row("Updated", &report.summary()))?;
    outln!(out, "{}", kv_row("From", url))?;
    outln!(out, "{}", kv_row("Catalog", &dest.display().to_string()))?;
    outln!(
        out,
        "{}",
        kv_row(
            "Your venues",
            &VenueRegistry::user_venues_dir().display().to_string()
        )
    )?;
    outln!(out, "{}", section_footer())?;
    Ok(())
}

/// Before a command that needs venues: when no catalog is installed, fetch
/// it once (first use), saying so. Offline, it says how to get venues later
/// and leaves the example file; the command then runs with what there is.
pub async fn ensure_catalog(config: &Config, http: &dyn HttpClient, out: &Output) {
    static TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    ensure_catalog_at(
        config.market.catalog_url(),
        http,
        &VenueRegistry::catalog_dir(),
        &catalog::example_path(),
        &TRIED,
        out,
    )
    .await;
}

/// [`ensure_catalog`] for the catalog `dest`, the `example` file and a
/// once-per-process `tried` flag.
async fn ensure_catalog_at(
    url: &str,
    http: &dyn HttpClient,
    dest: &std::path::Path,
    example: &std::path::Path,
    tried: &std::sync::atomic::AtomicBool,
    out: &Output,
) {
    if !catalog::installed(dest).is_empty() || tried.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let _ = catalog::write_example(example);
    crate::note!(out, "Fetching the venue catalog (first use)…");
    match catalog::sync(http, url, dest).await {
        Ok(report) => crate::note!(out, "Venue catalog installed: {}", report.summary()),
        Err(e) => crate::note!(
            out,
            "Could not fetch the venue catalog ({e}). Run `scope venues update` when online, or describe an exchange yourself: see {}",
            example.display()
        ),
    }
}

// =============================================================================
// Stale copies: your files that hide a newer catalog venue
// =============================================================================

/// The question, in the user's words.
const OVERRIDE_QUESTION: &str = "Would you like me to save your existing copies and overwrite? [Y]es/[N]o/[O]verwrite, don't save: ";

/// What to do with the user's copies that hide catalog capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideAnswer {
    /// Move the copies to a backup folder; the catalog's apply.
    SaveAndReplace,
    /// Keep them, and do not ask again until the catalog changes.
    Keep,
    /// Remove the copies without a backup; the catalog's apply.
    Replace,
}

/// `y`, `n` or `o` (any case, a word starting with one); anything else,
/// an empty line included, keeps the copies.
pub fn parse_override_answer(answer: &str) -> OverrideAnswer {
    match answer.trim().chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('y') => OverrideAnswer::SaveAndReplace,
        Some('o') => OverrideAnswer::Replace,
        _ => OverrideAnswer::Keep,
    }
}

/// One of the user's files that hides a newer catalog venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleCopy {
    pub id: String,
    pub file: std::path::PathBuf,
    /// What the catalog's version has that the copy lacks.
    pub hidden: Vec<&'static str>,
    /// The catalog file's checksum, so a "no" lasts until it changes.
    pub catalog_sha: String,
}

/// The user's copies in `registry` that hide catalog capabilities, minus
/// those already declined for the same catalog version (`declined`).
pub fn stale_copies(
    registry: &VenueRegistry,
    catalog_dir: &std::path::Path,
    declined: &std::collections::HashMap<String, String>,
) -> Vec<StaleCopy> {
    let shas = catalog::installed(catalog_dir);
    registry
        .list()
        .into_iter()
        .filter_map(|id| {
            let hidden = registry.hidden_capabilities(id);
            let file = registry.user_file(id)?.to_path_buf();
            let catalog_sha = shas.iter().find(|e| e.id == id)?.sha256.clone();
            (!hidden.is_empty() && declined.get(id) != Some(&catalog_sha)).then(|| StaleCopy {
                id: id.to_string(),
                file,
                hidden,
                catalog_sha,
            })
        })
        .collect()
}

/// Where the "no" answers are kept: `{id: catalog sha256}`.
fn declined_path() -> std::path::PathBuf {
    scope::paths::state_dir().join("venue-overrides-declined.json")
}

fn read_declined(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Carries out `answer` for `stale`: moves the copies to a backup folder
/// under `home` (named by `stamp`), removes them, or records the "no" in
/// `declined_file`. Returns what it did, for the user.
pub fn apply_override_answer(
    answer: OverrideAnswer,
    stale: &[StaleCopy],
    home: &std::path::Path,
    stamp: &str,
    declined_file: &std::path::Path,
) -> Result<String> {
    let names = stale
        .iter()
        .map(|c| c.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    match answer {
        OverrideAnswer::SaveAndReplace => {
            let backup = home.join(format!("venues-backup-{stamp}"));
            std::fs::create_dir_all(&backup).map_err(|e| {
                scope::error::ScopeError::io_context(
                    e,
                    format!("Cannot create {}", backup.display()),
                )
            })?;
            for c in stale {
                let to = backup.join(c.file.file_name().unwrap_or_default());
                std::fs::rename(&c.file, &to).map_err(|e| {
                    scope::error::ScopeError::io_context(
                        e,
                        format!("Cannot move {}", c.file.display()),
                    )
                })?;
            }
            Ok(format!(
                "Saved your copies of {names} to {} and switched to the catalog's.",
                backup.display()
            ))
        }
        OverrideAnswer::Replace => {
            for c in stale {
                std::fs::remove_file(&c.file).map_err(|e| {
                    scope::error::ScopeError::io_context(
                        e,
                        format!("Cannot remove {}", c.file.display()),
                    )
                })?;
            }
            Ok(format!(
                "Removed your copies of {names}; the catalog's apply."
            ))
        }
        OverrideAnswer::Keep => {
            let mut declined = read_declined(declined_file);
            for c in stale {
                declined.insert(c.id.clone(), c.catalog_sha.clone());
            }
            if let Some(dir) = declined_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let json = serde_json::to_string_pretty(&declined).unwrap_or_default();
            std::fs::write(declined_file, json).map_err(|e| {
                scope::error::ScopeError::io_context(
                    e,
                    format!("Cannot write {}", declined_file.display()),
                )
            })?;
            Ok(format!(
                "Kept your copies of {names}; scope will ask again when the catalog changes."
            ))
        }
    }
}

/// The question's text: which venues, and what the catalog adds.
fn override_prompt(stale: &[StaleCopy]) -> String {
    let lines: Vec<String> = stale
        .iter()
        .map(|c| {
            format!(
                "  {} ({}): the catalog's also has {}",
                c.id,
                c.file.display(),
                c.hidden.join(", ")
            )
        })
        .collect();
    format!(
        "New version of the venues catalog found. Your own venue files hide what it adds:\n{}\n",
        lines.join("\n")
    )
}

/// Before a command that reads venues: when the user's copies hide a newer
/// catalog venue, ask (once a process) whether to save and replace them,
/// replace them, or keep them. In the TUI the question is a list; on a
/// terminal, the `[Y]es/[N]o/[O]verwrite` prompt; scripted runs never ask.
pub async fn offer_catalog_updates(scripted: bool, out: &Output) {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if scripted || ASKED.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(registry) = VenueRegistry::load() else {
        return;
    };
    let asked = offer_with(
        &registry,
        &VenueRegistry::catalog_dir(),
        &declined_path(),
        &scope::paths::home(),
        out,
    )
    .await;
    if asked {
        ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// [`offer_catalog_updates`] for `registry`, its catalog folder, the file
/// of "no" answers and the scope home (where backups go). Returns whether
/// it asked.
async fn offer_with(
    registry: &VenueRegistry,
    catalog_dir: &std::path::Path,
    declined_file: &std::path::Path,
    home: &std::path::Path,
    out: &Output,
) -> bool {
    let stale = stale_copies(registry, catalog_dir, &read_declined(declined_file));
    if stale.is_empty() {
        return false;
    }
    let answer = if let Some(asker) = out.asker() {
        let picker = crate::cli::resolve::TuiPicker { asker };
        let choice = picker
            .ask(
                format!("{}Use the catalog's versions?", override_prompt(&stale)),
                vec![
                    "Yes: save my copies, then use the catalog's".to_string(),
                    "No: keep my copies".to_string(),
                    "Overwrite: use the catalog's, don't save my copies".to_string(),
                ],
            )
            .await;
        match choice {
            Some(0) => OverrideAnswer::SaveAndReplace,
            Some(2) => OverrideAnswer::Replace,
            // A cancelled question: nothing changes, and it was asked.
            None => return true,
            _ => OverrideAnswer::Keep,
        }
    } else {
        if !crate::cli::ghola_offer::may_ask(out) {
            return false;
        }
        let question = format!("{}{OVERRIDE_QUESTION}", override_prompt(&stale));
        match crate::cli::prompt::ask_stdin(out, &question) {
            Ok(a) => parse_override_answer(&a),
            Err(_) => return false,
        }
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S").to_string();
    match apply_override_answer(answer, &stale, home, &stamp, declined_file) {
        Ok(done) => crate::note!(out, "{done}"),
        Err(e) => crate::note!(out, "Could not update your venue files: {e}"),
    }
    true
}

// =============================================================================
// List
// =============================================================================

fn run_list(args: ListArgs, out: &Output) -> Result<()> {
    run_list_with(&VenueRegistry::load()?, args, out)
}

/// [`run_list`] for `registry`.
fn run_list_with(registry: &VenueRegistry, args: ListArgs, out: &Output) -> Result<()> {
    match args.format {
        ListFormat::Table => {
            outln!(out, "{}", section_header("Available Venues"))?;
            for listing in MarketVenue::listings(registry) {
                let caps = listing.capabilities.join(", ");
                let detail = match (listing.chain, registry.source(&listing.id)) {
                    (Some(chain), _) => format!("{caps} (DEX on {chain})"),
                    (None, Some(VenueSource::User)) => {
                        match registry.hidden_capabilities(&listing.id).as_slice() {
                            [] => format!("{caps} (yours)"),
                            hidden => format!(
                                "{caps} (yours; the catalog's also has {})",
                                hidden.join(", ")
                            ),
                        }
                    }
                    (None, _) => caps,
                };
                outln!(out, "{}", kv_row(&listing.id, &detail))?;
            }
            outln!(out, "{}", separator())?;
            let user_dir = VenueRegistry::user_venues_dir();
            let ids = registry.list();
            let yours = ids
                .iter()
                .filter(|id| registry.source(id) == Some(VenueSource::User))
                .count();
            outln!(
                out,
                "{}",
                kv_row(
                    "Loaded",
                    &format!(
                        "{} venues ({} from the catalog, {} yours)",
                        ids.len(),
                        ids.len() - yours,
                        yours
                    )
                )
            )?;
            if ids.is_empty() {
                outln!(
                    out,
                    "{}",
                    info_row("No exchange venues yet: run `scope venues update`")
                )?;
            }
            for id in &ids {
                if let Some(hint) = registry.override_hint(id) {
                    outln!(out, "{}", info_row(&hint))?;
                }
            }
            outln!(
                out,
                "{}",
                kv_row(
                    "Catalog",
                    &VenueRegistry::catalog_dir().display().to_string()
                )
            )?;
            outln!(
                out,
                "{}",
                kv_row("Your venues", &user_dir.display().to_string())
            )?;
            outln!(out, "{}", section_footer())?;
        }
        ListFormat::Json => {
            // The same list the web serves at /api/venues.
            let venues = MarketVenue::listings(registry);
            let output = serde_json::json!({
                "total": venues.len(),
                "venues": venues,
                "user_venues_dir": VenueRegistry::user_venues_dir().display().to_string(),
                "catalog_dir": VenueRegistry::catalog_dir().display().to_string(),
            });
            outln!(out, "{}", serde_json::to_string_pretty(&output).unwrap())?;
        }
    }

    Ok(())
}

// =============================================================================
// Schema
// =============================================================================

fn run_schema(args: SchemaArgs, out: &Output) -> Result<()> {
    match args.format {
        SchemaFormat::Text => print_annotated_schema(out)?,
        SchemaFormat::Json => print_json_schema(out)?,
    }
    Ok(())
}

fn print_annotated_schema(out: &Output) -> std::io::Result<()> {
    outln!(out, "{}", section_header("Venue Descriptor Schema"))?;
    outln!(
        out,
        r#"
A venue descriptor is a YAML file that tells Scope how to talk to
an exchange API. Place custom descriptors in:

  {}

Each file defines one venue with the following structure:

  id: my_exchange              # Unique ID (lowercase, underscores ok)
  name: My Exchange            # Human-readable display name
  base_url: https://api.example.com  # API base URL

  symbol:
    template: "{{base}}{{quote}}"  # Pair format (placeholders: {{base}}, {{quote}})
    default_quote: USDT          # Quote currency when user omits it
    case: upper                  # "upper" or "lower" (default: upper)

  error_envelope:                # Optional: errors sent with HTTP 200
    code_key: retCode            # Status field; omit to test message_key only
    ok_codes: ["0"]              # Status values that mean success
    message_key: retMsg          # Error text (a string, or an array like Kraken's)

  capabilities:
    order_book:                  # Omit if the venue has no depth API
      method: GET                # GET (default) or POST
      path: /api/v1/depth       # URL path appended to base_url
      params:                    # Query parameters (GET) — values can use {{pair}}, {{limit}}
        symbol: "{{pair}}"
        limit: "{{limit}}"
      response_root: data        # JSON path to the relevant data (optional)
      response:
        asks_key: asks           # JSON key for asks array
        bids_key: bids           # JSON key for bids array
        level_format: positional # "positional" for [price, qty] or "object"
        level_price_field: price # Only needed when level_format = object
        level_size_field: size   # Only needed when level_format = object

    ticker:                      # Omit if the venue has no ticker API
      path: /api/v1/ticker
      params:
        symbol: "{{pair}}"
      response:
        last_price: lastPrice
        high_24h: highPrice
        low_24h: lowPrice
        volume_24h: volume
        quote_volume_24h: quoteVolume
        price_change_24h: priceChange
        price_change_pct_24h: priceChangePercent

    trades:                      # Omit if the venue has no trades API
      path: /api/v1/trades
      params:
        symbol: "{{pair}}"
        limit: "{{limit}}"
      response:
        items_key: data          # JSON key containing the trades array (omit if root)
        price: price             # Field for trade price
        quantity: qty            # Field for trade quantity
        timestamp_ms: time       # Field for trade timestamp (epoch ms)
        side:                    # Side detection
          field: side            # JSON field containing buy/sell indicator
          mapping:
            buy: buy
            sell: sell

Validate your file with:  scope venues validate <file>
"#,
        VenueRegistry::user_venues_dir().display()
    )?;
    outln!(out, "{}", section_footer())?;
    Ok(())
}

fn print_json_schema(out: &Output) -> std::io::Result<()> {
    use serde_json::{Map, Value};

    fn str_prop(desc: &str) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), Value::String("string".into()));
        m.insert("description".into(), Value::String(desc.into()));
        Value::Object(m)
    }

    fn str_type() -> Value {
        serde_json::json!({"type": "string"})
    }

    // Build response properties
    let mut resp_props = Map::new();
    for key in &[
        "asks_key",
        "bids_key",
        "level_format",
        "level_price_field",
        "level_size_field",
        "last_price",
        "high_24h",
        "low_24h",
        "volume_24h",
        "quote_volume_24h",
        "best_bid",
        "best_ask",
        "items_key",
        "price",
        "quantity",
        "quote_quantity",
        "timestamp_ms",
        "id",
    ] {
        resp_props.insert((*key).into(), str_type());
    }
    resp_props.insert(
        "filter".into(),
        serde_json::json!({
            "type": "object",
            "properties": {"field": {"type": "string"}, "value": {"type": "string"}}
        }),
    );
    resp_props.insert(
        "side".into(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "field": {"type": "string"},
                "mapping": {"type": "object", "additionalProperties": {"type": "string"}}
            }
        }),
    );

    // Build endpoint def
    let endpoint_def = serde_json::json!({
        "type": "object",
        "required": ["path", "response"],
        "properties": {
            "method": {"type": "string", "enum": ["GET", "POST"], "default": "GET"},
            "path": {"type": "string"},
            "params": {"type": "object", "additionalProperties": {"type": "string"}},
            "request_body": {"description": "JSON body template for POST requests"},
            "response_root": {"type": "string", "description": "Dot-path to navigate JSON response"},
            "response": {"type": "object", "properties": Value::Object(resp_props)}
        }
    });

    let endpoint_ref = serde_json::json!({"$ref": "#/$defs/endpoint"});

    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "VenueDescriptor",
        "description": "Schema for Scope exchange venue descriptor YAML files.",
        "type": "object",
        "required": ["id", "name", "base_url", "symbol", "capabilities"],
        "properties": {
            "id": str_prop("Unique venue identifier (e.g., 'binance')"),
            "name": str_prop("Human-readable venue name (e.g., 'Binance Spot')"),
            "base_url": str_prop("API base URL (e.g., 'https://api.binance.com')"),
            "symbol": {
                "type": "object",
                "required": ["template", "default_quote"],
                "properties": {
                    "template": str_prop("Pair template with {base} and {quote} placeholders"),
                    "default_quote": str_prop("Default quote currency (e.g., 'USDT')"),
                    "case": {"type": "string", "enum": ["upper", "lower"], "default": "upper"}
                }
            },
            "error_envelope": {
                "type": "object",
                "description": "How the venue reports an error inside an HTTP 200 reply",
                "required": ["message_key"],
                "properties": {
                    "code_key": str_prop("Status field; a missing field means success"),
                    "ok_codes": {"type": "array", "items": {"type": "string"}, "description": "Status values that mean success, compared as strings"},
                    "message_key": str_prop("Field with the error message (string or array)")
                }
            },
            "capabilities": {
                "type": "object",
                "properties": {
                    "order_book": endpoint_ref.clone(),
                    "ticker": endpoint_ref.clone(),
                    "trades": endpoint_ref
                }
            }
        },
        "$defs": {
            "endpoint": endpoint_def
        }
    });

    outln!(out, "{}", serde_json::to_string_pretty(&schema).unwrap())?;
    Ok(())
}

// =============================================================================
// Init
// =============================================================================

fn run_init(args: InitArgs, out: &Output) -> Result<()> {
    run_init_impl(args, VenueRegistry::user_venues_dir(), out)
}

/// Core init logic with explicit destination path (used by tests).
fn run_init_impl(args: InitArgs, dest: std::path::PathBuf, out: &Output) -> Result<()> {
    // Ensure directory exists
    if !dest.exists() {
        std::fs::create_dir_all(&dest).map_err(|e| {
            let context = format!("Failed to create venues directory {}", dest.display());
            scope::error::ScopeError::io_context(e, context)
        })?;
        outln!(out, "Created {}", dest.display())?;
    }

    // The catalog's files, copied as they are: a re-serialised copy lost
    // what the serialiser did not know (the `ohlc` capability).
    let source = VenueRegistry::catalog_dir();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&source)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return Err(scope::error::ScopeError::InvalidInput(
            "The venue catalog is not installed: run `scope venues update` first.".into(),
        ));
    }
    let mut copied = 0;
    let mut skipped = 0;

    for path in files {
        let filename = path.file_name().unwrap().to_string_lossy().into_owned();
        let target = dest.join(&filename);

        if target.exists() && !args.force {
            skipped += 1;
            outln!(
                out,
                "  skip {} (exists, use --force to overwrite)",
                filename
            )?;
            continue;
        }

        std::fs::copy(&path, &target).map_err(|e| {
            scope::error::ScopeError::io_context(e, format!("Failed to write {}", target.display()))
        })?;
        copied += 1;
        outln!(out, "  {}", check_pass(&filename))?;
    }

    outln!(out)?;
    outln!(out, "{}", section_header("Venues Init"))?;
    outln!(out, "{}", kv_row("Directory", &dest.display().to_string()))?;
    outln!(out, "{}", kv_row("Copied", &format!("{} files", copied)))?;
    if skipped > 0 {
        outln!(
            out,
            "{}",
            kv_row("Skipped", &format!("{} files (already exist)", skipped))
        )?;
    }
    outln!(out, "{}", section_footer())?;

    Ok(())
}

// =============================================================================
// Validate
// =============================================================================

fn run_validate(args: ValidateArgs, out: &Output) -> Result<()> {
    let path = &args.file;

    if !path.exists() {
        outln!(
            out,
            "{}",
            check_fail(&format!("File not found: {}", path.display()))
        )?;
        return Err(scope::error::ScopeError::Chain(format!(
            "File not found: {}",
            path.display()
        )));
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        scope::error::ScopeError::io_context(e, format!("Failed to read {}", path.display()))
    })?;

    outln!(out, "{}", section_header("Venue Validation"))?;
    outln!(out, "{}", kv_row("File", &path.display().to_string()))?;
    outln!(out, "{}", separator())?;

    match VenueRegistry::validate_yaml(&content) {
        Ok(desc) => {
            outln!(out, "{}", check_pass("Valid YAML syntax"))?;
            outln!(out, "{}", check_pass(&format!("Venue ID: {}", desc.id)))?;
            outln!(out, "{}", check_pass(&format!("Name: {}", desc.name)))?;
            outln!(
                out,
                "{}",
                check_pass(&format!("Base URL: {}", desc.base_url))
            )?;

            // Check capabilities
            let caps = desc.capability_names();
            if caps.is_empty() {
                outln!(
                    out,
                    "{}",
                    check_fail(
                        "No capabilities defined (need at least one of: order_book, ticker, trades)"
                    )
                )?;
            } else {
                for cap in &caps {
                    outln!(out, "{}", check_pass(&format!("Capability: {}", cap)))?;
                }
            }

            // Validate symbol template
            if desc.symbol.template.contains("{base}") {
                outln!(
                    out,
                    "{}",
                    check_pass("Symbol template contains {base} placeholder")
                )?;
            } else {
                outln!(
                    out,
                    "{}",
                    check_fail("Symbol template missing {base} placeholder")
                )?;
            }

            outln!(out, "{}", separator())?;
            if caps.is_empty() {
                outln!(out, "{}", check_fail("Validation completed with warnings"))?;
            } else {
                outln!(out, "{}", check_pass("Validation passed"))?;
            }
            outln!(out, "{}", section_footer())?;
            Ok(())
        }
        Err(e) => {
            outln!(out, "{}", check_fail("Invalid YAML"))?;
            outln!(out, "{}", check_fail(&format!("Error: {}", e)))?;
            outln!(out, "{}", separator())?;
            outln!(
                out,
                "{}",
                kv_row(
                    "Hint",
                    "Run `scope venues schema` to see the expected format"
                )
            )?;
            outln!(out, "{}", section_footer())?;
            Err(e)
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_list_format_default() {
        let fmt = ListFormat::default();
        assert!(matches!(fmt, ListFormat::Table));
    }

    #[test]
    fn test_schema_format_default() {
        let fmt = SchemaFormat::default();
        assert!(matches!(fmt, SchemaFormat::Text));
    }

    #[test]
    fn test_run_list_table() {
        let args = ListArgs {
            format: ListFormat::Table,
        };
        let result = run_list(args, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_json() {
        let args = ListArgs {
            format: ListFormat::Json,
        };
        let (o, cap) = Output::capture();
        run_list(args, &o).unwrap();
        // Scripts parse this: it must be one JSON document on the data channel.
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert!(v["venues"].as_array().is_some_and(|a| !a.is_empty()));
        assert_eq!(
            v["total"].as_u64().unwrap() as usize,
            v["venues"].as_array().unwrap().len()
        );
        // DEX venues are listed too, marked by kind.
        assert!(
            v["venues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|venue| venue["kind"] == "dex")
        );
    }

    #[test]
    fn test_run_schema_text() {
        let args = SchemaArgs {
            format: SchemaFormat::Text,
        };
        let result = run_schema(args, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_schema_json() {
        let args = SchemaArgs {
            format: SchemaFormat::Json,
        };
        let (o, cap) = Output::capture();
        run_schema(args, &o).unwrap();
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert!(v.is_object());
    }

    #[test]
    fn test_validate_missing_file() {
        let args = ValidateArgs {
            file: std::path::PathBuf::from("/tmp/nonexistent_venue_test.yaml"),
        };
        let result = run_validate(args, &quiet());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_valid_file() {
        let yaml = r#"
id: test_venue
name: Test Exchange
base_url: https://api.test.com
symbol:
  template: "{base}{quote}"
  default_quote: USDT
capabilities:
  order_book:
    path: /depth
    params:
      symbol: "{pair}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        std::fs::write(&path, yaml).unwrap();

        let args = ValidateArgs { file: path };
        let (o, cap) = Output::capture();
        run_validate(args, &o).unwrap();
        assert!(cap.out().contains("Venue ID: test_venue"));
        assert!(cap.out().contains("Validation passed"));
    }

    #[test]
    fn test_validate_invalid_file() {
        let yaml = "this is not valid yaml: [";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, yaml).unwrap();

        let args = ValidateArgs { file: path };
        let result = run_validate(args, &quiet());
        assert!(result.is_err());
    }

    #[test]
    fn test_run_init_to_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().to_path_buf();
        let args = InitArgs { force: true };
        let result = run_init_impl(args, dest.clone(), &quiet());
        assert!(result.is_ok());

        // Verify venue files were created (registry has 11 built-in venues)
        let registry = VenueRegistry::load().unwrap();
        for id in registry.list() {
            let filename = format!("{}.yaml", id);
            let target = dest.join(&filename);
            assert!(target.exists(), "Expected {} to exist", filename);
            let content = std::fs::read_to_string(&target).unwrap();
            assert!(content.contains(&format!("id: {}", id)));
        }
    }

    #[test]
    fn test_run_init_skips_existing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().to_path_buf();

        // Write an existing binance.yaml before init
        let existing_path = dest.join("binance.yaml");
        std::fs::create_dir_all(&dest).unwrap();
        let original_content = "id: binance\n# pre-existing file\n";
        std::fs::write(&existing_path, original_content).unwrap();

        // Run init with force=false
        let args = InitArgs { force: false };
        let result = run_init_impl(args, dest.clone(), &quiet());
        assert!(result.is_ok());

        // Verify binance.yaml was NOT overwritten (content unchanged)
        let content = std::fs::read_to_string(&existing_path).unwrap();
        assert_eq!(
            content, original_content,
            "Existing file should not be overwritten when force=false"
        );
    }

    #[test]
    fn test_run_init_creates_directory_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested").join("venues");
        // Ensure parent doesn't exist
        assert!(!dest.exists());
        let args = InitArgs { force: true };
        let result = run_init_impl(args, dest.clone(), &quiet());
        assert!(result.is_ok());
        assert!(dest.exists());
        assert!(dest.is_dir());
    }

    #[test]
    fn test_validate_file_template_missing_base() {
        let yaml = r#"
id: bad_template
name: Bad Template Exchange
base_url: https://api.test.com
symbol:
  template: "nobase{quote}"
  default_quote: USDT
capabilities:
  order_book:
    path: /depth
    params:
      symbol: "{pair}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad_template.yaml");
        std::fs::write(&path, yaml).unwrap();

        let args = ValidateArgs { file: path };
        let result = run_validate(args, &quiet());
        assert!(result.is_ok()); // YAML parses; check_fail is printed for missing {base}
    }

    #[test]
    fn test_validate_file_empty_capabilities() {
        let yaml = r#"
id: no_caps
name: No Capabilities
base_url: https://api.test.com
symbol:
  template: "{base}{quote}"
  default_quote: USDT
capabilities: {}
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_caps.yaml");
        std::fs::write(&path, yaml).unwrap();

        let args = ValidateArgs { file: path };
        let result = run_validate(args, &quiet());
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_venues_command_routes_to_subcommands() {
        let config = Config::default();
        let http = scope::http::test_transport();
        let result = run(
            VenuesCommands::List(ListArgs {
                format: ListFormat::Table,
            }),
            &config,
            http.clone(),
            &quiet(),
        )
        .await;
        assert!(result.is_ok());
        let result = run(
            VenuesCommands::Schema(SchemaArgs {
                format: SchemaFormat::Text,
            }),
            &config,
            http,
            &quiet(),
        )
        .await;
        assert!(result.is_ok());
    }

    /// A catalog served by a local server: `index.json` and one venue.
    async fn catalog_server(venue: &str) -> mockito::ServerGuard {
        let mut server = mockito::Server::new_async().await;
        let index = serde_json::json!({
            "schema": 1,
            "venues": [{"id": "weex", "file": "weex.yaml", "sha256": catalog::checksum(venue)}],
        });
        server
            .mock("GET", "/c/index.json")
            .with_body(index.to_string())
            .create_async()
            .await;
        server
            .mock("GET", "/c/weex.yaml")
            .with_body(venue)
            .create_async()
            .await;
        server
    }

    const WEEX: &str = "id: weex\nname: WEEX Spot\nbase_url: https://api-spot.weex.com\nsymbol:\n  template: \"{base}{quote}\"\n  default_quote: USDT\ncapabilities:\n  ohlc:\n    path: /api/v3/market/klines\n    response:\n      ohlc_format: array_of_arrays\n      ohlc_fields: [open_time, open, high, low, close, volume, close_time]\n";

    #[tokio::test]
    async fn test_update_installs_the_catalog_and_reports_it() {
        let server = catalog_server(WEEX).await;
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("catalog");
        let (out, cap) = Output::capture();
        run_update(
            &format!("{}/c", server.url()),
            scope::http::test_transport().as_ref(),
            &dest,
            &out,
        )
        .await
        .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        assert!(
            text.contains("1 venues: 1 added (weex), 0 updated, 0 removed"),
            "{text}"
        );
        let registry = VenueRegistry::load_dirs(&dest, &home.path().join("none")).unwrap();
        // The file is the catalog's, byte for byte: nothing is re-serialised.
        assert_eq!(
            std::fs::read_to_string(dest.join("weex.yaml")).unwrap(),
            WEEX
        );
        assert!(registry.get("weex").unwrap().capabilities.ohlc.is_some());
    }

    #[test]
    fn test_init_copies_the_catalog_files_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("venues");
        run_init_impl(InitArgs { force: false }, dest.clone(), &quiet()).unwrap();
        let catalog = VenueRegistry::catalog_dir();
        for name in ["binance.yaml", "kraken.yaml"] {
            assert_eq!(
                std::fs::read(dest.join(name)).unwrap(),
                std::fs::read(catalog.join(name)).unwrap(),
                "{name} is copied as it is"
            );
        }
        // ohlc survives, which the old re-serialised copies lost.
        let copied = VenueRegistry::load_dirs(&dir.path().join("none"), &dest).unwrap();
        assert!(copied.get("binance").unwrap().capabilities.ohlc.is_some());
    }

    #[test]
    fn test_list_marks_your_own_venues() {
        let (out, cap) = Output::capture();
        run_list(
            ListArgs {
                format: ListFormat::Table,
            },
            &out,
        )
        .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        assert!(text.contains("from the catalog"), "{text}");
        assert!(text.contains("Catalog"), "{text}");
    }

    #[tokio::test]
    async fn test_first_use_fetches_the_catalog_once_and_says_so() {
        let server = catalog_server(WEEX).await;
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("catalog");
        let example = home.path().join(catalog::EXAMPLE_FILE);
        let tried = std::sync::atomic::AtomicBool::new(false);
        let http = scope::http::test_transport();
        let url = format!("{}/c", server.url());
        let (out, cap) = Output::capture();
        ensure_catalog_at(&url, http.as_ref(), &dest, &example, &tried, &out).await;
        let notes = cap.err();
        assert!(
            notes.contains("Fetching the venue catalog (first use)"),
            "{notes}"
        );
        assert!(notes.contains("1 added (weex)"), "{notes}");
        assert!(example.exists(), "the example is written");
        // Installed: nothing more, even with a fresh flag.
        let (out, cap) = Output::capture();
        let fresh = std::sync::atomic::AtomicBool::new(false);
        ensure_catalog_at(&url, http.as_ref(), &dest, &example, &fresh, &out).await;
        assert!(cap.err().is_empty(), "{}", cap.err());
    }

    #[tokio::test]
    async fn test_first_use_offline_says_how_to_get_venues_and_tries_once() {
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("catalog");
        let example = home.path().join(catalog::EXAMPLE_FILE);
        let tried = std::sync::atomic::AtomicBool::new(false);
        let http = scope::http::test_transport();
        let (out, cap) = Output::capture();
        ensure_catalog_at(
            "http://127.0.0.1:9/none",
            http.as_ref(),
            &dest,
            &example,
            &tried,
            &out,
        )
        .await;
        let notes = cap.err();
        assert!(
            notes.contains("Could not fetch the venue catalog"),
            "{notes}"
        );
        assert!(notes.contains("scope venues update"), "{notes}");
        assert!(notes.contains(catalog::EXAMPLE_FILE), "{notes}");
        // Once a process: the second command does not try again.
        let (out, cap) = Output::capture();
        ensure_catalog_at(
            "http://127.0.0.1:9/none",
            http.as_ref(),
            &dest,
            &example,
            &tried,
            &out,
        )
        .await;
        assert!(cap.err().is_empty(), "{}", cap.err());
    }

    #[tokio::test]
    async fn test_update_from_an_unreachable_url_fails_and_keeps_the_catalog() {
        let before = catalog::installed(&VenueRegistry::catalog_dir());
        let result = run(
            VenuesCommands::Update(UpdateArgs {
                url: Some("http://127.0.0.1:9/none".into()),
            }),
            &Config::default(),
            scope::http::test_transport(),
            &quiet(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(catalog::installed(&VenueRegistry::catalog_dir()), before);
    }

    #[test]
    fn test_list_with_no_venues_says_how_to_get_them_and_marks_yours() {
        let (out, cap) = Output::capture();
        run_list_with(
            &VenueRegistry::default(),
            ListArgs {
                format: ListFormat::Table,
            },
            &out,
        )
        .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        assert!(
            text.contains("0 venues (0 from the catalog, 0 yours)"),
            "{text}"
        );
        assert!(text.contains("No exchange venues yet"), "{text}");

        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("weex.yaml"), WEEX).unwrap();
        let registry =
            VenueRegistry::load_dirs(&VenueRegistry::catalog_dir(), user.path()).unwrap();
        let (out, cap) = Output::capture();
        run_list_with(
            &registry,
            ListArgs {
                format: ListFormat::Table,
            },
            &out,
        )
        .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        // This weex describes only candles: it hides the catalog's book,
        // ticker and trades, and the list says so (wrapped at any width).
        let words = text
            .split_whitespace()
            .filter(|w| *w != "│")
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            words.contains("ohlc (yours; the catalog's also has order_book, ticker, trades)"),
            "{text}"
        );
        assert!(text.contains(", 1 yours)"), "{text}");
    }

    #[test]
    fn test_list_says_when_your_file_hides_catalog_capabilities() {
        let user = tempfile::tempdir().unwrap();
        let without_ohlc = &WEEX[..WEEX.find("  ohlc:").unwrap()];
        let weex_catalog = tempfile::tempdir().unwrap();
        std::fs::write(weex_catalog.path().join("weex.yaml"), WEEX).unwrap();
        std::fs::write(user.path().join("weex.yaml"), without_ohlc).unwrap();
        let registry = VenueRegistry::load_dirs(weex_catalog.path(), user.path()).unwrap();
        let (out, cap) = Output::capture();
        run_list_with(
            &registry,
            ListArgs {
                format: ListFormat::Table,
            },
            &out,
        )
        .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        // The hint wraps under the box rail.
        let words = text
            .split_whitespace()
            .filter(|w| *w != "│")
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            words.contains("(yours; the catalog's also has ohlc)"),
            "{text}"
        );
        assert!(
            words.contains("overrides the catalog's weex, which also has ohlc"),
            "{text}"
        );
    }

    #[test]
    fn test_override_answers_parse_like_the_prompt_says() {
        for (input, want) in [
            ("y", OverrideAnswer::SaveAndReplace),
            ("Yes\n", OverrideAnswer::SaveAndReplace),
            ("o", OverrideAnswer::Replace),
            (" Overwrite", OverrideAnswer::Replace),
            ("n", OverrideAnswer::Keep),
            ("", OverrideAnswer::Keep),
            ("maybe", OverrideAnswer::Keep),
        ] {
            assert_eq!(parse_override_answer(input), want, "{input:?}");
        }
    }

    /// A catalog with `weex` (with ohlc) and its index, and a user folder
    /// with a weex that lacks it.
    fn stale_setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let catalog_dir = home.path().join("venues").join("catalog");
        let user = home.path().join("venues");
        std::fs::create_dir_all(&catalog_dir).unwrap();
        std::fs::write(catalog_dir.join("weex.yaml"), WEEX).unwrap();
        let index = serde_json::json!({"schema": 1, "venues": [
            {"id": "weex", "file": "weex.yaml", "sha256": catalog::checksum(WEEX)}
        ]});
        std::fs::write(catalog_dir.join("index.json"), index.to_string()).unwrap();
        std::fs::write(
            user.join("weex.yaml"),
            &WEEX[..WEEX.find("  ohlc:").unwrap()],
        )
        .unwrap();
        (home, catalog_dir, user)
    }

    #[test]
    fn test_stale_copies_are_found_until_declined_for_that_catalog_version() {
        let (_home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let none = std::collections::HashMap::new();
        let stale = stale_copies(&registry, &catalog_dir, &none);
        assert_eq!(stale.len(), 1);
        assert_eq!(
            (stale[0].id.as_str(), stale[0].hidden.as_slice()),
            ("weex", &["ohlc"][..])
        );
        assert!(
            override_prompt(&stale).contains("weex"),
            "{}",
            override_prompt(&stale)
        );
        // Declined for this catalog version: not asked again.
        let declined =
            std::collections::HashMap::from([("weex".to_string(), catalog::checksum(WEEX))]);
        assert!(stale_copies(&registry, &catalog_dir, &declined).is_empty());
        // Declined for an older one: asked again.
        let old = std::collections::HashMap::from([("weex".to_string(), "old".to_string())]);
        assert_eq!(stale_copies(&registry, &catalog_dir, &old).len(), 1);
    }

    #[test]
    fn test_yes_saves_and_replaces_overwrite_removes_no_remembers() {
        // Yes: the copy moves to a backup; the catalog's applies.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let stale = stale_copies(&registry, &catalog_dir, &Default::default());
        let declined = home.path().join("state").join("declined.json");
        let done = apply_override_answer(
            OverrideAnswer::SaveAndReplace,
            &stale,
            home.path(),
            "t",
            &declined,
        )
        .unwrap();
        assert!(done.contains("Saved your copies of weex"), "{done}");
        assert!(
            home.path()
                .join("venues-backup-t")
                .join("weex.yaml")
                .exists()
        );
        let after = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        assert_eq!(after.source("weex"), Some(VenueSource::Catalog));
        assert!(after.get("weex").unwrap().has_ohlc());

        // Overwrite: removed, no backup.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let stale = stale_copies(&registry, &catalog_dir, &Default::default());
        apply_override_answer(OverrideAnswer::Replace, &stale, home.path(), "t", &declined)
            .unwrap();
        assert!(!user.join("weex.yaml").exists());
        assert!(!home.path().join("venues-backup-t").exists());

        // No: kept, and the answer recorded for this catalog version.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let stale = stale_copies(&registry, &catalog_dir, &Default::default());
        let declined = home.path().join("state").join("declined.json");
        let done = apply_override_answer(OverrideAnswer::Keep, &stale, home.path(), "t", &declined)
            .unwrap();
        assert!(
            done.contains("ask again when the catalog changes"),
            "{done}"
        );
        assert!(user.join("weex.yaml").exists());
        assert!(stale_copies(&registry, &catalog_dir, &read_declined(&declined)).is_empty());
    }

    /// Answers the TUI's next list question with `choice`.
    fn answer_with(asker: crate::cli::output::Asker, choice: Option<usize>) {
        tokio::spawn(async move {
            loop {
                let ask = asker.lock().unwrap().take();
                if let Some(ask) = ask {
                    assert!(
                        ask.prompt
                            .contains("New version of the venues catalog found")
                    );
                    assert_eq!(ask.options.len(), 3);
                    let _ = ask.reply.send(choice);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });
    }

    #[tokio::test]
    async fn test_the_tui_offer_saves_and_replaces_on_yes() {
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let declined = home.path().join("declined.json");
        let asker: crate::cli::output::Asker = Default::default();
        let (out, cap) = Output::capture();
        let out = out.with_asker(asker.clone());
        answer_with(asker, Some(0));
        assert!(offer_with(&registry, &catalog_dir, &declined, home.path(), &out).await);
        assert!(
            cap.err().contains("Saved your copies of weex"),
            "{}",
            cap.err()
        );
        assert!(!user.join("weex.yaml").exists());
    }

    #[tokio::test]
    async fn test_the_tui_offer_overwrites_keeps_or_cancels() {
        // Overwrite.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let declined = home.path().join("declined.json");
        let asker: crate::cli::output::Asker = Default::default();
        let (out, cap) = Output::capture();
        let out = out.with_asker(asker.clone());
        answer_with(asker, Some(2));
        assert!(offer_with(&registry, &catalog_dir, &declined, home.path(), &out).await);
        assert!(
            cap.err().contains("Removed your copies of weex"),
            "{}",
            cap.err()
        );

        // No: kept and recorded.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let declined = home.path().join("declined.json");
        let asker: crate::cli::output::Asker = Default::default();
        let (out, _) = Output::capture();
        let out = out.with_asker(asker.clone());
        answer_with(asker, Some(1));
        assert!(offer_with(&registry, &catalog_dir, &declined, home.path(), &out).await);
        assert!(user.join("weex.yaml").exists() && declined.exists());

        // Cancelled: nothing changes.
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let declined = home.path().join("declined.json");
        let asker: crate::cli::output::Asker = Default::default();
        let (out, cap) = Output::capture();
        let out = out.with_asker(asker.clone());
        answer_with(asker, None);
        assert!(offer_with(&registry, &catalog_dir, &declined, home.path(), &out).await);
        assert!(cap.err().is_empty(), "{}", cap.err());
        assert!(user.join("weex.yaml").exists() && !declined.exists());
    }

    #[tokio::test]
    async fn test_no_offer_without_stale_copies_or_a_way_to_ask() {
        let (home, catalog_dir, user) = stale_setup();
        let declined = home.path().join("declined.json");
        // Nothing stale: the catalog alone.
        let clean = VenueRegistry::load_dirs(&catalog_dir, &home.path().join("none")).unwrap();
        let (out, _) = Output::capture();
        assert!(!offer_with(&clean, &catalog_dir, &declined, home.path(), &out).await);
        // Stale, but no TUI and no terminal: not asked, nothing changes.
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        assert!(!offer_with(&registry, &catalog_dir, &declined, home.path(), &out).await);
        assert!(user.join("weex.yaml").exists());
        // A scripted run returns before reading anything.
        offer_catalog_updates(true, &out).await;
    }

    #[test]
    fn test_a_failed_move_or_removal_is_an_error_not_a_panic() {
        let (home, catalog_dir, user) = stale_setup();
        let registry = VenueRegistry::load_dirs(&catalog_dir, &user).unwrap();
        let stale = stale_copies(&registry, &catalog_dir, &Default::default());
        std::fs::remove_file(user.join("weex.yaml")).unwrap();
        let declined = home.path().join("declined.json");
        for answer in [OverrideAnswer::SaveAndReplace, OverrideAnswer::Replace] {
            let err = apply_override_answer(answer, &stale, home.path(), "t", &declined)
                .unwrap_err()
                .to_string();
            assert!(err.contains("weex.yaml"), "{err}");
        }
        // The answers file cannot be written where a folder is.
        std::fs::create_dir_all(&declined).unwrap();
        assert!(
            apply_override_answer(OverrideAnswer::Keep, &stale, home.path(), "t", &declined)
                .is_err()
        );
    }
}
