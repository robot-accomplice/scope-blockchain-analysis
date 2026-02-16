//! `scope venues` subcommands for managing venue descriptors.
//!
//! Provides venue discovery, schema documentation, initialisation of user
//! venue directories, and YAML validation.

use crate::display::terminal::{
    check_fail, check_pass, kv_row, section_footer, section_header, separator,
};
use crate::error::Result;
use crate::market::{VenueDescriptor, VenueRegistry};
use clap::{Args, Subcommand};

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
    /// Shows built-in and user-defined venues with their supported
    /// API capabilities (order_book, ticker, trades).
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

    /// Initialise the user venues directory with built-in descriptors.
    ///
    /// Copies all built-in venue YAML files to ~/.config/scope/venues/
    /// so you can customise them or use them as templates for new venues.
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
    /// scope venues validate ~/.config/scope/venues/custom.yaml
    /// ```
    Validate(ValidateArgs),
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
  scope venues validate ~/.config/scope/venues/custom.yaml")]
pub struct ValidateArgs {
    /// Path to the YAML file to validate.
    pub file: std::path::PathBuf,
}

/// Run the venues command.
pub fn run(cmd: VenuesCommands) -> Result<()> {
    match cmd {
        VenuesCommands::List(args) => run_list(args),
        VenuesCommands::Schema(args) => run_schema(args),
        VenuesCommands::Init(args) => run_init(args),
        VenuesCommands::Validate(args) => run_validate(args),
    }
}

// =============================================================================
// List
// =============================================================================

fn run_list(args: ListArgs) -> Result<()> {
    let registry = VenueRegistry::load()?;

    match args.format {
        ListFormat::Table => {
            println!("{}", section_header("Available Venues"));
            for id in registry.list() {
                if let Some(desc) = registry.get(id) {
                    let caps = desc.capability_names().join(", ");
                    println!("{}", kv_row(id, &caps));
                }
            }
            println!("{}", separator());
            let user_dir = VenueRegistry::user_venues_dir();
            let user_count = count_user_venues(&user_dir);
            let total = registry.len();
            let built_in = total - user_count;
            println!(
                "{}",
                kv_row(
                    "Loaded",
                    &format!(
                        "{} venues ({} built-in, {} user)",
                        total, built_in, user_count
                    )
                )
            );
            println!("{}", kv_row("User dir", &user_dir.display().to_string()));
            println!("{}", section_footer());
        }
        ListFormat::Json => {
            let venues: Vec<serde_json::Value> = registry
                .list()
                .iter()
                .filter_map(|id| {
                    registry.get(id).map(|desc| {
                        serde_json::json!({
                            "id": desc.id,
                            "name": desc.name,
                            "base_url": desc.base_url,
                            "capabilities": desc.capability_names(),
                        })
                    })
                })
                .collect();
            let output = serde_json::json!({
                "venues": venues,
                "total": registry.len(),
                "user_venues_dir": VenueRegistry::user_venues_dir().display().to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
    }

    Ok(())
}

/// Count YAML files in the user venues directory.
fn count_user_venues(dir: &std::path::Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "yaml" || ext == "yml")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

// =============================================================================
// Schema
// =============================================================================

fn run_schema(args: SchemaArgs) -> Result<()> {
    match args.format {
        SchemaFormat::Text => print_annotated_schema(),
        SchemaFormat::Json => print_json_schema(),
    }
    Ok(())
}

fn print_annotated_schema() {
    println!("{}", section_header("Venue Descriptor Schema"));
    println!(
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
    );
    println!("{}", section_footer());
}

fn print_json_schema() {
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

    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}

// =============================================================================
// Init
// =============================================================================

fn run_init(args: InitArgs) -> Result<()> {
    run_init_impl(args, VenueRegistry::user_venues_dir())
}

/// Core init logic with explicit destination path (used by tests).
fn run_init_impl(args: InitArgs, dest: std::path::PathBuf) -> Result<()> {
    // Ensure directory exists
    if !dest.exists() {
        std::fs::create_dir_all(&dest).map_err(|e| {
            crate::error::ScopeError::Chain(format!(
                "Failed to create venues directory {}: {}",
                dest.display(),
                e
            ))
        })?;
        println!("Created {}", dest.display());
    }

    // Get built-in venues to copy
    let registry = VenueRegistry::load()?;
    let mut copied = 0;
    let mut skipped = 0;

    for id in registry.list() {
        let filename = format!("{}.yaml", id);
        let target = dest.join(&filename);

        if target.exists() && !args.force {
            skipped += 1;
            println!("  skip {} (exists, use --force to overwrite)", filename);
            continue;
        }

        // Get the YAML content from the embedded built-in data
        if let Some(desc) = registry.get(id) {
            // Re-serialise the descriptor to YAML for the user's copy
            let yaml = format!(
                "# {name} venue descriptor\n# Auto-generated by `scope venues init`\n\n{content}",
                name = desc.name,
                content = serialize_descriptor_yaml(desc),
            );
            std::fs::write(&target, yaml).map_err(|e| {
                crate::error::ScopeError::Chain(format!(
                    "Failed to write {}: {}",
                    target.display(),
                    e
                ))
            })?;
            copied += 1;
            println!("  {}", check_pass(&filename));
        }
    }

    println!();
    println!("{}", section_header("Venues Init"));
    println!("{}", kv_row("Directory", &dest.display().to_string()));
    println!("{}", kv_row("Copied", &format!("{} files", copied)));
    if skipped > 0 {
        println!(
            "{}",
            kv_row("Skipped", &format!("{} files (already exist)", skipped))
        );
    }
    println!("{}", section_footer());

    Ok(())
}

/// Serialize a VenueDescriptor to a readable YAML string.
/// We use serde_yaml for this since VenueDescriptor doesn't derive Serialize;
/// instead, we manually build a representation.
fn serialize_descriptor_yaml(desc: &VenueDescriptor) -> String {
    // Build a YAML-compatible representation manually
    let mut lines = Vec::new();
    lines.push(format!("id: {}", desc.id));
    lines.push(format!("name: \"{}\"", desc.name));
    lines.push(format!("base_url: \"{}\"", desc.base_url));

    lines.push("symbol:".to_string());
    lines.push(format!("  template: \"{}\"", desc.symbol.template));
    lines.push(format!("  default_quote: {}", desc.symbol.default_quote));
    let case_str = match desc.symbol.case {
        crate::market::descriptor::SymbolCase::Upper => "upper",
        crate::market::descriptor::SymbolCase::Lower => "lower",
    };
    lines.push(format!("  case: {}", case_str));

    lines.push("capabilities:".to_string());

    if let Some(ref ep) = desc.capabilities.order_book {
        lines.push("  order_book:".to_string());
        append_endpoint_yaml(&mut lines, ep, "    ");
    }
    if let Some(ref ep) = desc.capabilities.ticker {
        lines.push("  ticker:".to_string());
        append_endpoint_yaml(&mut lines, ep, "    ");
    }
    if let Some(ref ep) = desc.capabilities.trades {
        lines.push("  trades:".to_string());
        append_endpoint_yaml(&mut lines, ep, "    ");
    }

    lines.join("\n")
}

fn append_endpoint_yaml(
    lines: &mut Vec<String>,
    ep: &crate::market::descriptor::EndpointDescriptor,
    indent: &str,
) {
    let method = match ep.method {
        crate::market::descriptor::HttpMethod::GET => "GET",
        crate::market::descriptor::HttpMethod::POST => "POST",
    };
    if method != "GET" {
        lines.push(format!("{indent}method: {method}"));
    }
    lines.push(format!("{indent}path: \"{}\"", ep.path));

    if !ep.params.is_empty() {
        lines.push(format!("{indent}params:"));
        let mut params: Vec<_> = ep.params.iter().collect();
        params.sort_by_key(|(k, _)| (*k).clone());
        for (k, v) in params {
            lines.push(format!("{indent}  {k}: \"{v}\""));
        }
    }

    if let Some(ref body) = ep.request_body {
        lines.push(format!(
            "{indent}request_body: {}",
            serde_json::to_string(body).unwrap_or_default()
        ));
    }

    if let Some(ref root) = ep.response_root {
        lines.push(format!("{indent}response_root: \"{}\"", root));
    }

    lines.push(format!("{indent}response:"));
    let r = &ep.response;
    let resp_indent = format!("{indent}  ");
    if let Some(ref v) = r.asks_key {
        lines.push(format!("{resp_indent}asks_key: {v}"));
    }
    if let Some(ref v) = r.bids_key {
        lines.push(format!("{resp_indent}bids_key: {v}"));
    }
    if let Some(ref v) = r.level_format {
        lines.push(format!("{resp_indent}level_format: {v}"));
    }
    if let Some(ref v) = r.level_price_field {
        lines.push(format!("{resp_indent}level_price_field: {v}"));
    }
    if let Some(ref v) = r.level_size_field {
        lines.push(format!("{resp_indent}level_size_field: {v}"));
    }
    if let Some(ref v) = r.last_price {
        lines.push(format!("{resp_indent}last_price: {v}"));
    }
    if let Some(ref v) = r.high_24h {
        lines.push(format!("{resp_indent}high_24h: {v}"));
    }
    if let Some(ref v) = r.low_24h {
        lines.push(format!("{resp_indent}low_24h: {v}"));
    }
    if let Some(ref v) = r.volume_24h {
        lines.push(format!("{resp_indent}volume_24h: {v}"));
    }
    if let Some(ref v) = r.quote_volume_24h {
        lines.push(format!("{resp_indent}quote_volume_24h: {v}"));
    }
    if let Some(ref v) = r.best_bid {
        lines.push(format!("{resp_indent}best_bid: {v}"));
    }
    if let Some(ref v) = r.best_ask {
        lines.push(format!("{resp_indent}best_ask: {v}"));
    }
    if let Some(ref v) = r.items_key {
        lines.push(format!("{resp_indent}items_key: {v}"));
    }
    if let Some(ref f) = r.filter {
        lines.push(format!("{resp_indent}filter:"));
        lines.push(format!("{resp_indent}  field: \"{}\"", f.field));
        lines.push(format!("{resp_indent}  value: \"{}\"", f.value));
    }
    if let Some(ref v) = r.price {
        lines.push(format!("{resp_indent}price: {v}"));
    }
    if let Some(ref v) = r.quantity {
        lines.push(format!("{resp_indent}quantity: {v}"));
    }
    if let Some(ref v) = r.quote_quantity {
        lines.push(format!("{resp_indent}quote_quantity: {v}"));
    }
    if let Some(ref v) = r.timestamp_ms {
        lines.push(format!("{resp_indent}timestamp_ms: {v}"));
    }
    if let Some(ref v) = r.id {
        lines.push(format!("{resp_indent}id: {v}"));
    }
    if let Some(ref sm) = r.side {
        lines.push(format!("{resp_indent}side:"));
        lines.push(format!("{resp_indent}  field: \"{}\"", sm.field));
        if !sm.mapping.is_empty() {
            lines.push(format!("{resp_indent}  mapping:"));
            let mut entries: Vec<_> = sm.mapping.iter().collect();
            entries.sort_by_key(|(k, _)| (*k).clone());
            for (k, v) in entries {
                lines.push(format!("{resp_indent}    \"{k}\": \"{v}\""));
            }
        }
    }
}

// =============================================================================
// Validate
// =============================================================================

fn run_validate(args: ValidateArgs) -> Result<()> {
    let path = &args.file;

    if !path.exists() {
        println!(
            "{}",
            check_fail(&format!("File not found: {}", path.display()))
        );
        return Err(crate::error::ScopeError::Chain(format!(
            "File not found: {}",
            path.display()
        )));
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        crate::error::ScopeError::Chain(format!("Failed to read {}: {}", path.display(), e))
    })?;

    println!("{}", section_header("Venue Validation"));
    println!("{}", kv_row("File", &path.display().to_string()));
    println!("{}", separator());

    match VenueRegistry::validate_yaml(&content) {
        Ok(desc) => {
            println!("{}", check_pass("Valid YAML syntax"));
            println!("{}", check_pass(&format!("Venue ID: {}", desc.id)));
            println!("{}", check_pass(&format!("Name: {}", desc.name)));
            println!("{}", check_pass(&format!("Base URL: {}", desc.base_url)));

            // Check capabilities
            let caps = desc.capability_names();
            if caps.is_empty() {
                println!(
                    "{}",
                    check_fail(
                        "No capabilities defined (need at least one of: order_book, ticker, trades)"
                    )
                );
            } else {
                for cap in &caps {
                    println!("{}", check_pass(&format!("Capability: {}", cap)));
                }
            }

            // Validate symbol template
            if desc.symbol.template.contains("{base}") {
                println!(
                    "{}",
                    check_pass("Symbol template contains {base} placeholder")
                );
            } else {
                println!(
                    "{}",
                    check_fail("Symbol template missing {base} placeholder")
                );
            }

            println!("{}", separator());
            if caps.is_empty() {
                println!("{}", check_fail("Validation completed with warnings"));
            } else {
                println!("{}", check_pass("Validation passed"));
            }
            println!("{}", section_footer());
            Ok(())
        }
        Err(e) => {
            println!("{}", check_fail("Invalid YAML"));
            println!("{}", check_fail(&format!("Error: {}", e)));
            println!("{}", separator());
            println!(
                "{}",
                kv_row(
                    "Hint",
                    "Run `scope venues schema` to see the expected format"
                )
            );
            println!("{}", section_footer());
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
        let result = run_list(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_json() {
        let args = ListArgs {
            format: ListFormat::Json,
        };
        let result = run_list(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_schema_text() {
        let args = SchemaArgs {
            format: SchemaFormat::Text,
        };
        let result = run_schema(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_schema_json() {
        let args = SchemaArgs {
            format: SchemaFormat::Json,
        };
        let result = run_schema(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_missing_file() {
        let args = ValidateArgs {
            file: std::path::PathBuf::from("/tmp/nonexistent_venue_test.yaml"),
        };
        let result = run_validate(args);
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
        let result = run_validate(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_invalid_file() {
        let yaml = "this is not valid yaml: [";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, yaml).unwrap();

        let args = ValidateArgs { file: path };
        let result = run_validate(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_count_user_venues_nonexistent() {
        let count = count_user_venues(std::path::Path::new("/tmp/nonexistent_dir_test"));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_count_user_venues_with_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.yaml"), "").unwrap();
        std::fs::write(dir.path().join("b.yml"), "").unwrap();
        std::fs::write(dir.path().join("c.txt"), "").unwrap();
        assert_eq!(count_user_venues(dir.path()), 2);
    }

    #[test]
    fn test_serialize_descriptor_yaml_roundtrip() {
        let yaml = r#"
id: roundtrip_test
name: Roundtrip Exchange
base_url: https://api.roundtrip.com
symbol:
  template: "{base}_{quote}"
  default_quote: USDT
  case: lower
capabilities:
  order_book:
    path: /api/depth
    params:
      symbol: "{pair}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        let serialized = serialize_descriptor_yaml(&desc);
        assert!(serialized.contains("roundtrip_test"));
        assert!(serialized.contains("Roundtrip Exchange"));
        assert!(serialized.contains("/api/depth"));
        assert!(serialized.contains("case: lower"));
    }

    #[test]
    fn test_run_init_to_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().to_path_buf();
        let args = InitArgs { force: true };
        let result = run_init_impl(args, dest.clone());
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
    fn test_serialize_full_descriptor() {
        let yaml = r#"
id: full_caps
name: Full Capabilities Exchange
base_url: https://api.full.com
symbol:
  template: "{base}{quote}"
  default_quote: USDT
capabilities:
  order_book:
    path: /api/depth
    params:
      symbol: "{pair}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
  ticker:
    path: /api/ticker
    params:
      symbol: "{pair}"
    response:
      last_price: lastPrice
      high_24h: high
      low_24h: low
  trades:
    path: /api/trades
    params:
      symbol: "{pair}"
      limit: "{limit}"
    response:
      items_key: data
      price: price
      quantity: qty
      timestamp_ms: time
      side:
        field: side
        mapping:
          buy: buy
          sell: sell
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        let serialized = serialize_descriptor_yaml(&desc);
        assert!(serialized.contains("order_book:"));
        assert!(serialized.contains("ticker:"));
        assert!(serialized.contains("trades:"));
        assert!(serialized.contains("asks_key"));
        assert!(serialized.contains("last_price"));
        assert!(serialized.contains("items_key"));
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
        let result = run_init_impl(args, dest.clone());
        assert!(result.is_ok());

        // Verify binance.yaml was NOT overwritten (content unchanged)
        let content = std::fs::read_to_string(&existing_path).unwrap();
        assert_eq!(
            content, original_content,
            "Existing file should not be overwritten when force=false"
        );
    }
}
