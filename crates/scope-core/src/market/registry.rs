//! Venue registry: loads and manages venue descriptors.
//!
//! No venue is compiled in. The registry reads the venue catalog that
//! `scope venues update` keeps in `~/.scope-bca/venues/catalog/`
//! ([`crate::market::catalog`]), then the user's own descriptors in
//! `~/.scope-bca/venues/*.yaml`, which override a catalog venue with the
//! same `id`.

use crate::error::{Result, ScopeError};
use crate::market::descriptor::VenueDescriptor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a venue's descriptor came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueSource {
    /// The venue catalog, synced by `scope venues update`.
    Catalog,
    /// A file of the user's in `~/.scope-bca/venues/`.
    User,
}

// =============================================================================
// VenueRegistry
// =============================================================================

/// Registry of the venue descriptors: the catalog's, then the user's.
#[derive(Debug, Clone, Default)]
pub struct VenueRegistry {
    venues: HashMap<String, VenueDescriptor>,
    sources: HashMap<String, VenueSource>,
}

impl VenueRegistry {
    /// Load the catalog and the user's venues from the scope home.
    ///
    /// User venues with the same `id` as a catalog venue override it, so
    /// endpoints can be customized without waiting for a catalog update.
    pub fn load() -> Result<Self> {
        Self::load_dirs(&Self::catalog_dir(), &Self::user_venues_dir())
    }

    /// The venues in `catalog`, then those in `user` (which win). A missing
    /// folder adds nothing; a file that cannot be read or parsed is skipped
    /// with a warning.
    pub fn load_dirs(catalog: &Path, user: &Path) -> Result<Self> {
        let mut registry = Self::default();
        registry.read_dir(catalog, VenueSource::Catalog);
        registry.read_dir(user, VenueSource::User);
        Ok(registry)
    }

    /// Adds the `*.yaml` / `*.yml` descriptors in `dir` as `source`.
    fn read_dir(&mut self, dir: &Path, source: VenueSource) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml")
            })
            .collect();
        // A fixed order, so two files with one id resolve the same way.
        paths.sort();
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(contents) => match serde_saphyr::from_str::<VenueDescriptor>(&contents) {
                    Ok(desc) => {
                        self.sources.insert(desc.id.clone(), source);
                        self.venues.insert(desc.id.clone(), desc);
                    }
                    Err(e) => {
                        crate::notice!(
                            "Warning: failed to parse venue file {}: {}",
                            path.display(),
                            e
                        );
                    }
                },
                Err(e) => {
                    crate::notice!(
                        "Warning: failed to read venue file {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    /// Where `id`'s descriptor came from, if it is loaded.
    pub fn source(&self, id: &str) -> Option<VenueSource> {
        self.sources.get(id).copied()
    }

    /// List all available venue IDs, sorted alphabetically.
    pub fn list(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.venues.keys().map(String::as_str).collect();
        ids.sort();
        ids
    }

    /// Get a venue descriptor by ID.
    pub fn get(&self, id: &str) -> Option<&VenueDescriptor> {
        self.venues.get(id)
    }

    /// Number of loaded venues.
    pub fn len(&self) -> usize {
        self.venues.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.venues.is_empty()
    }

    /// Check if a venue ID exists in the registry.
    pub fn contains(&self, id: &str) -> bool {
        self.venues.contains_key(id)
    }

    /// Find the closest venue name to a given input (for typo suggestions).
    pub fn suggest(&self, input: &str) -> Option<&str> {
        closest(input, self.list())
    }

    /// Path to the user venues directory (`~/.scope-bca/venues/`).
    pub fn user_venues_dir() -> PathBuf {
        crate::paths::venues_dir()
    }

    /// Path to the synced catalog (`~/.scope-bca/venues/catalog/`).
    pub fn catalog_dir() -> PathBuf {
        crate::paths::venues_dir().join("catalog")
    }

    /// Create an ExchangeClient for the given venue ID.
    ///
    /// An unknown ID is [`ScopeError::InvalidInput`] that lists the registry
    /// venues and names the closest one. DEX venues are not listed: this
    /// lookup cannot serve them (see `MarketVenue::resolve`).
    #[deprecated(
        since = "0.8.0",
        note = "pass the transport: use `VenueRegistry::create_exchange_client_with_http`"
    )]
    pub fn create_exchange_client(
        &self,
        venue_id: &str,
    ) -> Result<crate::market::exchange::ExchangeClient> {
        self.create_exchange_client_with_http(venue_id, crate::http::shared::fallback())
    }

    /// [`VenueRegistry::create_exchange_client`], sending the client's
    /// requests through `http`.
    pub fn create_exchange_client_with_http(
        &self,
        venue_id: &str,
        http: std::sync::Arc<dyn crate::http::HttpClient>,
    ) -> Result<crate::market::exchange::ExchangeClient> {
        let desc = self
            .get(venue_id)
            .ok_or_else(|| unknown_venue(venue_id, &self.list()))?;
        Ok(crate::market::exchange::ExchangeClient::from_descriptor_with_http(desc, http))
    }

    /// Validate a venue descriptor YAML string.
    pub fn validate_yaml(yaml: &str) -> Result<VenueDescriptor> {
        serde_saphyr::from_str(yaml)
            .map_err(|e| ScopeError::Config(crate::error::ConfigError::Parse { source: e }))
    }
}

/// The unknown-venue error: names the closest of `ids` and lists them all.
/// Each lookup passes the ids it can serve, so it never suggests a venue it
/// would reject.
pub(crate) fn unknown_venue<S: AsRef<str>>(input: &str, ids: &[S]) -> ScopeError {
    let ids: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
    if ids
        .iter()
        .all(|id| crate::market::venue::DexVenue::parse(id).is_some())
    {
        return no_venues(input);
    }
    let available = format!(
        "\n\n  Available venues: {}\n  Run `scope venues list` for details.",
        ids.join(", ")
    );
    let hint = match closest(input, ids) {
        Some(s) => format!("\n\n  Did you mean \"{s}\"?{available}"),
        None => available,
    };
    ScopeError::InvalidInput(format!("Unknown venue \"{input}\"{hint}"))
}

/// The error for a venue when no exchange venue is loaded: how to get the
/// catalog, and how to add one by hand.
pub(crate) fn no_venues(input: &str) -> ScopeError {
    ScopeError::InvalidInput(format!(
        "Unknown venue \"{input}\": no exchange venues are set up yet.\n\n  \
         Run `scope venues update` to fetch the venue catalog (needs the network),\n  \
         or describe an exchange yourself: see {}",
        crate::market::catalog::example_path().display()
    ))
}

/// The id nearest to `input`, ignoring case, within one edit for each
/// [`SUGGEST_LENGTH_PER_EDIT`] characters of the longer name.
fn closest<'a>(input: &str, ids: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let input_lower = input.to_lowercase();
    ids.into_iter()
        .map(|k| (k, strsim_distance(&input_lower, &k.to_lowercase())))
        // A fixed limit suggested `okx` for `weex` (#69). Scale it to the name.
        .filter(|(k, dist)| {
            *dist * SUGGEST_LENGTH_PER_EDIT <= input_lower.chars().count().max(k.len())
        })
        .min_by_key(|(_, dist)| *dist)
        .map(|(k, _)| k)
}

/// A suggestion may differ by one edit for each this many characters of the
/// longer name.
const SUGGEST_LENGTH_PER_EDIT: usize = 3;

/// Simple Levenshtein distance for typo suggestions.
fn strsim_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate().take(n + 1) {
        *val = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo's catalog, as a synced one.
    fn catalog() -> VenueRegistry {
        crate::market::testing::catalog()
    }

    #[test]
    fn test_nothing_is_compiled_in() {
        assert!(VenueRegistry::default().is_empty());
        let dir = tempfile::tempdir().unwrap();
        let none = VenueRegistry::load_dirs(&dir.path().join("a"), &dir.path().join("b")).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_user_venue_overrides_the_catalog_and_bad_files_are_skipped() {
        let catalog_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("venues");
        let user = tempfile::tempdir().unwrap();
        let binance = std::fs::read_to_string(catalog_dir.join("binance.yaml"))
            .unwrap()
            .replace("name: Binance Spot", "name: My Binance");
        std::fs::write(user.path().join("binance.yaml"), binance).unwrap();
        std::fs::write(user.path().join("broken.yml"), "id: [unclosed").unwrap();
        std::fs::write(user.path().join("notes.txt"), "not a venue").unwrap();
        std::fs::write(user.path().join("example.yaml.sample"), "id: sample").unwrap();

        let registry = VenueRegistry::load_dirs(&catalog_dir, user.path()).unwrap();
        assert_eq!(registry.len(), catalog().len());
        assert_eq!(registry.get("binance").unwrap().name, "My Binance");
        assert_eq!(registry.source("binance"), Some(VenueSource::User));
        assert_eq!(registry.source("kraken"), Some(VenueSource::Catalog));
        assert!(!registry.contains("sample"));
    }

    #[test]
    fn test_the_catalog_has_the_known_venues() {
        let registry = catalog();
        for id in [
            "binance",
            "biconomy",
            "mexc",
            "bitget",
            "gateio",
            "bybit",
            "okx",
            "coinbase",
            "kraken",
            "htx",
            "crypto_com",
        ] {
            assert!(registry.contains(id), "{id} missing from the catalog");
        }
    }

    #[test]
    fn test_registry_list_sorted() {
        let list = catalog()
            .list()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut sorted = list.clone();
        sorted.sort();
        assert_eq!(list, sorted);
    }

    #[test]
    fn test_registry_get() {
        let registry = catalog();
        let binance = registry.get("binance").unwrap();
        assert_eq!(binance.name, "Binance Spot");
        assert_eq!(binance.base_url, "https://api.binance.com");
    }

    #[test]
    fn test_registry_suggest_typo() {
        let registry = catalog();
        assert_eq!(registry.suggest("binace"), Some("binance"));
        assert_eq!(registry.suggest("gate"), Some("gateio"));
        assert_eq!(registry.suggest("krakn"), Some("kraken"));
        assert_eq!(registry.suggest("bybi"), Some("bybit"));
        assert!(registry.suggest("zzzzzzzzz").is_none());
    }

    #[test]
    fn test_an_unknown_venue_with_no_venues_says_how_to_get_them() {
        let err = VenueRegistry::default()
            .create_exchange_client_with_http("binance", crate::http::test_transport())
            .unwrap_err()
            .to_string();
        assert!(err.contains("no exchange venues are set up yet"), "{err}");
        assert!(err.contains("scope venues update"), "{err}");
        assert!(err.contains("example.yaml.sample"), "{err}");
        // With venues, the usual suggestion.
        let err = catalog()
            .create_exchange_client_with_http("binace", crate::http::test_transport())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Did you mean \"binance\"?"), "{err}");
    }

    #[test]
    fn test_strsim_distance() {
        assert_eq!(strsim_distance("binance", "binance"), 0);
        assert_eq!(strsim_distance("binace", "binance"), 1);
        assert_eq!(strsim_distance("", "abc"), 3);
        assert_eq!(strsim_distance("abc", ""), 3);
        assert_eq!(strsim_distance("abc", "xyz"), 3);
        assert_eq!(strsim_distance("hello", "world"), 4);
    }

    #[test]
    fn test_validate_yaml() {
        let yaml = r#"
id: test
name: Test Exchange
base_url: https://example.com
symbol:
  template: "{base}{quote}"
  default_quote: USDT
"#;
        assert_eq!(VenueRegistry::validate_yaml(yaml).unwrap().id, "test");
        assert!(VenueRegistry::validate_yaml("not: valid: yaml: [[[").is_err());
    }

    #[test]
    fn test_venue_dirs_are_under_the_scope_home() {
        let user = VenueRegistry::user_venues_dir();
        assert!(user.ends_with("venues"), "{}", user.display());
        assert_eq!(VenueRegistry::catalog_dir(), user.join("catalog"));
    }

    #[test]
    fn test_every_catalog_venue_has_capabilities_and_formats_a_pair() {
        let registry = catalog();
        for id in registry.list() {
            let desc = registry.get(id).unwrap();
            assert!(
                !desc.capability_names().is_empty(),
                "{id} has no capabilities"
            );
            assert!(
                !desc.format_pair("BTC", None).is_empty(),
                "{id} formats no pair"
            );
        }
    }
}
