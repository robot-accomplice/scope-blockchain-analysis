//! Venue descriptor schema for data-driven exchange integration.
//!
//! Each venue is described by a YAML file that defines HTTP mechanics and
//! response field mappings. The [`VenueDescriptor`] struct is deserialized
//! from these files and interpreted at runtime by `ConfigurableExchangeClient`.

use serde::Deserialize;
use std::collections::HashMap;

/// How to format the trading pair symbol for a venue.
#[derive(Debug, Clone, Deserialize)]
pub struct SymbolConfig {
    /// Template with `{base}` and `{quote}` placeholders.
    /// Examples: `"{base}{quote}"` → `BTCUSDT`, `"{base}_{quote}"` → `BTC_USDT`.
    pub template: String,

    /// Default quote currency when the user doesn't specify one.
    pub default_quote: String,

    /// Case transformation for the final symbol. Defaults to `Upper`.
    #[serde(default)]
    pub case: SymbolCase,
}

/// Case transformation applied to the formatted symbol.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SymbolCase {
    #[default]
    Upper,
    Lower,
}

/// HTTP method for an endpoint (defaults to GET).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    #[serde(alias = "get")]
    GET,
    #[serde(alias = "post")]
    POST,
}

/// Describes a single API endpoint (order_book, ticker, or trades).
#[derive(Debug, Clone, Deserialize)]
pub struct EndpointDescriptor {
    /// HTTP method. Defaults to GET.
    #[serde(default)]
    pub method: HttpMethod,

    /// URL path appended to `base_url` (e.g., `/api/v3/depth`).
    pub path: String,

    /// Query parameters for GET, or ignored for POST.
    /// Values may contain `{pair}`, `{limit}`, `{base}`, `{quote}` placeholders.
    #[serde(default)]
    pub params: HashMap<String, String>,

    /// JSON body template for POST requests. Values may contain placeholders.
    pub request_body: Option<serde_json::Value>,

    /// Dot-path to navigate from the JSON root to the data before field mapping.
    ///
    /// Special values:
    /// - `""` or omitted: root is the data.
    /// - `"result"`: `json["result"]`.
    /// - `"data.0"`: `json["data"][0]`.
    /// - `"result.*"`: first value under `json["result"]` regardless of key.
    pub response_root: Option<String>,

    /// Maps canonical interval names (e.g., `1m`, `5m`, `1h`, `1d`) to the
    /// venue-specific strings (e.g., `1min`, `5min`, `hour`, `day`).
    /// Only used by the OHLC capability; omit when the venue accepts canonical names.
    #[serde(default)]
    pub interval_map: HashMap<String, String>,

    /// Field mappings for parsing the response.
    pub response: ResponseMapping,
}

/// Field mapping configuration for parsing venue API responses.
///
/// All fields are optional; omitting a field means the venue doesn't provide
/// that data (the corresponding Rust `Option` will be `None`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResponseMapping {
    // -- Order book fields --
    /// JSON key for the asks array.
    pub asks_key: Option<String>,
    /// JSON key for the bids array.
    pub bids_key: Option<String>,
    /// Level format: `"positional"` (default) for `[price, qty]` arrays,
    /// `"object"` for `{price: x, size: y}` objects.
    pub level_format: Option<String>,
    /// Field name for price when `level_format` is `"object"`.
    pub level_price_field: Option<String>,
    /// Field name for size/quantity when `level_format` is `"object"`.
    pub level_size_field: Option<String>,

    // -- Ticker fields (response key → JSON field name) --
    pub last_price: Option<String>,
    pub high_24h: Option<String>,
    pub low_24h: Option<String>,
    pub volume_24h: Option<String>,
    pub quote_volume_24h: Option<String>,
    pub best_bid: Option<String>,
    pub best_ask: Option<String>,

    // -- Trade / array fields --
    /// JSON key holding the array of items. Empty or omitted = root is the array.
    pub items_key: Option<String>,

    /// Filter configuration for endpoints that return data for all pairs.
    pub filter: Option<FilterConfig>,

    /// Field mappings for individual trade items.
    pub price: Option<String>,
    pub quantity: Option<String>,
    pub quote_quantity: Option<String>,
    pub timestamp_ms: Option<String>,
    pub id: Option<String>,
    pub side: Option<SideMapping>,

    // -- OHLC / klines fields --
    /// Response format: `"array_of_arrays"` (e.g., Binance) or `"objects"` (default).
    /// When `"array_of_arrays"`, each candle is a positional array and the
    /// `ohlc_fields` list determines field order.
    pub ohlc_format: Option<String>,
    /// Ordered field names for array-of-arrays format.
    /// Default: `["open_time", "open", "high", "low", "close", "volume", "close_time"]`.
    pub ohlc_fields: Option<Vec<String>>,
    // For object format, reuse: open_time, open, high, low, close, volume, close_time
    // mapped from explicit field names below.
    pub open_time: Option<String>,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    /// Base volume field name (for OHLC). Falls back to `volume_24h` if absent.
    pub ohlc_volume: Option<String>,
    pub close_time: Option<String>,
}

/// Maps venue-specific side indicators to canonical buy/sell.
#[derive(Debug, Clone, Deserialize)]
pub struct SideMapping {
    /// JSON field that contains the side indicator.
    pub field: String,
    /// Map from venue-specific values to `"buy"` or `"sell"`.
    pub mapping: HashMap<String, String>,
}

/// Filter configuration for multi-pair endpoints.
#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    /// Response field to match against.
    pub field: String,
    /// Expected value (supports `{pair}` interpolation).
    pub value: String,
}

/// Set of API capabilities a venue provides.
/// Each capability is optional — omit if the venue doesn't support it.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CapabilitySet {
    /// Order book / depth endpoint.
    pub order_book: Option<EndpointDescriptor>,
    /// 24h ticker endpoint.
    pub ticker: Option<EndpointDescriptor>,
    /// Recent trades endpoint.
    pub trades: Option<EndpointDescriptor>,
    /// OHLC / klines / candlestick endpoint.
    pub ohlc: Option<EndpointDescriptor>,
}

/// Complete venue descriptor deserialized from a YAML file.
///
/// Defines everything needed to interact with an exchange venue:
/// base URL, authentication headers, symbol formatting, rate limits,
/// and per-capability endpoint configurations.
#[derive(Debug, Clone, Deserialize)]
pub struct VenueDescriptor {
    /// Unique venue identifier (e.g., `"binance"`).
    pub id: String,
    /// Human-readable name (e.g., `"Binance Spot"`).
    pub name: String,
    /// API base URL (e.g., `"https://api.binance.com"`).
    pub base_url: String,
    /// Request timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// Rate limit (requests per second).
    pub rate_limit_per_sec: Option<u32>,
    /// How to format the trading pair symbol.
    pub symbol: SymbolConfig,
    /// Headers added to all requests (e.g., `X-SITE-ID: "127"`).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Available API capabilities.
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

impl VenueDescriptor {
    /// Format a trading pair symbol for this venue.
    ///
    /// Replaces `{base}` and `{quote}` in the template, then applies case.
    pub fn format_pair(&self, base: &str, quote: Option<&str>) -> String {
        let q = quote.unwrap_or(&self.symbol.default_quote);
        let raw = self
            .symbol
            .template
            .replace("{base}", base)
            .replace("{quote}", q);
        match self.symbol.case {
            SymbolCase::Upper => raw.to_uppercase(),
            SymbolCase::Lower => raw.to_lowercase(),
        }
    }

    /// Check which capabilities this venue supports.
    pub fn has_order_book(&self) -> bool {
        self.capabilities.order_book.is_some()
    }
    pub fn has_ticker(&self) -> bool {
        self.capabilities.ticker.is_some()
    }
    pub fn has_trades(&self) -> bool {
        self.capabilities.trades.is_some()
    }
    pub fn has_ohlc(&self) -> bool {
        self.capabilities.ohlc.is_some()
    }

    /// Return a list of capability names this venue supports.
    pub fn capability_names(&self) -> Vec<&'static str> {
        let mut caps = Vec::new();
        if self.has_order_book() {
            caps.push("order_book");
        }
        if self.has_ticker() {
            caps.push("ticker");
        }
        if self.has_trades() {
            caps.push("trades");
        }
        if self.has_ohlc() {
            caps.push("ohlc");
        }
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_case_default_is_upper() {
        let case = SymbolCase::default();
        assert_eq!(case, SymbolCase::Upper);
    }

    #[test]
    fn test_http_method_default_is_get() {
        let method = HttpMethod::default();
        assert_eq!(method, HttpMethod::GET);
    }

    #[test]
    fn test_format_pair_upper() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("BTC", None), "BTCUSDT");
        assert_eq!(desc.format_pair("btc", None), "BTCUSDT");
        assert_eq!(desc.format_pair("ETH", Some("USD")), "ETHUSD");
    }

    #[test]
    fn test_format_pair_lower() {
        let desc = VenueDescriptor {
            id: "htx".to_string(),
            name: "HTX".to_string(),
            base_url: "https://api.huobi.pro".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Lower,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("BTC", None), "btcusdt");
    }

    #[test]
    fn test_format_pair_underscore() {
        let desc = VenueDescriptor {
            id: "biconomy".to_string(),
            name: "Biconomy".to_string(),
            base_url: "https://api.biconomy.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}_{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("PUSD", None), "PUSD_USDT");
    }

    #[test]
    fn test_format_pair_dash() {
        let desc = VenueDescriptor {
            id: "okx".to_string(),
            name: "OKX".to_string(),
            base_url: "https://www.okx.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}-{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("BTC", None), "BTC-USDT");
    }

    #[test]
    fn test_capability_names_all() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/depth".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ticker: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/ticker".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                trades: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/trades".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ohlc: None,
            },
        };
        let caps = desc.capability_names();
        assert_eq!(caps, vec!["order_book", "ticker", "trades"]);
    }

    #[test]
    fn test_capability_names_partial() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/depth".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ticker: None,
                trades: None,
                ohlc: None,
            },
        };
        assert_eq!(desc.capability_names(), vec!["order_book"]);
    }

    #[test]
    fn test_deserialize_binance_yaml() {
        let yaml = r#"
id: binance
name: Binance Spot
base_url: https://api.binance.com
timeout_secs: 15
rate_limit_per_sec: 10

symbol:
  template: "{base}{quote}"
  default_quote: USDT

capabilities:
  order_book:
    path: /api/v3/depth
    params:
      symbol: "{pair}"
      limit: "100"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional

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
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(desc.id, "binance");
        assert_eq!(desc.name, "Binance Spot");
        assert_eq!(desc.base_url, "https://api.binance.com");
        assert_eq!(desc.timeout_secs, Some(15));
        assert_eq!(desc.symbol.template, "{base}{quote}");
        assert_eq!(desc.symbol.default_quote, "USDT");
        assert_eq!(desc.symbol.case, SymbolCase::Upper);

        // Order book
        let ob = desc.capabilities.order_book.as_ref().unwrap();
        assert_eq!(ob.path, "/api/v3/depth");
        assert_eq!(ob.params.get("symbol"), Some(&"{pair}".to_string()));
        assert_eq!(ob.response.asks_key, Some("asks".to_string()));
        assert_eq!(ob.response.level_format, Some("positional".to_string()));

        // Ticker
        let ticker = desc.capabilities.ticker.as_ref().unwrap();
        assert_eq!(ticker.response.last_price, Some("lastPrice".to_string()));
        assert_eq!(ticker.response.volume_24h, Some("volume".to_string()));

        // Trades
        let trades = desc.capabilities.trades.as_ref().unwrap();
        assert_eq!(trades.response.price, Some("price".to_string()));
        let side = trades.response.side.as_ref().unwrap();
        assert_eq!(side.field, "isBuyerMaker");
        assert_eq!(side.mapping.get("true"), Some(&"sell".to_string()));
    }

    #[test]
    fn test_deserialize_htx_lowercase() {
        let yaml = r#"
id: htx
name: HTX
base_url: https://api.huobi.pro

symbol:
  template: "{base}{quote}"
  default_quote: USDT
  case: lower

capabilities:
  order_book:
    path: /market/depth
    params:
      symbol: "{pair}"
      type: step0
    response_root: tick
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(desc.symbol.case, SymbolCase::Lower);
        assert_eq!(desc.format_pair("BTC", None), "btcusdt");
        let ob = desc.capabilities.order_book.as_ref().unwrap();
        assert_eq!(ob.response_root, Some("tick".to_string()));
    }

    #[test]
    fn test_deserialize_post_method() {
        let yaml = r#"
id: crypto_com
name: Crypto.com
base_url: https://api.crypto.com/exchange/v1

symbol:
  template: "{base}_{quote}"
  default_quote: USDT

capabilities:
  order_book:
    method: POST
    path: /public/get-book
    request_body:
      method: "public/get-book"
      params:
        instrument_name: "{pair}"
        depth: "100"
    response_root: "result.data.0"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        let ob = desc.capabilities.order_book.as_ref().unwrap();
        assert_eq!(ob.method, HttpMethod::POST);
        assert!(ob.request_body.is_some());
        assert_eq!(ob.response_root, Some("result.data.0".to_string()));
    }

    #[test]
    fn test_deserialize_object_level_format() {
        let yaml = r#"
id: coinbase
name: Coinbase
base_url: https://api.coinbase.com

symbol:
  template: "{base}-{quote}"
  default_quote: USD

capabilities:
  order_book:
    path: /api/v3/brokerage/market/product_book
    params:
      product_id: "{pair}"
      limit: "100"
    response_root: pricebook
    response:
      asks_key: asks
      bids_key: bids
      level_format: object
      level_price_field: price
      level_size_field: size
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        let ob = desc.capabilities.order_book.as_ref().unwrap();
        assert_eq!(ob.response.level_format, Some("object".to_string()));
        assert_eq!(ob.response.level_price_field, Some("price".to_string()));
        assert_eq!(ob.response.level_size_field, Some("size".to_string()));
        assert_eq!(ob.response_root, Some("pricebook".to_string()));
    }

    #[test]
    fn test_venue_descriptor_format_pair_upper() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("btc", None), "BTCUSDT");
        assert_eq!(desc.format_pair("ETH", Some("USD")), "ETHUSD");
    }

    #[test]
    fn test_venue_descriptor_format_pair_lower() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Lower,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("BTC", None), "btcusdt");
        assert_eq!(desc.format_pair("ETH", Some("usd")), "ethusd");
    }

    #[test]
    fn test_venue_descriptor_format_pair_with_separator() {
        let desc = VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}_{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert_eq!(desc.format_pair("BTC", None), "BTC_USDT");
        assert_eq!(desc.format_pair("PUSD", Some("USD")), "PUSD_USD");
    }

    #[test]
    fn test_capability_names() {
        // Empty capabilities
        let desc = VenueDescriptor {
            id: "empty".to_string(),
            name: "Empty".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet::default(),
        };
        assert!(desc.capability_names().is_empty());

        // Only trades
        let desc_trades = VenueDescriptor {
            id: "trades_only".to_string(),
            name: "Trades Only".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/trades".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ohlc: None,
            },
        };
        assert_eq!(desc_trades.capability_names(), vec!["trades"]);
    }

    #[test]
    fn test_has_order_book() {
        let with_ob = VenueDescriptor {
            id: "x".to_string(),
            name: "X".to_string(),
            base_url: "https://x.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/depth".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ticker: None,
                trades: None,
                ohlc: None,
            },
        };
        assert!(with_ob.has_order_book());

        let without = VenueDescriptor {
            capabilities: CapabilitySet::default(),
            ..with_ob
        };
        assert!(!without.has_order_book());
    }

    #[test]
    fn test_has_ticker() {
        let with_ticker = VenueDescriptor {
            id: "x".to_string(),
            name: "X".to_string(),
            base_url: "https://x.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/ticker".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                trades: None,
                ohlc: None,
            },
        };
        assert!(with_ticker.has_ticker());

        let without = VenueDescriptor {
            capabilities: CapabilitySet::default(),
            ..with_ticker
        };
        assert!(!without.has_ticker());
    }

    #[test]
    fn test_has_trades() {
        let with_trades = VenueDescriptor {
            id: "x".to_string(),
            name: "X".to_string(),
            base_url: "https://x.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/trades".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
                ohlc: None,
            },
        };
        assert!(with_trades.has_trades());

        let without = VenueDescriptor {
            capabilities: CapabilitySet::default(),
            ..with_trades
        };
        assert!(!without.has_trades());
    }

    #[test]
    fn test_has_ohlc() {
        let with_ohlc = VenueDescriptor {
            id: "x".to_string(),
            name: "X".to_string(),
            base_url: "https://x.com".to_string(),
            timeout_secs: None,
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    method: HttpMethod::GET,
                    path: "/klines".to_string(),
                    params: HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: HashMap::new(),
                    response: ResponseMapping::default(),
                }),
            },
        };
        assert!(with_ohlc.has_ohlc());
        assert!(with_ohlc.capability_names().contains(&"ohlc"));

        let without = VenueDescriptor {
            capabilities: CapabilitySet::default(),
            ..with_ohlc
        };
        assert!(!without.has_ohlc());
        assert!(!without.capability_names().contains(&"ohlc"));
    }

    #[test]
    fn test_response_mapping_default() {
        let m = ResponseMapping::default();
        assert!(m.asks_key.is_none());
        assert!(m.bids_key.is_none());
        assert!(m.level_format.is_none());
        assert!(m.side.is_none());
    }

    #[test]
    fn test_capability_set_default() {
        let c = CapabilitySet::default();
        assert!(c.order_book.is_none());
        assert!(c.ticker.is_none());
        assert!(c.trades.is_none());
        assert!(c.ohlc.is_none());
    }

    #[test]
    fn test_deserialize_ohlc_capability() {
        let yaml = r#"
id: ohlc_venue
name: OHLC Venue Test
base_url: https://api.example.com

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
  ohlc:
    path: /api/v3/klines
    params:
      symbol: "{pair}"
      interval: "{interval}"
      limit: "{limit}"
    response:
      ohlc_format: array_of_arrays
      ohlc_fields: [open_time, open, high, low, close, volume, close_time]
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(desc.has_ohlc());
        assert!(desc.capability_names().contains(&"ohlc"));
        let ohlc = desc.capabilities.ohlc.as_ref().unwrap();
        assert_eq!(ohlc.path, "/api/v3/klines");
        assert_eq!(ohlc.params.get("interval"), Some(&"{interval}".to_string()));
        // interval_map should default to empty when omitted from YAML
        assert!(ohlc.interval_map.is_empty());
    }

    #[test]
    fn test_deserialize_ohlc_with_interval_map() {
        let yaml = r#"
id: biconomy_test
name: Biconomy Test
base_url: https://api.biconomy.com

symbol:
  template: "{base}_{quote}"
  default_quote: USDT

capabilities:
  ohlc:
    path: /api/v1/kline
    params:
      symbol: "{pair}"
      type: "{interval}"
      size: "{limit}"
    interval_map:
      1m: 1min
      5m: 5min
      15m: 15min
      30m: 30min
      1h: hour
      4h: hour
      1d: day
    response:
      ohlc_format: array_of_arrays
      ohlc_fields: [open_time, open, high, low, close, volume]
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(desc.has_ohlc());
        let ohlc = desc.capabilities.ohlc.as_ref().unwrap();
        assert_eq!(ohlc.interval_map.len(), 7);
        assert_eq!(ohlc.interval_map.get("1m"), Some(&"1min".to_string()));
        assert_eq!(ohlc.interval_map.get("1h"), Some(&"hour".to_string()));
        assert_eq!(ohlc.interval_map.get("4h"), Some(&"hour".to_string()));
        assert_eq!(ohlc.interval_map.get("1d"), Some(&"day".to_string()));
        // Unmapped keys should not be present
        assert!(!ohlc.interval_map.contains_key("1w"));
    }

    #[test]
    fn test_deserialize_biconomy_venue_yaml() {
        let yaml =
            std::fs::read_to_string("venues/biconomy.yaml").expect("biconomy.yaml should exist");
        let desc: VenueDescriptor = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(desc.id, "biconomy");
        assert_eq!(desc.name, "Biconomy");
        assert!(desc.has_order_book());
        assert!(desc.has_ticker());
        assert!(desc.has_trades());
        assert!(desc.has_ohlc());
        let ohlc = desc.capabilities.ohlc.as_ref().unwrap();
        assert!(
            !ohlc.interval_map.is_empty(),
            "biconomy should have interval_map"
        );
        assert_eq!(ohlc.interval_map.get("1h"), Some(&"hour".to_string()));
    }

    #[test]
    fn test_endpoint_descriptor_interval_map_defaults_empty() {
        let ep = EndpointDescriptor {
            method: HttpMethod::GET,
            path: "/test".to_string(),
            params: HashMap::new(),
            request_body: None,
            response_root: None,
            interval_map: HashMap::new(),
            response: ResponseMapping::default(),
        };
        assert!(ep.interval_map.is_empty());
    }

    #[test]
    fn test_deserialize_full_descriptor_yaml() {
        let yaml = r#"
id: test_venue
name: Test Venue Full
base_url: https://api.test.com
timeout_secs: 30
rate_limit_per_sec: 5

symbol:
  template: "{base}-{quote}"
  default_quote: USD
  case: upper

headers:
  X-API-KEY: "placeholder"

capabilities:
  order_book:
    path: /book
    params:
      pair: "{pair}"
    response:
      asks_key: asks
      bids_key: bids
  ticker:
    path: /ticker
    response:
      last_price: last
  trades:
    path: /trades
    response:
      price: p
      quantity: q
"#;
        let desc: VenueDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(desc.id, "test_venue");
        assert_eq!(desc.name, "Test Venue Full");
        assert_eq!(desc.base_url, "https://api.test.com");
        assert_eq!(desc.timeout_secs, Some(30));
        assert_eq!(desc.rate_limit_per_sec, Some(5));
        assert_eq!(desc.symbol.template, "{base}-{quote}");
        assert_eq!(desc.symbol.default_quote, "USD");
        assert_eq!(
            desc.headers.get("X-API-KEY"),
            Some(&"placeholder".to_string())
        );
        assert!(desc.has_order_book());
        assert!(desc.has_ticker());
        assert!(desc.has_trades());
        assert_eq!(
            desc.capability_names(),
            vec!["order_book", "ticker", "trades"]
        );
    }
}
