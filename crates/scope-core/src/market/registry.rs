//! Venue registry: loads and manages venue descriptors.
//!
//! Built-in descriptors are embedded at compile time via `include_str!`.
//! User descriptors in `~/.config/scope/venues/*.yaml` override built-in
//! venues with the same `id`.

use crate::error::{Result, ScopeError};
use crate::market::descriptor::VenueDescriptor;
use std::collections::HashMap;
use std::path::PathBuf;

// =============================================================================
// Built-in venue YAML files (embedded at compile time)
// =============================================================================

const BUILT_IN_VENUES: &[(&str, &str)] = &[
    ("binance", include_str!("../../venues/binance.yaml")),
    ("biconomy", include_str!("../../venues/biconomy.yaml")),
    ("mexc", include_str!("../../venues/mexc.yaml")),
    ("bitget", include_str!("../../venues/bitget.yaml")),
    ("gateio", include_str!("../../venues/gateio.yaml")),
    ("bybit", include_str!("../../venues/bybit.yaml")),
    ("okx", include_str!("../../venues/okx.yaml")),
    ("coinbase", include_str!("../../venues/coinbase.yaml")),
    ("kraken", include_str!("../../venues/kraken.yaml")),
    ("htx", include_str!("../../venues/htx.yaml")),
    ("crypto_com", include_str!("../../venues/crypto_com.yaml")),
];

// =============================================================================
// VenueRegistry
// =============================================================================

/// Registry of all available venue descriptors (built-in + user-defined).
///
/// Built-in venues are compiled into the binary. User venues in
/// `~/.config/scope/venues/*.yaml` can override or extend them.
#[derive(Debug, Clone)]
pub struct VenueRegistry {
    venues: HashMap<String, VenueDescriptor>,
}

impl Default for VenueRegistry {
    fn default() -> Self {
        let mut registry = Self {
            venues: HashMap::new(),
        };
        // Load built-in venues, ignoring parse errors for robustness.
        for (_id, yaml) in BUILT_IN_VENUES {
            if let Ok(desc) = serde_yaml::from_str::<VenueDescriptor>(yaml) {
                registry.venues.insert(desc.id.clone(), desc);
            }
        }
        registry
    }
}

impl VenueRegistry {
    /// Load all venues: built-in (embedded) + user-defined from disk.
    ///
    /// User venues with the same `id` as a built-in override the built-in,
    /// allowing users to customize endpoints without recompiling.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::user_venues_dir())
    }

    /// The built-in venues, plus the user venues in `user_dir`. A file that
    /// cannot be read or parsed is skipped with a warning.
    pub fn load_from(user_dir: &std::path::Path) -> Result<Self> {
        let mut registry = Self::default();
        if user_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(user_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    match std::fs::read_to_string(&path) {
                        Ok(contents) => match serde_yaml::from_str::<VenueDescriptor>(&contents) {
                            Ok(desc) => {
                                registry.venues.insert(desc.id.clone(), desc);
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
        }

        Ok(registry)
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

    /// Path to the user venues directory (`~/.config/scope/venues/`).
    pub fn user_venues_dir() -> PathBuf {
        crate::paths::venues_dir()
    }

    /// Create an ExchangeClient for the given venue ID.
    ///
    /// An unknown ID is [`ScopeError::InvalidInput`] that lists the registry
    /// venues and names the closest one. DEX venues are not listed: this
    /// lookup cannot serve them (see `MarketVenue::resolve`).
    pub fn create_exchange_client(
        &self,
        venue_id: &str,
    ) -> Result<crate::market::exchange::ExchangeClient> {
        let desc = self
            .get(venue_id)
            .ok_or_else(|| unknown_venue(venue_id, &self.list()))?;
        Ok(crate::market::exchange::ExchangeClient::from_descriptor(
            desc,
        ))
    }

    /// Validate a venue descriptor YAML string.
    pub fn validate_yaml(yaml: &str) -> Result<VenueDescriptor> {
        serde_yaml::from_str(yaml)
            .map_err(|e| ScopeError::Config(crate::error::ConfigError::Parse { source: e }))
    }
}

/// The unknown-venue error: names the closest of `ids` and lists them all.
/// Each lookup passes the ids it can serve, so it never suggests a venue it
/// would reject.
pub(crate) fn unknown_venue<S: AsRef<str>>(input: &str, ids: &[S]) -> ScopeError {
    let ids: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
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

    #[test]
    fn test_user_venue_overrides_built_in_and_bad_files_are_skipped() {
        // A user venue with a built-in id replaces the built-in (custom
        // endpoints without a rebuild); a broken file does not stop the load.
        let dir = tempfile::tempdir().unwrap();
        let binance = include_str!("../../venues/binance.yaml")
            .replace("name: Binance Spot", "name: My Binance");
        std::fs::write(dir.path().join("binance.yaml"), binance).unwrap();
        std::fs::write(dir.path().join("broken.yml"), "id: [unclosed").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a venue").unwrap();

        let registry = VenueRegistry::load_from(dir.path()).unwrap();
        assert_eq!(registry.len(), VenueRegistry::default().len());
        assert_eq!(registry.get("binance").unwrap().name, "My Binance");
    }

    #[test]
    fn test_missing_user_dir_gives_the_built_ins() {
        let dir = tempfile::tempdir().unwrap();
        let registry = VenueRegistry::load_from(&dir.path().join("none")).unwrap();
        assert_eq!(registry.len(), VenueRegistry::default().len());
    }

    #[test]
    fn test_registry_default_loads_built_in() {
        let registry = VenueRegistry::default();
        assert!(
            registry.len() >= 11,
            "Expected at least 11 built-in venues, got {}",
            registry.len()
        );
        assert!(registry.contains("binance"));
        assert!(registry.contains("biconomy"));
        assert!(registry.contains("mexc"));
        assert!(registry.contains("bitget"));
        assert!(registry.contains("gateio"));
        assert!(registry.contains("bybit"));
        assert!(registry.contains("okx"));
        assert!(registry.contains("coinbase"));
        assert!(registry.contains("kraken"));
        assert!(registry.contains("htx"));
        assert!(registry.contains("crypto_com"));
    }

    #[test]
    fn test_registry_list_sorted() {
        let registry = VenueRegistry::default();
        let list = registry.list();
        let mut sorted = list.clone();
        sorted.sort();
        assert_eq!(list, sorted);
    }

    #[test]
    fn test_registry_get() {
        let registry = VenueRegistry::default();
        let binance = registry.get("binance").unwrap();
        assert_eq!(binance.name, "Binance Spot");
        assert_eq!(binance.base_url, "https://api.binance.com");
    }

    #[test]
    fn test_registry_suggest_typo() {
        let registry = VenueRegistry::default();
        assert_eq!(registry.suggest("binace"), Some("binance"));
        assert_eq!(registry.suggest("gate"), Some("gateio"));
        // #69: "weex" is 3 edits from "okx", too far for a 4-letter name.
        assert_eq!(registry.suggest("weex"), None);
        assert_eq!(registry.suggest("krakn"), Some("kraken"));
        assert_eq!(registry.suggest("bybi"), Some("bybit"));
    }

    #[test]
    fn test_registry_suggest_no_match() {
        let registry = VenueRegistry::default();
        assert!(registry.suggest("zzzzzzzzz").is_none());
    }

    #[test]
    fn test_strsim_distance() {
        assert_eq!(strsim_distance("binance", "binance"), 0);
        assert_eq!(strsim_distance("binace", "binance"), 1);
        assert_eq!(strsim_distance("", "abc"), 3);
        assert_eq!(strsim_distance("abc", ""), 3);
    }

    #[test]
    fn test_strsim_distance_completely_different() {
        assert_eq!(strsim_distance("abc", "xyz"), 3);
        assert_eq!(strsim_distance("hello", "world"), 4);
    }

    #[test]
    fn test_validate_yaml_valid() {
        let yaml = r#"
id: test
name: Test Exchange
base_url: https://example.com
symbol:
  template: "{base}{quote}"
  default_quote: USDT
"#;
        let result = VenueRegistry::validate_yaml(yaml);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "test");
    }

    #[test]
    fn test_validate_yaml_invalid() {
        let yaml = "not: valid: yaml: [[[";
        let result = VenueRegistry::validate_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_user_venues_dir_is_reasonable() {
        let dir = VenueRegistry::user_venues_dir();
        let path_str = dir.to_string_lossy();
        assert!(path_str.contains("scope"));
        assert!(path_str.contains("venues"));
    }

    #[test]
    fn test_all_built_in_venues_have_capabilities() {
        let registry = VenueRegistry::default();
        for id in registry.list() {
            let desc = registry.get(id).unwrap();
            let caps = desc.capability_names();
            assert!(!caps.is_empty(), "Venue {} has no capabilities", id);
        }
    }

    #[test]
    fn test_all_built_in_format_pair() {
        let registry = VenueRegistry::default();
        for id in registry.list() {
            let desc = registry.get(id).unwrap();
            let pair = desc.format_pair("BTC", None);
            assert!(!pair.is_empty(), "Venue {} produced empty pair", id);
        }
    }

    #[test]
    fn test_registry_contains_existing_and_non_existing() {
        let registry = VenueRegistry::default();
        assert!(registry.contains("binance"));
        assert!(registry.contains("kraken"));
        assert!(!registry.contains("nonexistent"));
        assert!(!registry.contains("binace")); // typo, not exact match
    }

    #[test]
    fn test_registry_is_empty_returns_false_for_default() {
        let registry = VenueRegistry::default();
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_create_exchange_client_error_unknown_venue() {
        let registry = VenueRegistry::default();
        let err = registry
            .create_exchange_client("nonexistent_venue_xyz")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Unknown venue"));
        assert!(msg.contains("nonexistent_venue_xyz"));
    }

    #[test]
    fn test_create_exchange_client_error_includes_did_you_mean() {
        let registry = VenueRegistry::default();
        let err = registry.create_exchange_client("binace").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Unknown venue"));
        assert!(msg.contains("Did you mean"));
        assert!(msg.contains("binance"));
    }

    #[test]
    fn test_create_exchange_client_success() {
        let registry = VenueRegistry::default();
        let client = registry.create_exchange_client("binance").unwrap();
        assert_eq!(client.venue_name(), "Binance Spot");
    }

    #[test]
    fn test_registry_load_returns_ok() {
        let result = VenueRegistry::load();
        assert!(result.is_ok());
        let registry = result.unwrap();
        assert!(registry.len() >= 11);
    }

    #[test]
    fn test_strsim_distance_single_char_diff() {
        assert_eq!(strsim_distance("a", "b"), 1);
        assert_eq!(strsim_distance("test", "best"), 1);
    }

    #[test]
    fn test_suggest_exact_match_returns_same() {
        let registry = VenueRegistry::default();
        assert_eq!(registry.suggest("binance"), Some("binance"));
    }

    #[test]
    fn test_registry_get_nonexistent() {
        let registry = VenueRegistry::default();
        assert!(registry.get("nonexistent_xyz").is_none());
    }
}
