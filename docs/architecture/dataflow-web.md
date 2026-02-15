# Web Mode Data Flow

## Overview

The web mode (`scope web`) starts a local HTTP server that mirrors all CLI functionality
through a REST API and single-page web UI. It reuses the same `Config` and `DefaultClientFactory`
as the CLI, ensuring identical behavior.

## Request Flow

```mermaid
sequenceDiagram
    participant Browser
    participant Axum as Axum Server
    participant API as API Handlers
    participant Core as Scope Core
    participant External as External APIs

    Browser->>Axum: HTTP Request (GET/POST)
    Axum->>API: Route to handler
    API->>Core: Call core functions<br/>(analyze_address, fetch_tx_report, etc.)
    Core->>External: Fetch data<br/>(RPC, DexScreener, Etherscan)
    External-->>Core: Raw data
    Core-->>API: Structured data (AddressReport, etc.)
    API-->>Axum: JSON response
    Axum-->>Browser: HTTP Response
```

## WebSocket Monitor Flow

```mermaid
sequenceDiagram
    participant Browser
    participant WS as WebSocket Handler
    participant DexClient as DexScreener Client

    Browser->>WS: WS Connect (/ws/monitor?token=USDC)
    WS-->>Browser: {"type":"connected"}

    loop Every N seconds
        WS->>DexClient: get_token_data(chain, token)
        DexClient-->>WS: DexTokenData
        WS-->>Browser: {"type":"update", "price_usd":..., ...}
    end

    Browser->>WS: Close
```

## API Endpoints

| Endpoint                    | Method | Handler                     | Core Function                    |
|-----------------------------|--------|-----------------------------|----------------------------------|
| `GET /`                     | GET    | `serve_ui`                  | Static HTML/CSS/JS               |
| `POST /api/address`         | POST   | `api::address::handle`      | `address::analyze_address`       |
| `POST /api/tx`              | POST   | `api::tx::handle`           | `tx::fetch_transaction_report`   |
| `POST /api/insights`        | POST   | `api::insights::handle`     | `insights::infer_target` + core  |
| `POST /api/crawl`           | POST   | `api::crawl::handle`        | `crawl::fetch_analytics_for_input` |
| `GET /api/discover`         | GET    | `api::discover::handle`     | `DexClient::get_token_profiles`  |
| `POST /api/token-health`    | POST   | `api::token_health::handle` | `crawl::fetch_analytics_for_input` + market |
| `POST /api/market/summary`  | POST   | `api::market::handle`       | `OrderBookClient::fetch_order_book` |
| `POST /api/exchange/snapshot` | POST | `api::exchange::handle`    | Full market snapshot (order book, ticker, trades) |
| `POST /api/exchange/ohlc`   | POST   | `api::exchange::handle_ohlc` | OHLC/candlestick data for venue/pair |
| `POST /api/exchange/trades` | POST   | `api::exchange::handle_trades` | Recent trades for venue/pair |
| `GET /api/portfolio/list`   | GET    | `api::portfolio::handle_list` | `Portfolio::load`              |
| `POST /api/portfolio/add`   | POST   | `api::portfolio::handle_add`  | `Portfolio::add_address`       |
| `POST /api/export`          | POST   | `api::export::handle`       | `ChainClient::get_*`             |
| `POST /api/compliance/risk` | POST   | `api::compliance::handle_risk` | `RiskEngine::assess_address`  |
| `GET /api/config/status`    | GET    | `api::config_status::handle`  | Config inspection              |
| `POST /api/config`          | POST   | `api::config_status::handle_save` | Config write               |
| `WS /ws/monitor`            | WS     | `monitor::ws_handler`       | `DexDataSource::get_token_data`  |

## Application State

```mermaid
classDiagram
    class AppState {
        +Config config
        +DefaultClientFactory factory
    }
    class Config {
        +ChainsConfig chains
        +OutputConfig output
        +PortfolioConfig portfolio
    }
    class DefaultClientFactory {
        +ChainsConfig chains_config
        +create_chain_client(chain) Box~ChainClient~
        +create_dex_client() Box~DexDataSource~
    }
    AppState --> Config
    AppState --> DefaultClientFactory
```

## Daemon Mode

```mermaid
flowchart TD
    A[scope web --daemon] --> B{Fork child process}
    B --> C[Write PID to ~/.local/share/scope/scope-web.pid]
    B --> D[Redirect output to scope-web.log]
    C --> E[Parent exits]
    D --> F[Child runs Axum server]

    G[scope web --stop] --> H[Read PID file]
    H --> I[Send SIGTERM]
    I --> J[Remove PID file]
```

## Security

- **Bind**: `127.0.0.1` by default (localhost only)
- **Auth**: None (local use only)
- **API Keys**: Stored on disk, never exposed to browser
- **CORS**: Permissive (same-origin expected)
