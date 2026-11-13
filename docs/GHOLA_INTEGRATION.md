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

### Option 1: Let scope install it (recommended)

Run `scope setup`. Its Ghola step asks whether to download ghola. A command run on a
terminal with `ghola.enabled: true` asks too, once, when no ghola is found.

On a yes, scope downloads the latest release of `robot-accomplice/ghola` from GitHub
over HTTPS, checks the archive's SHA-256 against the release's `checksums.txt`, and
only then extracts `ghola` (`ghola.exe` on Windows) to `~/.scope-bca/bin`. It does
not change PATH; it runs that file by its full path. When the release has no archive
for your system and `go` is on PATH, it offers `go install` into the same folder.

Scope never installs without asking. `--yes`, JSON / CSV / Markdown output, piped
output, the web API and the TUI keep the native transport and print these steps.

`checksums.txt` is not signed: the check proves the archive is the one the release
lists, and HTTPS to GitHub ties the release to its publisher.

### Option 2: Go Install

```bash
go install github.com/robot-accomplice/ghola/cmd/ghola@latest
```

### Option 3: Download Binary

Pre-built binaries for Linux, macOS, and Windows are available on the [Releases page](https://github.com/robot-accomplice/ghola/releases).

### Option 4: Build from Source

```bash
git clone https://github.com/robot-accomplice/ghola.git
cd ghola
go build -o ghola ./cmd/ghola
cp ghola /usr/local/bin/
```

## Configuration

Enable the ghola sidecar in `~/.scope-bca/config.yaml`:

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
│  ✓ ghola binary: /usr/local/bin/ghola
│  ✓ Ghola transport enabled in config
│  ✓ Stealth mode active (temporal drift + ghost signing)
│  Buffer size     4096 bytes
```

If ghola is not installed:

```text
├── Ghola Sidecar
│  ✗ ghola binary not found (PATH, ~/.scope-bca/bin)
│  ℹ Install: run 'scope setup' (it offers to download ghola)
```

## How It Works

When `ghola.enabled` is `true` in your config, Scope:

1. Checks if the ghola sidecar answers `GET /health` on `127.0.0.1:18789` with status 200
2. If not, starts `ghola --serve` from PATH or `~/.scope-bca/bin` and waits up to 5 seconds for it
3. Reads the bridge token (see below)
4. Routes all HTTP requests through the sidecar bridge, with the token as a bearer token
5. Falls back to native HTTP if no sidecar answers or the token cannot be read (and, on a terminal, offers to install ghola when it is missing)

### Bridge token

Newer ghola releases require a token on every `POST /` to the bridge. Each time the bridge
starts, it writes a new token (64 hex characters) to this file:

| System | Token file |
|--------|-----------|
| Linux | `~/.config/ghola/bridge-token` |
| macOS | `~/Library/Application Support/ghola/bridge-token` |
| Windows | `%AppData%\ghola\bridge-token` |

Scope reads the file only after `GET /health` returns 200, and sends the token in the
`Authorization: Bearer` header. The bridge writes the file after it opens the port, so
the file can be older than the port for a short time. When the bridge answers 401, Scope
reads the file once more and sends the request once more. A second 401 is an error that
shows the bridge's reason and the path of the token file that Scope read, with `read` or
`not found`. The path helps when ghola and Scope find the config folder in different
ways. Scope never logs or prints the token.

If the file exists but is empty or not 64 hex characters, Scope prints the
`Ghola sidecar enabled but unavailable` notice and uses native HTTP.

If the file does not exist, Scope sends the request without a token and logs
`no bridge token file; sending without token`. An older ghola release (0.7.0) writes no
token file and checks no token, so ghola mode keeps working with it. A newer release
always writes the file when it starts.

A sidecar that scope started is not left behind silently. At the end of the run scope
asks `Stop the Ghola sidecar that scope started (pid N)? [Y/n]`; with no answer in 15
seconds it stops it. Answer `n` to keep it running for later runs. A run that cannot
ask (scripted output, a pipe, the web server) stops it.

With `ghola.stealth` enabled, every request through the sidecar additionally gets temporal drift jitter and ghost signing applied.

The `buffer_size` setting (default 4096) controls the read buffer for responses with large headers. If you encounter truncated header errors, increase it (e.g. `8192` or `16384`).

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `⚠ Ghola sidecar enabled but unavailable` | ghola not installed | `scope setup`, or `go install github.com/robot-accomplice/ghola/cmd/ghola@latest` |
| `did not answer on 127.0.0.1:18789` | Port 18789 blocked or ghola crash | Check `ghola --serve` manually, verify port availability |
| `does not hold 64 hex characters` | The token file is empty or damaged | Stop `ghola --serve`, start it again, and run Scope again |
| `sidecar refused the request (401) ... (token file: PATH, not found)` | Ghola uses another config folder than Scope | Look for `bridge-token` in the ghola config folder; check `XDG_CONFIG_HOME` |
| `sidecar refused the request (401) ... (token file: PATH, read)` | Token is wrong after a re-read | Stop every `ghola --serve`, start one, and run Scope again |
| Scope works but requests feel slow | Sidecar started on each run | Answer `n` at the exit question, or run `ghola --serve &` yourself |
| No install question any more | You answered no once | Run `scope setup`, or delete `~/.scope-bca/state/ghola-install-declined` |
