# Venue Descriptor Schema

This document describes the YAML schema for exchange venue descriptors used by Scope.

## Overview

Each `.yaml` file in this directory defines how Scope communicates with one exchange's REST API. The `VenueRegistry` loads all built-in descriptors at compile time and any user descriptors from `~/.config/scope/venues/` (Linux) or `~/Library/Application Support/scope/venues/` (macOS) at runtime.

To validate a descriptor: `scope venues validate <file>`

To view the interactive schema: `scope venues schema`

---

## Top-Level Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique lowercase identifier (e.g., `binance`). Used in `--venue` flags. |
| `name` | string | Yes | Human-readable display name (e.g., `Binance Spot`). |
| `base_url` | string | Yes | API base URL (e.g., `https://api.binance.com`). |
| `symbol` | object | Yes | How to format trading pair symbols. |
| `capabilities` | object | Yes | API endpoint definitions (at least one required). |

---

## `symbol` Object

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `template` | string | Yes | — | Pair format with `{base}` and `{quote}` placeholders. Examples: `"{base}{quote}"` → `BTCUSDT`, `"{base}_{quote}"` → `BTC_USDT`, `"{base}-{quote}"` → `BTC-USDT`. |
| `default_quote` | string | Yes | — | Quote currency when the user provides only a base symbol (usually `USDT`). |
| `case` | string | No | `upper` | Case transformation: `upper` or `lower`. Applied after placeholder substitution. |

---

## `capabilities` Object

Each capability is optional. Omit a capability if the exchange doesn't support it.

| Capability | Description |
|-----------|-------------|
| `order_book` | Level-2 order book / depth endpoint |
| `ticker` | 24h ticker / market stats endpoint |
| `trades` | Recent trades endpoint |

---

## Endpoint Descriptor (each capability)

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `method` | string | No | `GET` | HTTP method: `GET` or `POST`. |
| `path` | string | Yes | — | URL path appended to `base_url`. |
| `params` | map | No | `{}` | Query parameters (for GET). Values support placeholders: `{pair}`, `{limit}`, `{base}`, `{quote}`. |
| `request_body` | object | No | — | JSON body template for POST requests. Supports the same placeholders in string values. |
| `response_root` | string | No | — | Dot-path to navigate the JSON response before field mapping. Special values: `"data"` → `json["data"]`, `"data.0"` → `json["data"][0]`, `"result.*"` → first value under `json["result"]`. |
| `response` | object | Yes | — | Field mappings for parsing the response. |

---

## `response` Object — Order Book Fields

| Field | Type | Description |
|-------|------|-------------|
| `asks_key` | string | JSON key for the asks array. |
| `bids_key` | string | JSON key for the bids array. |
| `level_format` | string | `"positional"` (default) for `[price, qty]` arrays, or `"object"` for `{price: x, size: y}` objects. |
| `level_price_field` | string | Field name for price when `level_format = "object"`. |
| `level_size_field` | string | Field name for quantity when `level_format = "object"`. |

---

## `response` Object — Ticker Fields

| Field | Type | Description |
|-------|------|-------------|
| `last_price` | string | JSON field for last trade price. Supports dot-notation (e.g., `c.0` for Kraken). |
| `high_24h` | string | JSON field for 24h high. |
| `low_24h` | string | JSON field for 24h low. |
| `volume_24h` | string | JSON field for 24h base volume. |
| `quote_volume_24h` | string | JSON field for 24h quote volume. |
| `best_bid` | string | JSON field for best bid price. |
| `best_ask` | string | JSON field for best ask price. |

---

## `response` Object — Trade Fields

| Field | Type | Description |
|-------|------|-------------|
| `items_key` | string | JSON key containing the trades array. Omit if the root is the array. |
| `price` | string | Field for trade price. |
| `quantity` | string | Field for trade quantity. |
| `quote_quantity` | string | Field for trade quote quantity. |
| `timestamp_ms` | string | Field for trade timestamp (epoch milliseconds). |
| `id` | string | Field for trade ID. |
| `side` | object | Side detection configuration (see below). |
| `filter` | object | Filter for multi-pair endpoints (see below). |

### `side` Object

| Field | Type | Description |
|-------|------|-------------|
| `field` | string | JSON field containing the side indicator. |
| `mapping` | map | Maps venue-specific values to `"buy"` or `"sell"`. Example: `{"true": "buy", "false": "sell"}` for `isBuyerMaker`. |

### `filter` Object

Used when an endpoint returns data for all pairs and you need to filter.

| Field | Type | Description |
|-------|------|-------------|
| `field` | string | Response field to match against. |
| `value` | string | Expected value (supports `{pair}` placeholder). |

---

## Minimal Example

```yaml
id: my_exchange
name: My Exchange
base_url: https://api.myexchange.com

symbol:
  template: "{base}{quote}"
  default_quote: USDT

capabilities:
  order_book:
    path: /api/depth
    params:
      symbol: "{pair}"
      limit: "100"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
```

## Full Example (all capabilities)

```yaml
id: full_example
name: Full Example Exchange
base_url: https://api.example.com

symbol:
  template: "{base}_{quote}"
  default_quote: USDT
  case: upper

capabilities:
  order_book:
    path: /api/v1/depth
    params:
      symbol: "{pair}"
      limit: "{limit}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional

  ticker:
    path: /api/v1/ticker/24hr
    params:
      symbol: "{pair}"
    response:
      last_price: lastPrice
      high_24h: highPrice
      low_24h: lowPrice
      volume_24h: volume
      quote_volume_24h: quoteVolume

  trades:
    path: /api/v1/trades
    params:
      symbol: "{pair}"
      limit: "{limit}"
    response:
      price: price
      quantity: qty
      timestamp_ms: time
      side:
        field: isBuyerMaker
        mapping:
          "true": sell
          "false": buy
```

---

## Validation

```bash
scope venues validate my-exchange.yaml
```

This checks:
- Valid YAML syntax
- All required fields present
- At least one capability defined
- Symbol template contains `{base}` placeholder
