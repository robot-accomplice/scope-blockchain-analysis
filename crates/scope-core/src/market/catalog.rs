//! The venue catalog: the venue descriptors kept in the repo
//! (`crates/scope-core/venues/`), synced into the scope home by
//! `scope venues update`. Nothing is compiled in, so a new or fixed venue
//! reaches users without a release.
//!
//! The catalog is an `index.json` (schema, and each venue's file and
//! SHA-256) and one YAML file per venue. A sync fetches the index, then
//! every file; it checks each against its checksum and its id, writes them
//! to a new folder and swaps it in, so a failed sync leaves the old catalog
//! as it was. Checksums are over the text with `\n` line ends, so a Windows
//! checkout (`\r\n`) and the served files agree.

use crate::error::{Result, ScopeError};
use crate::http::{HttpClient, Request};
use crate::market::descriptor::VenueDescriptor;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Where `scope venues update` fetches the catalog from, unless the config
/// says otherwise (`venues.catalog_url`).
pub const DEFAULT_CATALOG_URL: &str = "https://raw.githubusercontent.com/robot-accomplice/scope-blockchain-analysis/main/crates/scope-core/venues";

/// The index file of a catalog.
pub const INDEX_FILE: &str = "index.json";

/// The newest index schema this scope reads.
pub const SCHEMA: u32 = 1;

/// The file the user copies to describe an exchange of their own.
pub const EXAMPLE_FILE: &str = "example.yaml.sample";

/// The catalog's table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub venues: Vec<Entry>,
}

/// One venue of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub file: String,
    pub sha256: String,
}

/// What a sync changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
}

impl SyncReport {
    /// `12 venues: 1 added (weex), 0 updated, 0 removed`.
    pub fn summary(&self) -> String {
        let total = self.added.len() + self.updated.len() + self.unchanged;
        let list = |ids: &[String]| {
            if ids.is_empty() {
                String::new()
            } else {
                format!(" ({})", ids.join(", "))
            }
        };
        format!(
            "{total} venues: {} added{}, {} updated{}, {} removed{}",
            self.added.len(),
            list(&self.added),
            self.updated.len(),
            list(&self.updated),
            self.removed.len(),
            list(&self.removed)
        )
    }
}

/// The SHA-256 of `text` with `\n` line ends, in lowercase hex.
pub fn checksum(text: &str) -> String {
    hex::encode(Sha256::digest(text.replace("\r\n", "\n").as_bytes()))
}

/// The index of the descriptors in `dir` (`*.yaml`), sorted by file name.
/// Used to write the repo's `index.json` and to check it.
pub fn build_index(dir: &Path) -> Result<Index> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| ScopeError::io_context(e, format!("Cannot read {}", dir.display())))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    files.sort();
    let mut venues = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ScopeError::io_context(e, format!("Cannot read {}", path.display())))?;
        let desc = parse(&text, &path.display().to_string())?;
        venues.push(Entry {
            id: desc.id,
            file: path.file_name().unwrap().to_string_lossy().into_owned(),
            sha256: checksum(&text),
        });
    }
    Ok(Index {
        schema: SCHEMA,
        venues,
    })
}

/// `text` as a descriptor, or an error naming `what`.
fn parse(text: &str, what: &str) -> Result<VenueDescriptor> {
    serde_saphyr::from_str(text)
        .map_err(|e| ScopeError::InvalidInput(format!("Venue file {what} does not parse: {e}")))
}

/// A catalog file name: a plain `*.yaml` name, nothing that leaves the
/// catalog folder.
fn safe_file_name(file: &str) -> bool {
    file.ends_with(".yaml")
        && !file.starts_with('.')
        && file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Fetches `url` and returns its body, or an error that names it.
async fn fetch(http: &dyn HttpClient, url: &str) -> Result<String> {
    let response = http.send(Request::get(url)).await?;
    if !response.is_success() {
        return Err(ScopeError::Network(format!(
            "{url} answered HTTP {}",
            response.status_code
        )));
    }
    Ok(response.body)
}

/// Syncs the catalog at `base_url` into `dest`: fetches the index and every
/// venue file, checks each, and swaps the new catalog in. On any error the
/// old catalog stays as it was.
pub async fn sync(http: &dyn HttpClient, base_url: &str, dest: &Path) -> Result<SyncReport> {
    let base = base_url.trim_end_matches('/');
    let index: Index = serde_json::from_str(&fetch(http, &format!("{base}/{INDEX_FILE}")).await?)
        .map_err(|e| {
        ScopeError::InvalidInput(format!("The venue catalog index does not parse: {e}"))
    })?;
    if index.schema > SCHEMA {
        return Err(ScopeError::InvalidInput(format!(
            "The venue catalog needs a newer scope (catalog schema {}, this scope reads {SCHEMA}). Update scope, then run `scope venues update` again.",
            index.schema
        )));
    }

    let mut files = Vec::with_capacity(index.venues.len());
    for entry in &index.venues {
        if !safe_file_name(&entry.file) {
            return Err(ScopeError::InvalidInput(format!(
                "The venue catalog names an unsafe file: {:?}",
                entry.file
            )));
        }
        let text = fetch(http, &format!("{base}/{}", entry.file)).await?;
        let got = checksum(&text);
        if got != entry.sha256 {
            return Err(ScopeError::InvalidInput(format!(
                "Venue file {} does not match the catalog index (sha256 {got}, expected {}); the catalog was not changed.",
                entry.file, entry.sha256
            )));
        }
        let desc = parse(&text, &entry.file)?;
        if desc.id != entry.id {
            return Err(ScopeError::InvalidInput(format!(
                "Venue file {} has id {:?}; the catalog index says {:?}.",
                entry.file, desc.id, entry.id
            )));
        }
        files.push((entry, text));
    }

    let old = installed(dest);
    let parent = dest.parent().unwrap_or(dest);
    std::fs::create_dir_all(parent)
        .map_err(|e| ScopeError::io_context(e, format!("Cannot create {}", parent.display())))?;
    let staging = parent.join("catalog.new");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| ScopeError::io_context(e, format!("Cannot create {}", staging.display())))?;
    for (entry, text) in &files {
        let path = staging.join(&entry.file);
        std::fs::write(&path, text)
            .map_err(|e| ScopeError::io_context(e, format!("Cannot write {}", path.display())))?;
    }
    let index_json = serde_json::to_string_pretty(&index).unwrap_or_default();
    std::fs::write(staging.join(INDEX_FILE), index_json + "\n")
        .map_err(|e| ScopeError::io_context(e, "Cannot write the catalog index".to_string()))?;
    let retired = parent.join("catalog.old");
    let _ = std::fs::remove_dir_all(&retired);
    if dest.exists() {
        std::fs::rename(dest, &retired)
            .map_err(|e| ScopeError::io_context(e, format!("Cannot replace {}", dest.display())))?;
    }
    if let Err(e) = std::fs::rename(&staging, dest) {
        // Put the old catalog back before reporting.
        let _ = std::fs::rename(&retired, dest);
        return Err(ScopeError::io_context(
            e,
            format!("Cannot install the catalog in {}", dest.display()),
        ));
    }
    let _ = std::fs::remove_dir_all(&retired);

    let mut report = SyncReport::default();
    for entry in &index.venues {
        match old.iter().find(|o| o.id == entry.id) {
            None => report.added.push(entry.id.clone()),
            Some(o) if o.sha256 != entry.sha256 => report.updated.push(entry.id.clone()),
            Some(_) => report.unchanged += 1,
        }
    }
    report.removed = old
        .iter()
        .filter(|o| !index.venues.iter().any(|e| e.id == o.id))
        .map(|o| o.id.clone())
        .collect();
    Ok(report)
}

/// The entries of the catalog installed in `dir` (from its index), or none.
pub fn installed(dir: &Path) -> Vec<Entry> {
    std::fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Index>(&text).ok())
        .map(|index| index.venues)
        .unwrap_or_default()
}

/// Where the example venue file goes: the user's venues folder.
pub fn example_path() -> PathBuf {
    crate::paths::venues_dir().join(EXAMPLE_FILE)
}

/// Writes [`EXAMPLE`] to `path` unless a file is there already.
pub fn write_example(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, EXAMPLE)
}

/// A commented venue descriptor: how to describe an exchange by hand. It is
/// not a venue (scope reads `*.yaml` and `*.yml`, not `*.sample`).
pub const EXAMPLE: &str = r#"# How to add an exchange to scope by hand
# =========================================
#
# Copy this file to a name ending in .yaml in this folder (for example
# myexchange.yaml), fill it in, and check it with:
#
#     scope venues validate ~/.scope-bca/venues/myexchange.yaml
#
# Files in this folder override a catalog venue with the same `id`. The
# catalog itself lives in the `catalog/` folder and is refreshed by
# `scope venues update`; do not edit it, your changes there are replaced.
#
# Every path below is the exchange's public REST API: no keys are needed.
# `{pair}` is the venue's symbol for the pair (see `symbol`), `{limit}` the
# number of items asked for, `{interval}` a candle interval such as 1h.

# A short, lowercase name: what `--venue` takes.
id: myexchange
# The name shown in listings.
name: My Exchange Spot
# The API's root URL, without a trailing slash.
base_url: https://api.myexchange.example
timeout_secs: 15
rate_limit_per_sec: 10

# How the pair is written in the exchange's URLs: BTC/USDT becomes BTCUSDT
# with "{base}{quote}", BTC_USDT with "{base}_{quote}", BTC-USDT with
# "{base}-{quote}". `default_quote` is used when only the base is given.
symbol:
  template: "{base}{quote}"
  default_quote: USDT

capabilities:
  # The order book. `level_format: positional` means each level is
  # ["price", "quantity"]; use `object` with `level_price` and
  # `level_size` when levels are {"price": ..., "size": ...}.
  order_book:
    path: /api/v3/depth
    params:
      symbol: "{pair}"
      limit: "100"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional

  # The 24-hour ticker: map each field to the key the exchange uses.
  ticker:
    path: /api/v3/ticker/24hr
    params:
      symbol: "{pair}"
    response:
      last_price: lastPrice
      high_24h: highPrice
      low_24h: lowPrice
      volume_24h: volume
      quote_volume_24h: quoteVolume
      best_bid: bidPrice
      best_ask: askPrice

  # Recent trades, for the bot-pattern advisory. `side` maps the
  # exchange's value to buy or sell.
  trades:
    path: /api/v3/trades
    params:
      symbol: "{pair}"
      limit: "{limit}"
    response:
      price: price
      quantity: qty
      quote_quantity: quoteQty
      timestamp_ms: time
      id: id
      side:
        field: isBuyerMaker
        mapping:
          "true": sell
          "false": buy

  # Candles, for `scope market ohlc`. `array_of_arrays` reads rows such as
  # [open_time, open, high, low, close, volume, close_time].
  ohlc:
    path: /api/v3/klines
    params:
      symbol: "{pair}"
      interval: "{interval}"
      limit: "{limit}"
    response:
      ohlc_format: array_of_arrays
      ohlc_fields: [open_time, open, high, low, close, volume, close_time]

# The venues in catalog/ are complete, working examples of other API shapes
# (nested data keys, object levels, other symbol formats): copy the one
# closest to your exchange.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Response;
    use std::sync::Mutex;

    /// The repo catalog folder.
    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("venues")
    }

    #[test]
    fn test_the_repo_index_matches_the_venue_files() {
        // A venue file changed without `index.json`? Regenerate it:
        //   SCOPE_UPDATE_VENUE_INDEX=1 cargo test -p scope-bca-core repo_index
        let built = build_index(&repo()).unwrap();
        let path = repo().join(INDEX_FILE);
        if std::env::var_os("SCOPE_UPDATE_VENUE_INDEX").is_some() {
            let json = serde_json::to_string_pretty(&built).unwrap() + "\n";
            std::fs::write(&path, json).unwrap();
        }
        let on_disk: Index =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk, built,
            "crates/scope-core/venues/index.json is stale; run with SCOPE_UPDATE_VENUE_INDEX=1 to rewrite it"
        );
        assert!(built.venues.len() >= 11);
    }

    #[test]
    fn test_checksums_ignore_windows_line_ends() {
        assert_eq!(
            checksum("id: a\nname: b\n"),
            checksum("id: a\r\nname: b\r\n")
        );
        assert_ne!(checksum("id: a\n"), checksum("id: b\n"));
    }

    #[test]
    fn test_the_example_is_a_valid_descriptor_that_is_not_loaded() {
        let desc: VenueDescriptor = serde_saphyr::from_str(EXAMPLE).unwrap();
        assert_eq!(desc.id, "myexchange");
        assert_eq!(desc.capability_names().len(), 4);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EXAMPLE_FILE);
        write_example(&path).unwrap();
        // Not overwritten once the user has it.
        std::fs::write(&path, "mine").unwrap();
        write_example(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        let registry =
            crate::market::VenueRegistry::load_dirs(&dir.path().join("none"), dir.path()).unwrap();
        assert!(registry.is_empty(), "a .sample file is not a venue");
    }

    #[test]
    fn test_file_names_stay_in_the_catalog() {
        assert!(safe_file_name("weex.yaml"));
        assert!(safe_file_name("crypto_com.yaml"));
        for bad in [
            "../x.yaml",
            "a/b.yaml",
            "..\\x.yaml",
            ".hidden.yaml",
            "x.yml",
            "x.sh",
        ] {
            assert!(!safe_file_name(bad), "{bad}");
        }
    }

    /// A catalog server in memory: path → (status, body).
    struct Served(Mutex<Vec<(String, u16, String)>>);

    #[async_trait::async_trait]
    impl HttpClient for Served {
        async fn send(&self, request: Request) -> std::result::Result<Response, ScopeError> {
            let routes = self.0.lock().unwrap();
            let (_, status, body) = routes
                .iter()
                .find(|(path, _, _)| request.url.ends_with(path.as_str()))
                .cloned()
                .unwrap_or((String::new(), 404, String::new()));
            Ok(Response {
                status_code: status,
                headers: Default::default(),
                body,
            })
        }
    }

    fn venue(id: &str, name: &str) -> String {
        format!(
            "id: {id}\nname: {name}\nbase_url: https://{id}.example\nsymbol:\n  template: \"{{base}}{{quote}}\"\n  default_quote: USDT\ncapabilities:\n  ticker:\n    path: /t\n    response:\n      last_price: p\n"
        )
    }

    fn served(venues: &[(&str, &str, &str)]) -> Served {
        let index = Index {
            schema: SCHEMA,
            venues: venues
                .iter()
                .map(|(id, file, body)| Entry {
                    id: id.to_string(),
                    file: file.to_string(),
                    sha256: checksum(body),
                })
                .collect(),
        };
        let mut routes = vec![(
            format!("/{INDEX_FILE}"),
            200,
            serde_json::to_string(&index).unwrap(),
        )];
        routes.extend(
            venues
                .iter()
                .map(|(_, file, body)| (format!("/{file}"), 200, body.to_string())),
        );
        Served(Mutex::new(routes))
    }

    #[tokio::test]
    async fn test_a_sync_installs_reports_and_replaces_the_catalog() {
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("venues").join("catalog");
        let a = venue("alpha", "Alpha");
        let b = venue("beta", "Beta");
        let report = sync(
            &served(&[("alpha", "alpha.yaml", &a), ("beta", "beta.yaml", &b)]),
            "https://x/c",
            &dest,
        )
        .await
        .unwrap();
        assert_eq!(report.added, ["alpha", "beta"]);
        assert_eq!(
            report.summary(),
            "2 venues: 2 added (alpha, beta), 0 updated, 0 removed"
        );
        let registry =
            crate::market::VenueRegistry::load_dirs(&dest, &home.path().join("u")).unwrap();
        assert_eq!(registry.list(), ["alpha", "beta"]);

        // Beta changes, alpha goes, gamma comes.
        let b2 = venue("beta", "Beta 2");
        let c = venue("gamma", "Gamma");
        let report = sync(
            &served(&[("beta", "beta.yaml", &b2), ("gamma", "gamma.yaml", &c)]),
            "https://x/c/",
            &dest,
        )
        .await
        .unwrap();
        assert_eq!(report.added, ["gamma"]);
        assert_eq!(report.updated, ["beta"]);
        assert_eq!(report.removed, ["alpha"]);
        assert!(!dest.join("alpha.yaml").exists());
        assert!(!home.path().join("venues").join("catalog.new").exists());
        assert!(!home.path().join("venues").join("catalog.old").exists());
    }

    #[tokio::test]
    async fn test_a_bad_file_leaves_the_old_catalog() {
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("catalog");
        let a = venue("alpha", "Alpha");
        sync(&served(&[("alpha", "alpha.yaml", &a)]), "https://x", &dest)
            .await
            .unwrap();

        // The file served is not the one the index hashed.
        let tampered = served(&[("alpha", "alpha.yaml", &a)]);
        tampered.0.lock().unwrap()[1].2 = venue("alpha", "Evil");
        let err = sync(&tampered, "https://x", &dest)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match the catalog index"), "{err}");
        // An id that is not the index's.
        let err = sync(&served(&[("beta", "alpha.yaml", &a)]), "https://x", &dest)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("the catalog index says"), "{err}");
        // A path out of the folder.
        let err = sync(
            &served(&[("alpha", "../alpha.yaml", &a)]),
            "https://x",
            &dest,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("unsafe file"), "{err}");
        // A newer schema.
        let newer = served(&[]);
        newer.0.lock().unwrap()[0].2 = r#"{"schema": 99, "venues": []}"#.to_string();
        let err = sync(&newer, "https://x", &dest)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("needs a newer scope"), "{err}");
        // Offline: the index cannot be fetched.
        let err = sync(&Served(Mutex::new(vec![])), "https://x", &dest)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("HTTP 404"), "{err}");

        // Through all of it, the first catalog stays.
        assert_eq!(installed(&dest).len(), 1);
        assert_eq!(std::fs::read_to_string(dest.join("alpha.yaml")).unwrap(), a);
    }
}
