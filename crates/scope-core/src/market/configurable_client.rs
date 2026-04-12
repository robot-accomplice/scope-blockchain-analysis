//! Generic exchange client that interprets a [`VenueDescriptor`] at runtime.
//!
//! Implements [`OrderBookClient`], [`TickerClient`], and [`TradeHistoryClient`]
//! by reading endpoint configurations, building HTTP requests, navigating
//! JSON responses via `response_root`, and mapping fields to common types.

use crate::error::{Result, ScopeError};
use crate::market::descriptor::{EndpointDescriptor, HttpMethod, ResponseMapping, VenueDescriptor};
use crate::market::orderbook::{
    Candle, OhlcClient, OrderBook, OrderBookClient, OrderBookLevel, Ticker, TickerClient, Trade,
    TradeHistoryClient, TradeSide,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

fn format_http_api_error(venue: &str, status: reqwest::StatusCode, body: &str) -> ScopeError {
    let body = body.trim();
    let binance_invalid_symbol = venue.to_lowercase().contains("binance")
        && (body.contains("\"code\":-1121")
            || body.contains("\"code\": -1121")
            || body.to_lowercase().contains("invalid symbol"));

    if body.is_empty() {
        ScopeError::Chain(format!("{venue} API error: HTTP {status}"))
    } else if binance_invalid_symbol {
        let preview: String = body.chars().take(200).collect();
        ScopeError::Chain(format!(
            "{venue} API error: HTTP {status} — response: {preview}\n\
             Hint: Binance returned 'Invalid symbol'. Verify pair ordering and venue format \
             (e.g. BASE/QUOTE -> BASEQUOTE), or try another venue if the market is unavailable."
        ))
    } else {
        let preview: String = body.chars().take(200).collect();
        ScopeError::Chain(format!(
            "{venue} API error: HTTP {status} — response: {preview}"
        ))
    }
}

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
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format_http_api_error(&self.descriptor.name, status, &body));
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
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format_http_api_error(&self.descriptor.name, status, &body));
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

    /// Interpolate `{pair}`, `{limit}`, and `{interval}` placeholders in a string value.
    fn interpolate_value(&self, template: &str, pair: &str, limit: &str) -> String {
        self.interpolate_value_full(template, pair, limit, "")
    }

    /// Full interpolation with all supported placeholders.
    fn interpolate_value_full(
        &self,
        template: &str,
        pair: &str,
        limit: &str,
        interval: &str,
    ) -> String {
        template
            .replace("{pair}", pair)
            .replace("{limit}", limit)
            .replace("{interval}", interval)
    }

    /// Recursively interpolate `{pair}`, `{limit}`, and `{interval}` in a JSON value template.
    fn interpolate_json(&self, value: &Value, pair: &str, limit: &str) -> Value {
        self.interpolate_json_full(value, pair, limit, "")
    }

    fn interpolate_json_full(
        &self,
        value: &Value,
        pair: &str,
        limit: &str,
        interval: &str,
    ) -> Value {
        match value {
            Value::String(s) => {
                Value::String(self.interpolate_value_full(s, pair, limit, interval))
            }
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(
                        k.clone(),
                        self.interpolate_json_full(v, pair, limit, interval),
                    );
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => Value::Array(
                arr.iter()
                    .map(|v| self.interpolate_json_full(v, pair, limit, interval))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Execute an endpoint request with `{interval}` support (for OHLC).
    async fn fetch_endpoint_with_interval(
        &self,
        endpoint: &EndpointDescriptor,
        pair: &str,
        limit: Option<u32>,
        interval: &str,
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
                for (k, v) in &self.descriptor.headers {
                    req = req.header(k, v);
                }
                let params: Vec<(String, String)> = endpoint
                    .params
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            self.interpolate_value_full(v, pair, &limit_str, interval),
                        )
                    })
                    .collect();
                if !params.is_empty() {
                    req = req.query(&params);
                }
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format_http_api_error(&self.descriptor.name, status, &body));
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
                if let Some(body_template) = &endpoint.request_body {
                    let body =
                        self.interpolate_json_full(body_template, pair, &limit_str, interval);
                    req = req.json(&body);
                }
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format_http_api_error(&self.descriptor.name, status, &body));
                }
                resp.json::<Value>().await.map_err(|e| {
                    ScopeError::Chain(format!("{} JSON parse error: {}", self.descriptor.name, e))
                })
            }
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

#[async_trait]
impl OhlcClient for ConfigurableExchangeClient {
    async fn fetch_ohlc(
        &self,
        pair_symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let endpoint = self.descriptor.capabilities.ohlc.as_ref().ok_or_else(|| {
            ScopeError::Chain(format!("{} does not support OHLC", self.descriptor.name))
        })?;

        // Map canonical interval (e.g., "1m") to venue-specific format (e.g., "1min")
        let mapped_interval = endpoint
            .interval_map
            .get(interval)
            .map(|s| s.as_str())
            .unwrap_or(interval);

        let json = self
            .fetch_endpoint_with_interval(endpoint, pair_symbol, Some(limit), mapped_interval)
            .await?;
        let data = self.navigate_root(&json, endpoint.response_root.as_deref())?;

        // Determine the array of candle items
        let items_key = endpoint.response.items_key.as_deref().unwrap_or("");
        let arr = if items_key.is_empty() {
            data
        } else {
            data.get(items_key).unwrap_or(data)
        };

        let items = arr.as_array().ok_or_else(|| {
            ScopeError::Chain(format!(
                "{}: expected array for OHLC data",
                self.descriptor.name
            ))
        })?;

        let r = &endpoint.response;
        let format = r.ohlc_format.as_deref().unwrap_or("objects");
        let mut candles = Vec::with_capacity(items.len());

        if format == "array_of_arrays" {
            // Each candle is a positional array, e.g. Binance klines:
            // [open_time, open, high, low, close, volume, close_time, ...]
            let default_fields = vec![
                "open_time".to_string(),
                "open".to_string(),
                "high".to_string(),
                "low".to_string(),
                "close".to_string(),
                "volume".to_string(),
                "close_time".to_string(),
            ];
            let fields = r.ohlc_fields.as_ref().unwrap_or(&default_fields);
            let idx = |name: &str| -> Option<usize> { fields.iter().position(|f| f == name) };

            for item in items {
                let arr = match item.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                let get_f64 = |i: Option<usize>| -> Option<f64> {
                    i.and_then(|idx| arr.get(idx)).and_then(value_to_f64)
                };
                let get_u64 = |i: Option<usize>| -> Option<u64> { get_f64(i).map(|v| v as u64) };

                if let (Some(open), Some(high), Some(low), Some(close)) = (
                    get_f64(idx("open")),
                    get_f64(idx("high")),
                    get_f64(idx("low")),
                    get_f64(idx("close")),
                ) {
                    candles.push(Candle {
                        open_time: get_u64(idx("open_time")).unwrap_or(0),
                        open,
                        high,
                        low,
                        close,
                        volume: get_f64(idx("volume")).unwrap_or(0.0),
                        close_time: get_u64(idx("close_time")).unwrap_or(0),
                    });
                }
            }
        } else {
            // Object format — each candle is a JSON object with named fields.
            for item in items {
                let open = r.open.as_ref().and_then(|f| self.extract_f64(item, f));
                let high = r.high.as_ref().and_then(|f| self.extract_f64(item, f));
                let low = r.low.as_ref().and_then(|f| self.extract_f64(item, f));
                let close = r.close.as_ref().and_then(|f| self.extract_f64(item, f));

                if let (Some(open), Some(high), Some(low), Some(close)) = (open, high, low, close) {
                    let open_time = r
                        .open_time
                        .as_ref()
                        .and_then(|f| self.extract_f64(item, f))
                        .map(|v| v as u64)
                        .unwrap_or(0);
                    let volume = r
                        .ohlc_volume
                        .as_ref()
                        .and_then(|f| self.extract_f64(item, f))
                        .unwrap_or(0.0);
                    let close_time = r
                        .close_time
                        .as_ref()
                        .and_then(|f| self.extract_f64(item, f))
                        .map(|v| v as u64)
                        .unwrap_or(0);

                    candles.push(Candle {
                        open_time,
                        open,
                        high,
                        low,
                        close,
                        volume,
                        close_time,
                    });
                }
            }
        }

        Ok(candles)
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
    fn test_format_display_pair_eur_concatenated() {
        assert_eq!(format_display_pair("XBTEUR", "{base}{quote}"), "XBT/EUR");
    }

    #[test]
    fn test_format_display_pair_gbp_concatenated() {
        assert_eq!(format_display_pair("XBTGBP", "{base}{quote}"), "XBT/GBP");
    }

    #[test]
    fn test_format_display_pair_usdc_concatenated() {
        assert_eq!(format_display_pair("ETHUSDC", "{base}{quote}"), "ETH/USDC");
    }

    #[test]
    fn test_format_display_pair_no_quote_match_returns_raw() {
        assert_eq!(format_display_pair("XYZABC", "{base}{quote}"), "XYZABC");
    }

    #[test]
    fn test_format_display_pair_base_zero_len_returns_raw() {
        assert_eq!(format_display_pair("USDT", "{base}{quote}"), "USDT");
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

    #[test]
    fn test_interpolate_json_array_with_placeholders() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let template = serde_json::json!({
            "pairs": ["{pair}", "limit:{limit}"]
        });
        let result = client.interpolate_json(&template, "BTCUSDT", "50");
        assert_eq!(result["pairs"][0], "BTCUSDT");
        assert_eq!(result["pairs"][1], "limit:50");
    }

    #[test]
    fn test_interpolate_json_preserves_primitive_types() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let template = serde_json::json!({
            "flag": true,
            "count": 42,
            "name": "static"
        });
        let result = client.interpolate_json(&template, "BTC", "10");
        assert_eq!(result["flag"], true);
        assert_eq!(result["count"], 42);
        assert_eq!(result["name"], "static");
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

    // -------------------------------------------------------------------------
    // Additional tests for coverage of descriptor(), format_pair(), current_type,
    // extract_string(), navigate_field(), parse_levels(), parse_side(), format_display_pair
    // -------------------------------------------------------------------------

    #[test]
    fn test_descriptor_accessor() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc.clone());
        assert_eq!(client.descriptor().id, "test");
        assert_eq!(client.descriptor().name, "Test");
    }

    #[test]
    fn test_format_pair_accessor() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        assert_eq!(client.format_pair("BTC", None), "BTCUSDT");
        assert_eq!(client.format_pair("ETH", Some("USD")), "ETHUSD");
    }

    #[test]
    fn test_navigate_root_empty_string_path() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"price": 42});
        let result = client.navigate_root(&json, Some("")).unwrap();
        assert_eq!(result, &json);
    }

    #[test]
    fn test_navigate_root_wildcard_on_non_object_null() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": null});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("null"), "error should mention null type");
    }

    #[test]
    fn test_navigate_root_wildcard_on_non_object_bool() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": true});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("bool"), "error should mention bool type");
    }

    #[test]
    fn test_navigate_root_wildcard_on_non_object_number() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": 42});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("number"),
            "error should mention number type"
        );
    }

    #[test]
    fn test_navigate_root_wildcard_on_non_object_string() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": "not_an_object"});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("string"),
            "error should mention string type"
        );
    }

    #[test]
    fn test_navigate_root_wildcard_on_non_object_array() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": [1, 2, 3]});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("array"), "error should mention array type");
    }

    #[test]
    fn test_navigate_root_wildcard_empty_object() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"result": {}});
        let result = client.navigate_root(&json, Some("result.*"));
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("empty object"),
            "error should mention empty object"
        );
    }

    #[test]
    fn test_extract_string_from_string() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"id": "abc123"});
        assert_eq!(
            client.extract_string(&data, "id").as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn test_extract_string_from_number() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"id": 12345});
        assert_eq!(client.extract_string(&data, "id").as_deref(), Some("12345"));
    }

    #[test]
    fn test_extract_string_from_nested_path() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"a": {"b": {"c": ["x", "value"]}}});
        assert_eq!(
            client.extract_string(&data, "a.b.c.1").as_deref(),
            Some("value")
        );
    }

    #[test]
    fn test_extract_string_returns_none_for_object() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"id": {"nested": "obj"}});
        assert_eq!(client.extract_string(&data, "id"), None);
    }

    #[test]
    fn test_extract_string_returns_none_for_array() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"id": [1, 2, 3]});
        assert_eq!(client.extract_string(&data, "id"), None);
    }

    #[test]
    fn test_navigate_field_deeper_path() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"level1": {"level2": {"level3": [0, 99.5]}}});
        assert_eq!(
            client.extract_f64(&data, "level1.level2.level3.1"),
            Some(99.5)
        );
    }

    #[test]
    fn test_parse_levels_empty_array() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let arr = serde_json::json!([]);
        let mapping = ResponseMapping::default();
        let levels = client.parse_levels(&arr, &mapping).unwrap();
        assert!(levels.is_empty());
    }

    #[test]
    fn test_parse_levels_not_array_err() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let not_arr = serde_json::json!({"not": "array"});
        let mapping = ResponseMapping::default();
        let result = client.parse_levels(&not_arr, &mapping);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("expected array"));
    }

    #[test]
    fn test_parse_levels_filters_zero_price_and_quantity() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let arr = serde_json::json!([
            ["42000.0", "1.5"],
            ["0.0", "1.0"],
            ["42001.0", "0.0"],
            ["42002.0", ""]
        ]);
        let mapping = ResponseMapping {
            level_format: Some("positional".to_string()),
            ..Default::default()
        };
        let levels = client.parse_levels(&arr, &mapping).unwrap();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, 42000.0);
        assert_eq!(levels[0].quantity, 1.5);
    }

    #[test]
    fn test_parse_levels_object_format_default_fields() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let arr = serde_json::json!([
            {"price": "100.0", "size": "2.0"},
            {"price": "101.0", "size": "3.0"}
        ]);
        let mapping = ResponseMapping {
            level_format: Some("object".to_string()),
            level_price_field: None,
            level_size_field: None,
            ..Default::default()
        };
        let levels = client.parse_levels(&arr, &mapping).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, 100.0);
        assert_eq!(levels[0].quantity, 2.0);
    }

    #[test]
    fn test_parse_side_no_mapping_returns_buy() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": "sell"});
        let mapping = ResponseMapping {
            side: None,
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Buy);
    }

    #[test]
    fn test_parse_side_field_missing_returns_buy() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "nonexistent".to_string(),
                mapping: [("sell".to_string(), "sell".to_string())]
                    .into_iter()
                    .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Buy);
    }

    #[test]
    fn test_parse_side_string_mapped_to_sell() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": "ask"});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "side".to_string(),
                mapping: [
                    ("ask".to_string(), "sell".to_string()),
                    ("bid".to_string(), "buy".to_string()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Sell);
    }

    #[test]
    fn test_parse_side_string_mapped_to_buy() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": "bid"});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "side".to_string(),
                mapping: [
                    ("ask".to_string(), "sell".to_string()),
                    ("bid".to_string(), "buy".to_string()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Buy);
    }

    #[test]
    fn test_parse_side_numeric_value() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": 1});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "side".to_string(),
                mapping: [
                    ("1".to_string(), "sell".to_string()),
                    ("0".to_string(), "buy".to_string()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Sell);
    }

    #[test]
    fn test_parse_side_unknown_value_returns_buy() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": "unknown"});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "side".to_string(),
                mapping: [
                    ("ask".to_string(), "sell".to_string()),
                    ("bid".to_string(), "buy".to_string()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Buy);
    }

    #[test]
    fn test_parse_side_non_string_number_bool_returns_buy() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let data = serde_json::json!({"side": [1, 2, 3]});
        let mapping = ResponseMapping {
            side: Some(crate::market::descriptor::SideMapping {
                field: "side".to_string(),
                mapping: [("ask".to_string(), "sell".to_string())]
                    .into_iter()
                    .collect(),
            }),
            ..Default::default()
        };
        assert_eq!(client.parse_side(&data, &mapping), TradeSide::Buy);
    }

    #[test]
    fn test_format_display_pair_unknown_quote() {
        assert_eq!(format_display_pair("XYZABC", "{base}{quote}"), "XYZABC");
    }

    #[test]
    fn test_format_display_pair_single_char() {
        assert_eq!(format_display_pair("A", "{base}{quote}"), "A");
    }

    #[test]
    fn test_format_display_pair_quote_only_no_split() {
        assert_eq!(format_display_pair("USDT", "{base}{quote}"), "USDT");
    }

    // -------------------------------------------------------------------------
    // Mockito-based HTTP tests for async fetch paths
    // -------------------------------------------------------------------------

    fn make_http_test_descriptor(base_url: &str) -> VenueDescriptor {
        use crate::market::descriptor::*;
        VenueDescriptor {
            id: "mock_test".to_string(),
            name: "Mock Test".to_string(),
            base_url: base_url.to_string(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: Some(EndpointDescriptor {
                    path: "/api/v1/depth".to_string(),
                    method: HttpMethod::GET,
                    params: [("symbol".to_string(), "{pair}".to_string())]
                        .into_iter()
                        .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        asks_key: Some("asks".to_string()),
                        bids_key: Some("bids".to_string()),
                        level_format: Some("positional".to_string()),
                        ..Default::default()
                    },
                }),
                ticker: Some(EndpointDescriptor {
                    path: "/api/v1/ticker".to_string(),
                    method: HttpMethod::GET,
                    params: [("symbol".to_string(), "{pair}".to_string())]
                        .into_iter()
                        .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        last_price: Some("lastPrice".to_string()),
                        high_24h: Some("highPrice".to_string()),
                        low_24h: Some("lowPrice".to_string()),
                        volume_24h: Some("volume".to_string()),
                        quote_volume_24h: Some("quoteVolume".to_string()),
                        best_bid: Some("bidPrice".to_string()),
                        best_ask: Some("askPrice".to_string()),
                        ..Default::default()
                    },
                }),
                trades: Some(EndpointDescriptor {
                    path: "/api/v1/trades".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("limit".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        price: Some("price".to_string()),
                        quantity: Some("qty".to_string()),
                        quote_quantity: Some("quoteQty".to_string()),
                        timestamp_ms: Some("time".to_string()),
                        id: Some("id".to_string()),
                        side: Some(SideMapping {
                            field: "isBuyerMaker".to_string(),
                            mapping: [
                                ("true".to_string(), "sell".to_string()),
                                ("false".to_string(), "buy".to_string()),
                            ]
                            .into_iter()
                            .collect(),
                        }),
                        ..Default::default()
                    },
                }),
                ohlc: None,
            },
        }
    }

    #[tokio::test]
    async fn test_fetch_order_book_via_http() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/depth")
            .match_query(mockito::Matcher::AllOf(vec![mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            )]))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "asks": [["50010.0", "1.5"], ["50020.0", "2.0"]],
                    "bids": [["50000.0", "1.0"], ["49990.0", "3.0"]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let book = client.fetch_order_book("BTCUSDT").await.unwrap();
        assert_eq!(book.asks.len(), 2);
        assert_eq!(book.bids.len(), 2);
        assert_eq!(book.asks[0].price, 50010.0);
        assert_eq!(book.bids[0].price, 50000.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ticker_via_http() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/ticker")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "lastPrice": "50100.5",
                    "highPrice": "51200.0",
                    "lowPrice": "48800.0",
                    "volume": "1234.56",
                    "quoteVolume": "62000000.0",
                    "bidPrice": "50095.0",
                    "askPrice": "50105.0"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let ticker = client.fetch_ticker("BTCUSDT").await.unwrap();
        assert_eq!(ticker.pair, "BTC/USDT");
        assert_eq!(ticker.last_price, Some(50100.5));
        assert_eq!(ticker.high_24h, Some(51200.0));
        assert_eq!(ticker.low_24h, Some(48800.0));
        assert_eq!(ticker.volume_24h, Some(1234.56));
        assert_eq!(ticker.quote_volume_24h, Some(62000000.0));
        assert_eq!(ticker.best_bid, Some(50095.0));
        assert_eq!(ticker.best_ask, Some(50105.0));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_recent_trades_via_http() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/trades")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "10".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    {
                        "id": "trade-1",
                        "price": "50000.0",
                        "qty": "0.5",
                        "quoteQty": "25000.0",
                        "time": "1700000000000",
                        "isBuyerMaker": true
                    },
                    {
                        "id": "trade-2",
                        "price": "50001.0",
                        "qty": "1.0",
                        "quoteQty": "50001.0",
                        "time": "1700000001000",
                        "isBuyerMaker": false
                    }
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let trades = client.fetch_recent_trades("BTCUSDT", 10).await.unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].price, 50000.0);
        assert_eq!(trades[0].quantity, 0.5);
        assert_eq!(trades[0].quote_quantity, Some(25000.0));
        assert_eq!(trades[0].timestamp_ms, 1700000000000);
        assert_eq!(trades[0].id.as_deref(), Some("trade-1"));
        assert_eq!(trades[0].side, TradeSide::Sell);
        assert_eq!(trades[1].price, 50001.0);
        assert_eq!(trades[1].quantity, 1.0);
        assert_eq!(trades[1].side, TradeSide::Buy);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_order_book_http_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/depth")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_order_book("BTCUSDT").await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("API error: HTTP 500"),
            "expected error message to contain 'API error: HTTP 500', got: {}",
            err_msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_order_book_binance_invalid_symbol_hint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/depth")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "USDTPUSD".into(),
            ))
            .with_status(400)
            .with_body(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .create_async()
            .await;

        let mut desc = make_http_test_descriptor(&server.url());
        desc.name = "Binance Spot".to_string();
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_order_book("USDTPUSD").await.unwrap_err();
        let err_msg = err.to_string();

        assert!(err_msg.contains("API error: HTTP 400"));
        assert!(err_msg.contains("Invalid symbol"));
        assert!(err_msg.contains("Hint: Binance returned 'Invalid symbol'"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_order_book_no_capability() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_order_book("BTCUSDT").await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("does not support order book"),
            "expected error message to contain 'does not support order book', got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_fetch_order_book_via_post() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/v1/depth")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "symbol": "BTCUSDT"
            })))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "asks": [["50100.0", "2.0"]],
                    "bids": [["50000.0", "1.0"]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut desc = make_http_test_descriptor(&server.url());
        desc.capabilities.order_book = Some(EndpointDescriptor {
            path: "/api/v1/depth".to_string(),
            method: HttpMethod::POST,
            params: std::collections::HashMap::new(),
            request_body: Some(serde_json::json!({
                "symbol": "{pair}"
            })),
            response_root: None,
            interval_map: std::collections::HashMap::new(),
            response: ResponseMapping {
                asks_key: Some("asks".to_string()),
                bids_key: Some("bids".to_string()),
                level_format: Some("positional".to_string()),
                ..Default::default()
            },
        });

        let client = ConfigurableExchangeClient::new(desc);
        let book = client.fetch_order_book("BTCUSDT").await.unwrap();
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks[0].price, 50100.0);
        assert_eq!(book.bids[0].price, 50000.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_order_book_missing_asks_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/depth")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "bids": [["50000.0", "1.0"]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_order_book("BTCUSDT").await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("missing 'asks'"),
            "expected error message to contain 'missing \\'asks\\'', got: {}",
            err_msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_order_book_missing_bids_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/depth")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "asks": [["50010.0", "1.5"]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_order_book("BTCUSDT").await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("missing 'bids'"),
            "expected error message to contain 'missing \\'bids\\'', got: {}",
            err_msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ticker_with_filter() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/tickers")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    {"symbol": "ETHUSDT", "lastPrice": "3000.0"},
                    {"symbol": "BTCUSDT", "lastPrice": "50100.5", "highPrice": "51200.0"},
                    {"symbol": "BNBUSDT", "lastPrice": "400.0"}
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let mut desc = make_http_test_descriptor(&server.url());
        desc.capabilities.ticker = Some(EndpointDescriptor {
            path: "/api/v1/tickers".to_string(),
            method: HttpMethod::GET,
            params: [("symbol".to_string(), "{pair}".to_string())]
                .into_iter()
                .collect(),
            request_body: None,
            response_root: None,
            interval_map: std::collections::HashMap::new(),
            response: ResponseMapping {
                filter: Some(FilterConfig {
                    field: "symbol".to_string(),
                    value: "{pair}".to_string(),
                }),
                last_price: Some("lastPrice".to_string()),
                high_24h: Some("highPrice".to_string()),
                ..Default::default()
            },
        });

        let client = ConfigurableExchangeClient::new(desc);
        let ticker = client.fetch_ticker("BTCUSDT").await.unwrap();
        assert_eq!(ticker.pair, "BTC/USDT");
        assert_eq!(ticker.last_price, Some(50100.5));
        assert_eq!(ticker.high_24h, Some(51200.0));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ticker_filter_no_match() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/tickers")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    {"symbol": "ETHUSDT", "lastPrice": "3000.0"},
                    {"symbol": "BNBUSDT", "lastPrice": "400.0"}
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let mut desc = make_http_test_descriptor(&server.url());
        desc.capabilities.ticker = Some(EndpointDescriptor {
            path: "/api/v1/tickers".to_string(),
            method: HttpMethod::GET,
            params: [("symbol".to_string(), "{pair}".to_string())]
                .into_iter()
                .collect(),
            request_body: None,
            response_root: None,
            interval_map: std::collections::HashMap::new(),
            response: ResponseMapping {
                filter: Some(FilterConfig {
                    field: "symbol".to_string(),
                    value: "{pair}".to_string(),
                }),
                last_price: Some("lastPrice".to_string()),
                ..Default::default()
            },
        });

        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_ticker("BTCUSDT").await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("no ticker found for pair"),
            "expected error message to contain 'no ticker found for pair', got: {}",
            err_msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_trades_non_array_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/trades")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "10".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "trades": [{"price": "50000", "qty": "1"}]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_http_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_recent_trades("BTCUSDT", 10).await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("expected array for trades"),
            "expected error message to contain 'expected array for trades', got: {}",
            err_msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_with_custom_headers() {
        let mut server = mockito::Server::new_async().await;
        let mut headers = std::collections::HashMap::new();
        headers.insert("X-Api-Key".to_string(), "test123".to_string());
        let mock = server
            .mock("GET", "/api/v1/ticker")
            .match_header("x-api-key", "test123")
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "lastPrice": "50100.5",
                    "highPrice": "51200.0",
                    "lowPrice": "48800.0",
                    "volume": "1234.56",
                    "quoteVolume": "62000000.0",
                    "bidPrice": "50095.0",
                    "askPrice": "50105.0"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut desc = make_http_test_descriptor(&server.url());
        desc.headers = headers;
        desc.capabilities.ticker.as_mut().unwrap().response = ResponseMapping {
            last_price: Some("lastPrice".to_string()),
            high_24h: Some("highPrice".to_string()),
            low_24h: Some("lowPrice".to_string()),
            volume_24h: Some("volume".to_string()),
            quote_volume_24h: Some("quoteVolume".to_string()),
            best_bid: Some("bidPrice".to_string()),
            best_ask: Some("askPrice".to_string()),
            ..Default::default()
        };

        let client = ConfigurableExchangeClient::new(desc);
        let ticker = client.fetch_ticker("BTCUSDT").await.unwrap();
        assert_eq!(ticker.last_price, Some(50100.5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_trades_no_capability() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_recent_trades("BTCUSDT", 10).await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("does not support trades"),
            "expected error message to contain 'does not support trades', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_navigate_root_index_out_of_bounds() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"data": [1, 2]});
        let result = client.navigate_root(&json, Some("data.5"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));
    }

    #[test]
    fn test_navigate_root_missing_key() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let json = serde_json::json!({"data": {"nested": 1}});
        let result = client.navigate_root(&json, Some("data.missing_key"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing key"));
    }

    #[test]
    fn test_interpolate_json_passthrough() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        assert_eq!(
            client.interpolate_json(&serde_json::json!(42), "BTC", "100"),
            serde_json::json!(42)
        );
        assert_eq!(
            client.interpolate_json(&serde_json::json!(true), "BTC", "100"),
            serde_json::json!(true)
        );
        assert_eq!(
            client.interpolate_json(&serde_json::json!(null), "BTC", "100"),
            serde_json::json!(null)
        );
    }

    // =================================================================
    // OHLC / kline tests
    // =================================================================

    fn make_ohlc_test_descriptor(base_url: &str) -> VenueDescriptor {
        use crate::market::descriptor::*;
        VenueDescriptor {
            id: "ohlc_mock".to_string(),
            name: "OHLC Mock".to_string(),
            base_url: base_url.to_string(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/klines".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("interval".to_string(), "{interval}".to_string()),
                        ("limit".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        ohlc_format: Some("array_of_arrays".to_string()),
                        ohlc_fields: Some(vec![
                            "open_time".to_string(),
                            "open".to_string(),
                            "high".to_string(),
                            "low".to_string(),
                            "close".to_string(),
                            "volume".to_string(),
                            "close_time".to_string(),
                        ]),
                        ..Default::default()
                    },
                }),
            },
        }
    }

    #[tokio::test]
    async fn test_fetch_ohlc_array_of_arrays() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "3".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    [
                        1700000000000u64,
                        "50000.0",
                        "50500.0",
                        "49800.0",
                        "50200.0",
                        "100.5",
                        1700003599999u64
                    ],
                    [
                        1700003600000u64,
                        "50200.0",
                        "50800.0",
                        "50100.0",
                        "50700.0",
                        "120.3",
                        1700007199999u64
                    ],
                    [
                        1700007200000u64,
                        "50700.0",
                        "51000.0",
                        "50600.0",
                        "50900.0",
                        "95.7",
                        1700010799999u64
                    ]
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_ohlc_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1h", 3).await.unwrap();

        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].open_time, 1700000000000);
        assert_eq!(candles[0].open, 50000.0);
        assert_eq!(candles[0].high, 50500.0);
        assert_eq!(candles[0].low, 49800.0);
        assert_eq!(candles[0].close, 50200.0);
        assert_eq!(candles[0].volume, 100.5);
        assert_eq!(candles[0].close_time, 1700003599999);
        assert_eq!(candles[2].open, 50700.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_object_format() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/candles")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "ETHUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "15m".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "2".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    {
                        "ts": 1700000000000u64,
                        "o": "3000.0",
                        "h": "3050.0",
                        "l": "2980.0",
                        "c": "3020.0",
                        "vol": "500.0",
                        "ct": 1700000899999u64
                    },
                    {
                        "ts": 1700000900000u64,
                        "o": "3020.0",
                        "h": "3080.0",
                        "l": "3010.0",
                        "c": "3060.0",
                        "vol": "420.0",
                        "ct": 1700001799999u64
                    }
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "ohlc_obj_mock".to_string(),
            name: "OHLC Obj Mock".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/candles".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("interval".to_string(), "{interval}".to_string()),
                        ("limit".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        ohlc_format: Some("objects".to_string()),
                        open_time: Some("ts".to_string()),
                        open: Some("o".to_string()),
                        high: Some("h".to_string()),
                        low: Some("l".to_string()),
                        close: Some("c".to_string()),
                        ohlc_volume: Some("vol".to_string()),
                        close_time: Some("ct".to_string()),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("ETHUSDT", "15m", 2).await.unwrap();

        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1700000000000);
        assert_eq!(candles[0].open, 3000.0);
        assert_eq!(candles[0].high, 3050.0);
        assert_eq!(candles[0].low, 2980.0);
        assert_eq!(candles[0].close, 3020.0);
        assert_eq!(candles[0].volume, 500.0);
        assert_eq!(candles[1].close, 3060.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_no_capability() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_ohlc("BTCUSDT", "1h", 100).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does not support OHLC"),
            "expected OHLC error, got: {}",
            msg
        );
    }

    /// Verifies that `interval_map` translates canonical intervals to venue-specific
    /// names before sending the HTTP request (e.g., Biconomy "1h" → "hour").
    #[tokio::test]
    async fn test_fetch_ohlc_interval_map() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        // Expect the mapped interval "hour" rather than the canonical "1h"
        let mock = server
            .mock("GET", "/api/v1/kline")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("type".into(), "hour".into()),
                mockito::Matcher::UrlEncoded("size".into(), "2".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    [
                        1700000000000u64,
                        "50000.0",
                        "50500.0",
                        "49800.0",
                        "50200.0",
                        "100.5"
                    ],
                    [
                        1700003600000u64,
                        "50200.0",
                        "50800.0",
                        "50100.0",
                        "50700.0",
                        "120.3"
                    ]
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "interval_map_test".to_string(),
            name: "Interval Map Test".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/kline".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("type".to_string(), "{interval}".to_string()),
                        ("size".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: [
                        ("1m".to_string(), "1min".to_string()),
                        ("5m".to_string(), "5min".to_string()),
                        ("1h".to_string(), "hour".to_string()),
                        ("1d".to_string(), "day".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    response: ResponseMapping {
                        ohlc_format: Some("array_of_arrays".to_string()),
                        ohlc_fields: Some(vec![
                            "open_time".to_string(),
                            "open".to_string(),
                            "high".to_string(),
                            "low".to_string(),
                            "close".to_string(),
                            "volume".to_string(),
                        ]),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1h", 2).await.unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open, 50000.0);
        assert_eq!(candles[1].close, 50700.0);
        mock.assert_async().await;
    }

    /// When the interval is not in the map, the canonical value passes through.
    #[tokio::test]
    async fn test_fetch_ohlc_interval_map_passthrough() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        // "15m" is not in the interval_map, so it should pass through unchanged
        let mock = server
            .mock("GET", "/api/v1/kline")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("type".into(), "15m".into()),
                mockito::Matcher::UrlEncoded("size".into(), "1".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([[
                    1700000000000u64,
                    "50000.0",
                    "50500.0",
                    "49800.0",
                    "50200.0",
                    "100.5"
                ]])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "passthrough_test".to_string(),
            name: "Passthrough Test".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/kline".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("type".to_string(), "{interval}".to_string()),
                        ("size".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    // Only "1h" → "hour" mapped; "15m" should pass through as-is
                    interval_map: [("1h".to_string(), "hour".to_string())]
                        .into_iter()
                        .collect(),
                    response: ResponseMapping {
                        ohlc_format: Some("array_of_arrays".to_string()),
                        ohlc_fields: Some(vec![
                            "open_time".to_string(),
                            "open".to_string(),
                            "high".to_string(),
                            "low".to_string(),
                            "close".to_string(),
                            "volume".to_string(),
                        ]),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "15m", 1).await.unwrap();
        assert_eq!(candles.len(), 1);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_non_array_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "100".into()),
            ]))
            .with_status(200)
            .with_body(serde_json::json!({"error": "not an array"}).to_string())
            .create_async()
            .await;

        let desc = make_ohlc_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_ohlc("BTCUSDT", "1h", 100).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("expected array for OHLC"),
            "expected array error, got: {}",
            msg
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_empty_array() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1d".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "10".into()),
            ]))
            .with_status(200)
            .with_body("[]")
            .create_async()
            .await;

        let desc = make_ohlc_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1d", 10).await.unwrap();
        assert!(candles.is_empty());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_skips_malformed_inner_items() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "5".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!([
                    "not an array",
                    [
                        1700000000000u64,
                        "50000.0",
                        "50500.0",
                        "49800.0",
                        "50200.0",
                        "100.5",
                        1700003599999u64
                    ],
                    42
                ])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = make_ohlc_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1h", 5).await.unwrap();
        // Only the valid inner array should be parsed
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, 50000.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_with_response_root() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "4h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "2".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "result": [
                        [1700000000000u64, "50000.0", "50500.0", "49800.0", "50200.0", "100.5", 1700003599999u64],
                        [1700003600000u64, "50200.0", "50800.0", "50100.0", "50700.0", "120.3", 1700007199999u64]
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut desc = make_ohlc_test_descriptor(&server.url());
        desc.capabilities.ohlc.as_mut().unwrap().response_root = Some("result".to_string());

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "4h", 2).await.unwrap();
        assert_eq!(candles.len(), 2);
        mock.assert_async().await;
    }

    #[test]
    fn test_interpolate_value_full_with_interval() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let result =
            client.interpolate_value_full("{pair}_{interval}_{limit}", "BTCUSDT", "50", "1h");
        assert_eq!(result, "BTCUSDT_1h_50");
    }

    #[test]
    fn test_interpolate_json_full_with_interval() {
        let desc = make_test_descriptor();
        let client = ConfigurableExchangeClient::new(desc);
        let template = serde_json::json!({
            "symbol": "{pair}",
            "interval": "{interval}",
            "limit": "{limit}"
        });
        let result = client.interpolate_json_full(&template, "ETHUSDT", "100", "15m");
        assert_eq!(result["symbol"], "ETHUSDT");
        assert_eq!(result["interval"], "15m");
        assert_eq!(result["limit"], "100");
    }

    #[tokio::test]
    async fn test_fetch_ohlc_via_post_method() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/v1/klines")
            .with_status(200)
            .with_body(
                serde_json::json!([[
                    1700000000000u64,
                    "50000.0",
                    "50500.0",
                    "49800.0",
                    "50200.0",
                    "100.5",
                    1700003599999u64
                ]])
                .to_string(),
            )
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "post_ohlc".to_string(),
            name: "POST OHLC".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/klines".to_string(),
                    method: HttpMethod::POST,
                    params: std::collections::HashMap::new(),
                    request_body: Some(serde_json::json!({
                        "symbol": "{pair}",
                        "interval": "{interval}",
                        "limit": "{limit}"
                    })),
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        ohlc_format: Some("array_of_arrays".to_string()),
                        ohlc_fields: Some(vec![
                            "open_time".to_string(),
                            "open".to_string(),
                            "high".to_string(),
                            "low".to_string(),
                            "close".to_string(),
                            "volume".to_string(),
                            "close_time".to_string(),
                        ]),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1h", 1).await.unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, 50000.0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_post_http_error() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/v1/klines")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "post_ohlc_err".to_string(),
            name: "POST OHLC Err".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/klines".to_string(),
                    method: HttpMethod::POST,
                    params: std::collections::HashMap::new(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        ohlc_format: Some("array_of_arrays".to_string()),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_ohlc("BTCUSDT", "1h", 100).await.unwrap_err();
        assert!(err.to_string().contains("API error"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_get_http_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/klines")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "100".into()),
            ]))
            .with_status(429)
            .with_body("Rate limited")
            .create_async()
            .await;

        let desc = make_ohlc_test_descriptor(&server.url());
        let client = ConfigurableExchangeClient::new(desc);
        let err = client.fetch_ohlc("BTCUSDT", "1h", 100).await.unwrap_err();
        assert!(err.to_string().contains("API error"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_fetch_ohlc_with_items_key() {
        use crate::market::descriptor::*;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/v1/candles")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into()),
                mockito::Matcher::UrlEncoded("interval".into(), "1h".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "2".into()),
            ]))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "data": [
                        {"ts": 1700000000000u64, "o": "100.0", "h": "110.0", "l": "90.0", "c": "105.0", "vol": "1000.0", "ct": 1700003599999u64},
                        {"ts": 1700003600000u64, "o": "105.0", "h": "115.0", "l": "100.0", "c": "110.0", "vol": "800.0", "ct": 1700007199999u64}
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let desc = VenueDescriptor {
            id: "items_key_ohlc".to_string(),
            name: "Items Key OHLC".to_string(),
            base_url: server.url(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: std::collections::HashMap::new(),
            capabilities: CapabilitySet {
                order_book: None,
                ticker: None,
                trades: None,
                ohlc: Some(EndpointDescriptor {
                    path: "/api/v1/candles".to_string(),
                    method: HttpMethod::GET,
                    params: [
                        ("symbol".to_string(), "{pair}".to_string()),
                        ("interval".to_string(), "{interval}".to_string()),
                        ("limit".to_string(), "{limit}".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    request_body: None,
                    response_root: None,
                    interval_map: std::collections::HashMap::new(),
                    response: ResponseMapping {
                        items_key: Some("data".to_string()),
                        ohlc_format: Some("objects".to_string()),
                        open_time: Some("ts".to_string()),
                        open: Some("o".to_string()),
                        high: Some("h".to_string()),
                        low: Some("l".to_string()),
                        close: Some("c".to_string()),
                        ohlc_volume: Some("vol".to_string()),
                        close_time: Some("ct".to_string()),
                        ..Default::default()
                    },
                }),
            },
        };

        let client = ConfigurableExchangeClient::new(desc);
        let candles = client.fetch_ohlc("BTCUSDT", "1h", 2).await.unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[1].close, 110.0);
        mock.assert_async().await;
    }
}
