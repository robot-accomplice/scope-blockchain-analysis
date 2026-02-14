//! Generic exchange client that interprets a [`VenueDescriptor`] at runtime.
//!
//! Implements [`OrderBookClient`], [`TickerClient`], and [`TradeHistoryClient`]
//! by reading endpoint configurations, building HTTP requests, navigating
//! JSON responses via `response_root`, and mapping fields to common types.

use crate::error::{Result, ScopeError};
use crate::market::descriptor::{EndpointDescriptor, HttpMethod, ResponseMapping, VenueDescriptor};
use crate::market::orderbook::{
    OrderBook, OrderBookClient, OrderBookLevel, Ticker, TickerClient, Trade, TradeHistoryClient,
    TradeSide,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

/// A generic exchange client driven entirely by a [`VenueDescriptor`].
///
/// No venue-specific Rust code — all behavior comes from the YAML descriptor.
#[derive(Debug, Clone)]
pub struct ConfigurableExchangeClient {
    descriptor: VenueDescriptor,
    http: Client,
}

impl ConfigurableExchangeClient {
    /// Create a new client from a venue descriptor.
    pub fn new(descriptor: VenueDescriptor) -> Self {
        let timeout = descriptor.timeout_secs.unwrap_or(15);
        let http = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .build()
            .expect("reqwest client build");
        Self { descriptor, http }
    }

    /// The venue descriptor driving this client.
    pub fn descriptor(&self) -> &VenueDescriptor {
        &self.descriptor
    }

    /// Format a pair using the venue's symbol config.
    pub fn format_pair(&self, base: &str, quote: Option<&str>) -> String {
        self.descriptor.format_pair(base, quote)
    }

    // =========================================================================
    // HTTP request building
    // =========================================================================

    /// Execute an endpoint request and return the parsed JSON.
    async fn fetch_endpoint(
        &self,
        endpoint: &EndpointDescriptor,
        pair: &str,
        limit: Option<u32>,
    ) -> Result<Value> {
        let url = format!(
            "{}{}",
            self.descriptor.base_url,
            self.interpolate_path(&endpoint.path, pair)
        );

        let limit_str = limit.unwrap_or(100).to_string();

        match endpoint.method {
            HttpMethod::GET => {
                let mut req = self.http.get(&url);
                // Add headers
                for (k, v) in &self.descriptor.headers {
                    req = req.header(k, v);
                }
                // Add query params with interpolation
                let params: Vec<(String, String)> = endpoint
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), self.interpolate_value(v, pair, &limit_str)))
                    .collect();
                if !params.is_empty() {
                    req = req.query(&params);
                }

                let resp = req.send().await?;
                if !resp.status().is_success() {
                    return Err(ScopeError::Chain(format!(
                        "{} API error: HTTP {}",
                        self.descriptor.name,
                        resp.status()
                    )));
                }
                resp.json::<Value>().await.map_err(|e| {
                    ScopeError::Chain(format!("{} JSON parse error: {}", self.descriptor.name, e))
                })
            }
            HttpMethod::POST => {
                let mut req = self.http.post(&url);
                for (k, v) in &self.descriptor.headers {
                    req = req.header(k, v);
                }
                // Build request body from template with interpolation
                if let Some(body_template) = &endpoint.request_body {
                    let body = self.interpolate_json(body_template, pair, &limit_str);
                    req = req.json(&body);
                }

                let resp = req.send().await?;
                if !resp.status().is_success() {
                    return Err(ScopeError::Chain(format!(
                        "{} API error: HTTP {}",
                        self.descriptor.name,
                        resp.status()
                    )));
                }
                resp.json::<Value>().await.map_err(|e| {
                    ScopeError::Chain(format!("{} JSON parse error: {}", self.descriptor.name, e))
                })
            }
        }
    }

    /// Interpolate `{pair}` placeholders in a URL path.
    fn interpolate_path(&self, path: &str, pair: &str) -> String {
        path.replace("{pair}", pair)
    }

    /// Interpolate `{pair}` and `{limit}` placeholders in a string value.
    fn interpolate_value(&self, template: &str, pair: &str, limit: &str) -> String {
        template.replace("{pair}", pair).replace("{limit}", limit)
    }

    /// Recursively interpolate `{pair}` and `{limit}` in a JSON value template.
    fn interpolate_json(&self, value: &Value, pair: &str, limit: &str) -> Value {
        match value {
            Value::String(s) => Value::String(self.interpolate_value(s, pair, limit)),
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k.clone(), self.interpolate_json(v, pair, limit));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => Value::Array(
                arr.iter()
                    .map(|v| self.interpolate_json(v, pair, limit))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    // =========================================================================
    // Response navigation
    // =========================================================================

    /// Navigate from the JSON root to the data node using a dot-path.
    ///
    /// Supports:
    /// - `""` or `None`: returns root as-is
    /// - `"result"`: `json["result"]`
    /// - `"data.0"`: `json["data"][0]`
    /// - `"result.*"`: first value under `json["result"]` (for Kraken)
    /// - `"result.list.0"`: chained navigation
    fn navigate_root<'a>(&self, root: &'a Value, path: Option<&str>) -> Result<&'a Value> {
        let path = match path {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(root),
        };

        let mut current = root;
        for segment in path.split('.') {
            if segment == "*" {
                // Wildcard: take the first value in the object (for Kraken)
                current = match current {
                    Value::Object(map) => map.values().next().ok_or_else(|| {
                        ScopeError::Chain(format!(
                            "{}: empty object at wildcard '*'",
                            self.descriptor.name
                        ))
                    })?,
                    _ => {
                        return Err(ScopeError::Chain(format!(
                            "{}: expected object at wildcard '*', got {:?}",
                            self.descriptor.name,
                            current_type(current)
                        )));
                    }
                };
            } else if let Ok(idx) = segment.parse::<usize>() {
                // Numeric index
                current = current.get(idx).ok_or_else(|| {
                    ScopeError::Chain(format!(
                        "{}: index {} out of bounds",
                        self.descriptor.name, idx
                    ))
                })?;
            } else {
                // Object key
                current = current.get(segment).ok_or_else(|| {
                    ScopeError::Chain(format!(
                        "{}: missing key '{}' in response",
                        self.descriptor.name, segment
                    ))
                })?;
            }
        }
        Ok(current)
    }

    /// Extract a float value from a JSON value using a dot-path field name.
    ///
    /// Handles strings ("42.5"), numbers (42.5), and nested paths ("c.0").
    fn extract_f64(&self, data: &Value, field_path: &str) -> Option<f64> {
        let val = self.navigate_field(data, field_path)?;
        value_to_f64(val)
    }

    /// Navigate to a field within a data object using dot-notation.
    fn navigate_field<'a>(&self, data: &'a Value, path: &str) -> Option<&'a Value> {
        let mut current = data;
        for segment in path.split('.') {
            if let Ok(idx) = segment.parse::<usize>() {
                current = current.get(idx)?;
            } else {
                current = current.get(segment)?;
            }
        }
        Some(current)
    }

    /// Extract a string from a JSON value using a field path.
    fn extract_string(&self, data: &Value, field_path: &str) -> Option<String> {
        let val = self.navigate_field(data, field_path)?;
        match val {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    // =========================================================================
    // Order book parsing
    // =========================================================================

    /// Parse order book levels from a JSON array.
    fn parse_levels(&self, arr: &Value, mapping: &ResponseMapping) -> Result<Vec<OrderBookLevel>> {
        let items = arr.as_array().ok_or_else(|| {
            ScopeError::Chain(format!(
                "{}: expected array for levels",
                self.descriptor.name
            ))
        })?;

        let level_format = mapping.level_format.as_deref().unwrap_or("positional");
        let mut levels = Vec::with_capacity(items.len());

        for item in items {
            let (price, quantity) = match level_format {
                "object" => {
                    let price_field = mapping.level_price_field.as_deref().unwrap_or("price");
                    let size_field = mapping.level_size_field.as_deref().unwrap_or("size");
                    let p = self
                        .navigate_field(item, price_field)
                        .and_then(value_to_f64);
                    let q = self.navigate_field(item, size_field).and_then(value_to_f64);
                    (p, q)
                }
                _ => {
                    // Positional: [price, qty, ...optional_extra_fields]
                    let p = item.get(0).and_then(value_to_f64);
                    let q = item.get(1).and_then(value_to_f64);
                    (p, q)
                }
            };

            if let (Some(price), Some(quantity)) = (price, quantity)
                && price > 0.0
                && quantity > 0.0
            {
                levels.push(OrderBookLevel { price, quantity });
            }
        }

        Ok(levels)
    }

    // =========================================================================
    // Trade parsing
    // =========================================================================

    /// Parse a TradeSide from a JSON value using the side mapping.
    fn parse_side(&self, data: &Value, mapping: &ResponseMapping) -> TradeSide {
        if let Some(side_mapping) = &mapping.side
            && let Some(val) = self.navigate_field(data, &side_mapping.field)
        {
            let val_str = match val {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                _ => return TradeSide::Buy,
            };
            if let Some(canonical) = side_mapping.mapping.get(&val_str) {
                return match canonical.as_str() {
                    "sell" => TradeSide::Sell,
                    _ => TradeSide::Buy,
                };
            }
        }
        TradeSide::Buy
    }
}

// =============================================================================
// Trait implementations
// =============================================================================

#[async_trait]
impl OrderBookClient for ConfigurableExchangeClient {
    async fn fetch_order_book(&self, pair_symbol: &str) -> Result<OrderBook> {
        let endpoint = self
            .descriptor
            .capabilities
            .order_book
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!(
                    "{} does not support order book",
                    self.descriptor.name
                ))
            })?;

        let json = self.fetch_endpoint(endpoint, pair_symbol, None).await?;
        let data = self.navigate_root(&json, endpoint.response_root.as_deref())?;

        let asks_key = endpoint.response.asks_key.as_deref().unwrap_or("asks");
        let bids_key = endpoint.response.bids_key.as_deref().unwrap_or("bids");

        let asks_arr = data.get(asks_key).ok_or_else(|| {
            ScopeError::Chain(format!(
                "{}: missing '{}' in order book response",
                self.descriptor.name, asks_key
            ))
        })?;
        let bids_arr = data.get(bids_key).ok_or_else(|| {
            ScopeError::Chain(format!(
                "{}: missing '{}' in order book response",
                self.descriptor.name, bids_key
            ))
        })?;

        let mut asks = self.parse_levels(asks_arr, &endpoint.response)?;
        let mut bids = self.parse_levels(bids_arr, &endpoint.response)?;

        // Sort asks ascending, bids descending
        asks.sort_by(|a, b| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        bids.sort_by(|a, b| {
            b.price
                .partial_cmp(&a.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Build display pair name
        let pair = format_display_pair(pair_symbol, &self.descriptor.symbol.template);

        Ok(OrderBook { pair, bids, asks })
    }
}

#[async_trait]
impl TickerClient for ConfigurableExchangeClient {
    async fn fetch_ticker(&self, pair_symbol: &str) -> Result<Ticker> {
        let endpoint = self
            .descriptor
            .capabilities
            .ticker
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!("{} does not support ticker", self.descriptor.name))
            })?;

        let json = self.fetch_endpoint(endpoint, pair_symbol, None).await?;

        // For endpoints that return an array of tickers (with filter)
        let data = if let Some(filter) = &endpoint.response.filter {
            let root = self.navigate_root(&json, endpoint.response_root.as_deref())?;
            let items_key = endpoint.response.items_key.as_deref().unwrap_or("");
            let items = if items_key.is_empty() {
                root
            } else {
                root.get(items_key).unwrap_or(root)
            };
            let arr = items.as_array().ok_or_else(|| {
                ScopeError::Chain(format!(
                    "{}: expected array for ticker filter",
                    self.descriptor.name
                ))
            })?;
            let filter_value = filter.value.replace("{pair}", pair_symbol);
            arr.iter()
                .find(|item| {
                    item.get(&filter.field)
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == filter_value)
                })
                .ok_or_else(|| {
                    ScopeError::Chain(format!(
                        "{}: no ticker found for pair {}",
                        self.descriptor.name, pair_symbol
                    ))
                })?
                .clone()
        } else {
            self.navigate_root(&json, endpoint.response_root.as_deref())?
                .clone()
        };

        let r = &endpoint.response;
        let pair = format_display_pair(pair_symbol, &self.descriptor.symbol.template);

        Ok(Ticker {
            pair,
            last_price: r
                .last_price
                .as_ref()
                .and_then(|f| self.extract_f64(&data, f)),
            high_24h: r.high_24h.as_ref().and_then(|f| self.extract_f64(&data, f)),
            low_24h: r.low_24h.as_ref().and_then(|f| self.extract_f64(&data, f)),
            volume_24h: r
                .volume_24h
                .as_ref()
                .and_then(|f| self.extract_f64(&data, f)),
            quote_volume_24h: r
                .quote_volume_24h
                .as_ref()
                .and_then(|f| self.extract_f64(&data, f)),
            best_bid: r.best_bid.as_ref().and_then(|f| self.extract_f64(&data, f)),
            best_ask: r.best_ask.as_ref().and_then(|f| self.extract_f64(&data, f)),
        })
    }
}

#[async_trait]
impl TradeHistoryClient for ConfigurableExchangeClient {
    async fn fetch_recent_trades(&self, pair_symbol: &str, limit: u32) -> Result<Vec<Trade>> {
        let endpoint = self
            .descriptor
            .capabilities
            .trades
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!("{} does not support trades", self.descriptor.name))
            })?;

        let json = self
            .fetch_endpoint(endpoint, pair_symbol, Some(limit))
            .await?;
        let data = self.navigate_root(&json, endpoint.response_root.as_deref())?;

        // Determine the array of trade items
        let items_key = endpoint.response.items_key.as_deref().unwrap_or("");
        let arr = if items_key.is_empty() {
            data
        } else {
            data.get(items_key).unwrap_or(data)
        };

        let items = arr.as_array().ok_or_else(|| {
            ScopeError::Chain(format!(
                "{}: expected array for trades",
                self.descriptor.name
            ))
        })?;

        let r = &endpoint.response;
        let mut trades = Vec::with_capacity(items.len());

        for item in items {
            let price = r.price.as_ref().and_then(|f| self.extract_f64(item, f));
            let quantity = r.quantity.as_ref().and_then(|f| self.extract_f64(item, f));

            if let (Some(price), Some(quantity)) = (price, quantity) {
                let quote_quantity = r
                    .quote_quantity
                    .as_ref()
                    .and_then(|f| self.extract_f64(item, f));
                let timestamp_ms = r
                    .timestamp_ms
                    .as_ref()
                    .and_then(|f| self.extract_f64(item, f))
                    .map(|v| v as u64)
                    .unwrap_or(0);
                let id = r.id.as_ref().and_then(|f| self.extract_string(item, f));
                let side = self.parse_side(item, r);

                trades.push(Trade {
                    price,
                    quantity,
                    quote_quantity,
                    timestamp_ms,
                    side,
                    id,
                });
            }
        }

        Ok(trades)
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Convert a JSON value (string or number) to f64.
fn value_to_f64(val: &Value) -> Option<f64> {
    match val {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Return a human-readable type name for error messages.
fn current_type(val: &Value) -> &'static str {
    match val {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Convert a raw pair symbol back to display format (e.g., "BTCUSDT" → "BTC/USDT").
fn format_display_pair(raw: &str, template: &str) -> String {
    // Try to reverse-engineer the separator from the template
    let sep = if template.contains('_') {
        "_"
    } else if template.contains('-') {
        "-"
    } else {
        ""
    };

    if !sep.is_empty() {
        raw.replace(sep, "/")
    } else {
        // No separator: try to find where the quote starts
        let upper = raw.to_uppercase();
        for quote in &["USDT", "USD", "USDC", "BTC", "ETH", "EUR", "GBP"] {
            if upper.ends_with(quote) {
                let base_end = raw.len() - quote.len();
                if base_end > 0 {
                    return format!("{}/{}", &raw[..base_end], &raw[base_end..]);
                }
            }
        }
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_display_pair_underscore() {
        assert_eq!(
            format_display_pair("BTC_USDT", "{base}_{quote}"),
            "BTC/USDT"
        );
    }

    #[test]
    fn test_format_display_pair_dash() {
        assert_eq!(
            format_display_pair("BTC-USDT", "{base}-{quote}"),
            "BTC/USDT"
        );
    }

    #[test]
    fn test_format_display_pair_concatenated() {
        assert_eq!(format_display_pair("BTCUSDT", "{base}{quote}"), "BTC/USDT");
        assert_eq!(format_display_pair("ETHUSD", "{base}{quote}"), "ETH/USD");
    }

    #[test]
    fn test_value_to_f64_number() {
        let val = serde_json::json!(42.5);
        assert_eq!(value_to_f64(&val), Some(42.5));
    }

    #[test]
    fn test_value_to_f64_string() {
        let val = serde_json::json!("42.5");
        assert_eq!(value_to_f64(&val), Some(42.5));
    }

    #[test]
    fn test_value_to_f64_invalid() {
        let val = serde_json::json!(null);
        assert_eq!(value_to_f64(&val), None);
    }

    #[test]
    fn test_navigate_root_empty() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"price": 42});
        let result = client.navigate_root(&json, None).unwrap();
        assert_eq!(result, &json);
    }

    #[test]
    fn test_navigate_root_single_key() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": {"price": 42}});
        let result = client.navigate_root(&json, Some("result")).unwrap();
        assert_eq!(result, &serde_json::json!({"price": 42}));
    }

    #[test]
    fn test_navigate_root_nested_with_index() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"data": [{"price": 42}, {"price": 43}]});
        let result = client.navigate_root(&json, Some("data.0")).unwrap();
        assert_eq!(result, &serde_json::json!({"price": 42}));
    }

    #[test]
    fn test_navigate_root_wildcard() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": {"XXBTZUSD": {"a": ["42000.0"]}}});
        let result = client.navigate_root(&json, Some("result.*")).unwrap();
        assert_eq!(result, &serde_json::json!({"a": ["42000.0"]}));
    }

    #[test]
    fn test_extract_f64_nested() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"c": ["42000.5", "1.5"]});
        assert_eq!(client.extract_f64(&data, "c.0"), Some(42000.5));
        assert_eq!(client.extract_f64(&data, "c.1"), Some(1.5));
    }

    #[test]
    fn test_parse_positional_levels() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let arr = serde_json::json!([["42000.0", "1.5"], ["42001.0", "2.0"]]);
        let mapping = ResponseMapping {
            level_format: Some("positional".to_string()),
            ..Default::default()
        };
        let levels = client.parse_levels(&arr, &mapping).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, 42000.0);
        assert_eq!(levels[0].quantity, 1.5);
    }

    #[test]
    fn test_parse_object_levels() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let arr = serde_json::json!([
            {"price": "42000.0", "size": "1.5"},
            {"price": "42001.0", "size": "2.0"}
        ]);
        let mapping = ResponseMapping {
            level_format: Some("object".to_string()),
            level_price_field: Some("price".to_string()),
            level_size_field: Some("size".to_string()),
            ..Default::default()
        };
        let levels = client.parse_levels(&arr, &mapping).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, 42000.0);
        assert_eq!(levels[0].quantity, 1.5);
    }

    #[test]
    fn test_parse_side_mapping() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"isBuyerMaker": true});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "isBuyerMaker".to_string(),
                mapping: [
                    ("true".to_string(), "sell".to_string()),
                    ("false".to_string(), "buy".to_string()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Sell);
    }

    #[test]
    fn test_interpolate_json() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let template = serde_json::json!({
            "method": "get-book",
            "params": {"instrument": "{pair}", "depth": "{limit}"}
        });
        let result = client.interpolate_json(&template, "BTC_USDT", "100");
        assert_eq!(
            result,
            serde_json::json!({
                "method": "get-book",
                "params": {"instrument": "BTC_USDT", "depth": "100"}
            })
        );
    }

    fn make_test_descriptor() -> VenueDescriptor {
        use crate::market::descriptor::*;
        VenueDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet::default(),
        }
    }
}
