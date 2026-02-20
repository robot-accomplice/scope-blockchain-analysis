# Scope + Ghola Integration

[Ghola](https://github.com/robot-accomplice/ghola) is an optional networking sidecar for Scope. While Scope provides the analytical brain, Ghola provides stealthy, resilient HTTP transport built on [fasthttp](https://github.com/valyala/fasthttp).

## What Ghola Adds

| Feature | Description |
|---------|-------------|
| **Temporal Drift** (`-D`) | Cryptographically random jitter in request timing to evade bot detection |
| **Ghost Signing** (`-G`) | Unique `X-Ghola-Identity` header for distributed traceability |
| **Snoop Mode** (`-S`) | Pre-flight WAF/security posture reconnaissance |
| **Chain Shortcuts** (`-c`) | Pre-filled RPC headers for `eth`, `base`, or `solana` ecosystems |
| **Autonomous Retries** (`-r`) | Exponential backoff with configurable base delay |

When enabled, Scope routes HTTP requests through a local ghola sidecar process (`127.0.0.1:18789`) instead of making direct requests. This is useful for high-volume analysis, rate-limited endpoints, or when operating against infrastructure with aggressive bot detection.

## Architecture

Scope uses a trait-based `HttpClient` abstraction (`src/http/mod.rs`) so all chain clients
(Ethereum, Solana, Tron, DexScreener) share the same transport layer. At startup,
`main.rs` reads the `ghola` config section and creates either a `GholaHttpClient` or
a `NativeHttpClient` (plain `reqwest`). The chosen transport is wrapped in
`Arc<dyn HttpClient>` and injected into `DefaultClientFactory`, which passes it
through to every chain client.

```
┌──────────────────────────────────────────────────────────┐
│  main.rs                                                 │
│                                                          │
│  config.ghola.enabled?                                   │
│    ├─ yes ──▶ GholaHttpClient (src/http/ghola.rs)        │
│    │            └──▶ ghola sidecar (127.0.0.1:18789)     │
│    │                   └──▶ target endpoint               │
│    └─ no  ──▶ NativeHttpClient (src/http/native.rs)      │
│                 └──▶ target endpoint (reqwest direct)     │
│                                                          │
│  Arc<dyn HttpClient> ──▶ DefaultClientFactory            │
│    ├── EthereumClient                                    │
│    ├── SolanaClient                                      │
│    ├── TronClient                                        │
│    └── DexClient                                         │
└──────────────────────────────────────────────────────────┘
```

### Key Files

| File | Purpose |
|------|---------|
| `src/http/mod.rs` | `HttpClient` trait, `Request`, `Response` types |
| `src/http/native.rs` | `NativeHttpClient` — direct `reqwest` transport |
| `src/http/ghola.rs` | `GholaHttpClient` — sidecar bridge transport |
| `src/config.rs` | `GholaConfig` struct (`enabled`, `stealth`, `buffer_size`) |
| `src/chains/mod.rs` | `DefaultClientFactory` holds `Arc<dyn HttpClient>` |

### Decoupling Guarantees

- Ghola remains an external Go binary; Scope has zero Go dependencies
- All ghola config fields default to `false`; existing users see no behavior change
- `NativeHttpClient` is the default; ghola is opt-in via `config.yaml`
- If ghola binary is missing, the code falls back gracefully with a warning
- The `HttpClient` trait is Scope-owned; it imports nothing from ghola

## Installation

### Option 1: Go Install (recommended)

```bash
go install github.com/robot-accomplice/ghola/cmd/ghola@latest
```

### Option 2: Download Binary

Pre-built binaries for Linux, macOS, and Windows are available on the [Releases page](https://github.com/robot-accomplice/ghola/releases).

### Option 3: Build from Source

```bash
git clone https://github.com/robot-accomplice/ghola.git
cd ghola
go build -o ghola ./cmd/ghola
cp ghola /usr/local/bin/
```

## Configuration

Enable the ghola sidecar in `~/.config/scope/config.yaml`:

```yaml
ghola:
  enabled: true        # Route HTTP requests through ghola sidecar (default: false)
  stealth: false       # Enable temporal drift + ghost signing (default: false)
  buffer_size: 4096    # Read buffer for large response headers, bytes (default: 4096)
```

The full config schema with ghola:

```yaml
chains:
  api_keys:
    etherscan: "YOUR_KEY"
  ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
  solana_rpc: "https://api.mainnet-beta.solana.com"
  tron_api: "https://api.trongrid.io"

output:
  format: table
  color: true

ghola:
  enabled: true
  stealth: true
  buffer_size: 4096
```

## Verifying the Integration

Run `scope setup --status` to check whether ghola is detected:

```text
├── Ghola Sidecar
│  ✓ ghola binary found in PATH
│  ✓ Ghola transport enabled in config
│  ✓ Stealth mode active (temporal drift + ghost signing)
│  Buffer size     4096 bytes
```

If ghola is not installed:

```text
├── Ghola Sidecar
│  ✗ ghola binary not found in PATH
│  ℹ Install: go install github.com/robot-accomplice/ghola@latest
```

## How It Works

When `ghola.enabled` is `true` in your config, Scope:

1. Creates a `GholaHttpClient` at startup
2. Checks if the ghola sidecar is listening on `127.0.0.1:18789`
3. If not running, attempts to spawn `ghola --serve` automatically
4. Routes all HTTP requests through the sidecar bridge
5. Falls back to native HTTP if the sidecar is unavailable

With `ghola.stealth` enabled, every request through the sidecar additionally gets temporal drift jitter and ghost signing applied.

The `buffer_size` setting (default 4096) controls the read buffer for responses with large headers. If you encounter truncated header errors, increase it (e.g. `8192` or `16384`).

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `⚠ Ghola sidecar enabled but unavailable` | Binary not in PATH | `go install github.com/robot-accomplice/ghola/cmd/ghola@latest` |
| `sidecar did not become ready` | Port 18789 blocked or ghola crash | Check `ghola --serve` manually, verify port availability |
| Scope works but requests feel slow | Sidecar spawning on each run | Run `ghola --serve` in background: `ghola --serve &` |
