# TUI Test Harness: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route all scope HTTP through one shared transport, add a record/replay transport, and build a PTY harness that runs YAML scenarios against the real `scope tui`.

**Architecture:** `scope::http::shared` builds the one transport (replay, record, Ghola or native) and installs it for the process; every client that built its own `reqwest` client calls `transport()` instead. `ReplayHttpClient` serves or records JSON fixtures. A new unpublished crate, `scope-tui-harness`, drives `scope tui --no-banner` in a PTY, models the screen with `vt100`, runs scenario steps and writes evidence on failure.

**Tech Stack:** Rust 2024 workspace, `reqwest` 0.12 (rustls), `async-trait`, `serde`/`serde_json`/`serde_yaml` 0.9, `sha2` 0.10, `portable-pty` 0.9, `vt100` 0.16, `regex` 1, `clap` 4, `tempfile` 3.10, `anyhow` 1.

**Spec:** `docs/design/2026-10-12-tui-test-harness-design.md`

## Global Constraints

- Commit as `robot-accomplice <robot@accomplice.ch>`: `git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit …`. No Claude or Anthropic attribution.
- Each spec build step is one PR into `develop` (gitflow). Branch names: `feature/shared-http-transport`, `feature/replay-transport`, `feature/tui-harness`.
- Gates before every commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Never mutate a process-global env var (`HOME`, `SCOPE_HTTP_*`) inside an in-process test. Inject an env lookup function instead. The harness sets env only on the child process.
- Public signatures of existing constructors do not change (spec 2.2). Behavior changes only in which transport they use.
- Error text that users see does not change (for example the `format_http_api_error` messages and the Binance `-1121` hint).
- Replay mode never reaches the network. A miss is an error naming the method and URL.
- Prose in comments, docs, commits and PRs: Simplified Technical English.

## Review Focus

1. **A transport used before `init`:** a client built before `dispatch` runs (for example during config load) gets native, not the configured transport. Expect: `transport()` logs the fallback; `init` called afterwards returns the installed native with a notice that the configured transport was not applied, so the CLI prints it. Task 2 pins it (`shared_transport_fallback.rs`).
2. **Same request repeated in replay (`--every`):** the second and later calls return the next recorded response, then the last one again. Expect no miss error after the list runs out. Task 8 pins it.
3. **Secret in a URL query in a different case (`ApiKey=…`):** removal is case-insensitive, and the key and stored URL never contain it. Task 7 pins it.
4. **A scenario that waits for text that never appears:** the step fails at its timeout with the last screen in the evidence; the child process is killed, and the run continues with the next scenario. Task 12 pins it.
5. **The child exits early (crash or `exit`) while steps remain:** the next step fails with "process exited with status N" instead of hanging. Task 12 pins it.

---

## Part 1: One shared transport (spec step 1). Branch `feature/shared-http-transport`.

### Task 1: Per-request timeout

**Files:**
- Modify: `crates/scope-core/src/http/mod.rs` (struct `Request`, its constructors)
- Modify: `crates/scope-core/src/http/native.rs` (`send`)

**Interfaces:**
- Produces: `Request.timeout: Option<std::time::Duration>`, `Request::with_timeout(self, Duration) -> Self`. `NativeHttpClient` applies it with `RequestBuilder::timeout`. `GholaHttpClient` ignores it (the sidecar has its own timeout).

- [ ] **Step 1: Write the failing tests** in `http/mod.rs` `mod tests`:

```rust
#[test]
fn test_request_timeout_defaults_to_none() {
    assert!(Request::get("https://example.com").timeout.is_none());
    assert!(Request::post_json("https://example.com", "{}").timeout.is_none());
}

#[test]
fn test_request_with_timeout_sets_it() {
    let req = Request::get("https://example.com").with_timeout(std::time::Duration::from_secs(7));
    assert_eq!(req.timeout, Some(std::time::Duration::from_secs(7)));
}
```

and in `http/native.rs` tests (mockito is a dev-dependency of scope-core; check `crates/scope-core/Cargo.toml` and add `mockito = "1.2"` under `[dev-dependencies]` if missing):

```rust
#[tokio::test]
async fn test_native_applies_request_timeout() {
    // A server that answers after 2 s must fail a 200 ms request.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/slow")
        .with_chunked_body(|w| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            w.write_all(b"late")
        })
        .create_async()
        .await;
    let client = NativeHttpClient::new().unwrap();
    let req = Request::get(&format!("{}/slow", server.url()))
        .with_timeout(std::time::Duration::from_millis(200));
    assert!(client.send(req).await.is_err());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib http::`
Expected: compile errors (`no field timeout`, `no method with_timeout`).

- [ ] **Step 3: Implement**

In `Request`: add the field with a doc comment `/// Overrides the transport's default timeout for this request.` and `pub timeout: Option<std::time::Duration>,`. Set `timeout: None` in `get` and `post_json`. Add:

```rust
/// Sets a timeout for this request only.
pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
    self.timeout = Some(timeout);
    self
}
```

In `NativeHttpClient::send`, after the headers loop:

```rust
if let Some(t) = request.timeout {
    builder = builder.timeout(t);
}
```

Fix any struct literal of `Request { … }` that the compiler reports by adding `timeout: None`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-core --lib http::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-core/src/http crates/scope-core/Cargo.toml
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(http): add a per-request timeout"
```

### Task 2: `http::shared` (build, init, transport)

**Files:**
- Create: `crates/scope-core/src/http/shared.rs`
- Modify: `crates/scope-core/src/http/mod.rs` (`pub mod shared; pub use shared::{init, transport, Built};`)
- Test: `crates/scope-core/tests/shared_transport.rs` (own process, so the global is not shared with other tests)

**Interfaces:**
- Consumes: `crate::config::Config` (`ghola.enabled`, `ghola.stealth`, `ghola.buffer_size`), `GholaHttpClient::new(bool, u32)`, `NativeHttpClient::new()`.
- Produces:
  - `pub struct Built { pub client: Arc<dyn HttpClient>, pub notices: Vec<String> }`
  - `pub fn build_with(config: &Config, env: &dyn Fn(&str) -> Option<String>) -> Result<Built, ScopeError>`
  - `pub fn build(config: &Config) -> Result<Built, ScopeError>` (env = `std::env::var(k).ok()`)
  - `pub fn init(config: &Config) -> Result<Built, ScopeError>`
  - `pub fn transport() -> Arc<dyn HttpClient>`
  - `pub fn describe() -> &'static str` returns `"native"`, `"ghola"`, `"replay"` or `"record"` for the installed transport (`"none"` before install). Used in tests and logs.

- [ ] **Step 1: Write the failing unit tests** in `shared.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> { None }

    #[test]
    fn test_build_default_is_native_with_no_notices() {
        let built = build_with(&Config::default(), &no_env).unwrap();
        assert!(built.notices.is_empty());
        assert_eq!(built.kind, Kind::Native);
    }

    #[test]
    fn test_build_ghola_enabled_reports_its_choice() {
        // The test machine may or may not run a Ghola sidecar. Either way,
        // the result must say which transport it chose: Ghola with no
        // notice, or native with a notice that names Ghola.
        let mut config = Config::default();
        config.ghola.enabled = true;
        let built = build_with(&config, &no_env).unwrap();
        match built.kind {
            Kind::Ghola => assert!(built.notices.is_empty()),
            Kind::Native => assert!(built.notices.iter().any(|n| n.contains("Ghola"))),
            other => panic!("unexpected kind {other:?}"),
        }
    }
}
```

`Built` also carries `pub kind: Kind` with `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Kind { Native, Ghola, Replay, Record }`.

- [ ] **Step 2: Write the failing integration test** `crates/scope-core/tests/shared_transport.rs`:

```rust
//! The shared transport is process-global, so these checks run in their
//! own test binary.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_installs_once_and_transport_returns_it() {
    assert_eq!(shared::describe(), "none");
    let first = shared::init(&Config::default()).unwrap();
    assert_eq!(shared::describe(), "native");
    // A second init does not rebuild and reports no notices.
    let mut cfg = Config::default();
    cfg.ghola.enabled = true;
    let second = shared::init(&cfg).unwrap();
    assert!(second.notices.is_empty());
    assert!(std::sync::Arc::ptr_eq(&first.client, &second.client));
    assert!(std::sync::Arc::ptr_eq(&first.client, &shared::transport()));
}
```

and `crates/scope-core/tests/shared_transport_fallback.rs` (a second binary, so its global starts empty):

```rust
//! Review Focus 1: a client used before `init` gets native, and `init`
//! then says that the configured transport was not applied.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_after_fallback_reports_that_config_was_ignored() {
    let _early = shared::transport();
    assert_eq!(shared::describe(), "native");
    let mut cfg = Config::default();
    cfg.ghola.enabled = true;
    let built = shared::init(&cfg).unwrap();
    assert_eq!(shared::describe(), "native");
    assert!(
        built.notices.iter().any(|n| n.contains("not applied")),
        "{:?}",
        built.notices
    );
}
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib http::shared && cargo test -p scope-bca-core --test shared_transport --test shared_transport_fallback`
Expected: compile error, `shared` does not exist.

- [ ] **Step 4: Implement `shared.rs`**

```rust
//! # Shared HTTP transport
//!
//! The one place that selects the HTTP transport. `init` builds it from
//! config and installs it for the process. Clients that do not get a
//! transport passed in call `transport()`. See the TUI test harness
//! design, section 2.2.

use super::{GholaHttpClient, HttpClient, NativeHttpClient};
use crate::config::Config;
use crate::error::ScopeError;
use std::sync::{Arc, OnceLock};

/// Which transport was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Native,
    Ghola,
    Replay,
    Record,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Native => "native",
            Kind::Ghola => "ghola",
            Kind::Replay => "replay",
            Kind::Record => "record",
        }
    }
}

/// A built transport and the notices the caller must show the user.
#[derive(Clone)]
pub struct Built {
    pub client: Arc<dyn HttpClient>,
    pub kind: Kind,
    pub notices: Vec<String>,
}

static INSTALLED: OnceLock<(Arc<dyn HttpClient>, Kind)> = OnceLock::new();
/// True when `transport()` installed native because nothing was installed.
static FELL_BACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Builds the transport the config and environment select.
pub fn build(config: &Config) -> Result<Built, ScopeError> {
    build_with(config, &|k| std::env::var(k).ok())
}

/// Builds the transport, reading environment variables through `env`.
///
/// Order: replay or record (`SCOPE_HTTP_MODE`), else Ghola when enabled
/// and available, else native.
pub fn build_with(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Built, ScopeError> {
    let _ = env; // replay and record selection are added in Task 9
    let mut notices = Vec::new();
    if config.ghola.enabled {
        match GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size) {
            Ok(client) => {
                tracing::info!("Using Ghola sidecar for HTTP transport");
                return Ok(Built { client: Arc::new(client), kind: Kind::Ghola, notices });
            }
            Err(e) => {
                notices.push(format!("⚠ Ghola sidecar enabled but unavailable: {e}"));
                notices.push("  Install: go install github.com/robot-accomplice/ghola@latest".into());
                notices.push("  Falling back to native HTTP transport".into());
            }
        }
    }
    Ok(Built { client: Arc::new(NativeHttpClient::new()?), kind: Kind::Native, notices })
}

/// Builds and installs the transport the first time; later calls return
/// the installed transport with no notices.
pub fn init(config: &Config) -> Result<Built, ScopeError> {
    if let Some((client, kind)) = INSTALLED.get() {
        let mut notices = Vec::new();
        if FELL_BACK.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let msg = format!(
                "⚠ HTTP was used before startup; the {} transport from config was not applied",
                build(config).map_or("configured", |b| b.kind.name())
            );
            tracing::warn!("{msg}");
            notices.push(msg);
        }
        return Ok(Built { client: client.clone(), kind: *kind, notices });
    }
    let built = build(config)?;
    match INSTALLED.set((built.client.clone(), built.kind)) {
        Ok(()) => Ok(built),
        // Another thread installed first: use its transport.
        Err(_) => init(config),
    }
}

/// The installed transport. Installs native if nothing is installed.
pub fn transport() -> Arc<dyn HttpClient> {
    if let Some((client, _)) = INSTALLED.get() {
        return client.clone();
    }
    tracing::debug!("no HTTP transport installed; using native");
    let native: Arc<dyn HttpClient> =
        Arc::new(NativeHttpClient::new().expect("failed to build HTTP client"));
    if INSTALLED.set((native, Kind::Native)).is_ok() {
        FELL_BACK.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    INSTALLED.get().expect("installed above").0.clone()
}

/// The installed transport kind, or `"none"`.
pub fn describe() -> &'static str {
    INSTALLED.get().map_or("none", |(_, k)| k.name())
}
```

Add to `http/mod.rs`: `pub mod shared;` and `pub use shared::{Built, init, transport};`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p scope-bca-core --lib http::shared && cargo test -p scope-bca-core --test shared_transport --test shared_transport_fallback`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/scope-core/src/http crates/scope-core/tests/shared_transport.rs crates/scope-core/tests/shared_transport_fallback.rs
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(http): add the shared transport (build, init, transport)"
```

### Task 3: One transport selection for CLI and web

**Files:**
- Modify: `crates/scope-cli/src/cli/dispatch.rs:30-37` (`dispatch`) and `:135-162` (`create_http_client`), and its tests at `:181-201`
- Modify: `crates/scope-web/src/lib.rs:104-115` (`start_server`)

**Interfaces:**
- Consumes: `scope::http::init(&Config) -> Result<Built, ScopeError>`.
- Produces: `create_http_client(config, out) -> std::io::Result<Arc<dyn HttpClient>>` keeps its signature and now calls `init`, printing each notice with `errln!`.

- [ ] **Step 1: Write the failing test** in `dispatch.rs` tests:

```rust
#[test]
fn test_create_http_client_returns_the_installed_transport() {
    // Two calls must give the same transport: one process, one transport.
    let config = Config::default();
    let a = create_http_client(&config, &quiet()).unwrap();
    let b = create_http_client(&config, &quiet()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p scope-bca-cli --lib dispatch::tests::test_create_http_client_returns_the_installed_transport`
Expected: FAIL (`ptr_eq` false: each call builds a new client).

- [ ] **Step 3: Implement**

Replace the body of `create_http_client`:

```rust
pub fn create_http_client(config: &Config, out: &Output) -> std::io::Result<Arc<dyn HttpClient>> {
    let built = scope::http::init(config)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    for notice in &built.notices {
        errln!(out, "  {notice}")?;
    }
    Ok(built.client)
}
```

Update its doc comment: "Installs the process transport on the first call (see `scope::http::shared`) and prints the notices. Later calls return the same transport."

In `scope-web/src/lib.rs` `start_server`, replace the Ghola/native block:

```rust
let built = scope::http::init(&config)?;
for notice in &built.notices {
    tracing::warn!("{notice}");
}
let http = built.client;
```

Keep the three existing `create_http_client` tests; adjust any assertion that depended on a fresh client per call.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-cli --lib dispatch:: && cargo test -p scope-bca --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-cli/src/cli/dispatch.rs crates/scope-web/src/lib.rs
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "refactor(http): select the transport in one place for CLI and web"
```

### Task 4: Exchange venue client through the transport

**Files:**
- Modify: `crates/scope-core/src/market/configurable_client.rs:14-150, 205-260` (field `http`, `new`, `format_http_api_error`, both request functions)
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: `crate::http::{transport, HttpClient, Request, Response}`, `Request::with_timeout`.
- Produces: `ConfigurableExchangeClient::new(descriptor)` unchanged signature. New `pub fn with_http(descriptor: VenueDescriptor, http: Arc<dyn HttpClient>) -> Self` (used by tests). `Debug` stays derived by implementing it by hand for the `http` field (`Arc<dyn HttpClient>` is not `Debug`): print `descriptor` only.

- [ ] **Step 1: Write the failing tests** in `configurable_client.rs` tests:

```rust
use crate::http::{HttpClient, Request, Response};
use std::sync::{Arc, Mutex};

/// Records requests and answers with a fixed response.
struct Spy { seen: Mutex<Vec<Request>>, status: u16, body: String }

#[async_trait::async_trait]
impl HttpClient for Spy {
    async fn send(&self, req: Request) -> crate::error::Result<Response> {
        self.seen.lock().unwrap().push(req);
        Ok(Response { status_code: self.status, headers: Default::default(), body: self.body.clone() })
    }
}

fn binance_like() -> VenueDescriptor {
    // Use the built-in binance descriptor so the test follows real config.
    crate::market::VenueRegistry::load().unwrap().get("binance").unwrap().clone()
}

#[tokio::test]
async fn test_venue_requests_go_through_the_given_transport() {
    let spy = Arc::new(Spy { seen: Mutex::new(vec![]), status: 200, body: r#"{"bids":[],"asks":[]}"#.into() });
    let client = ConfigurableExchangeClient::with_http(binance_like(), spy.clone());
    let _ = client.fetch_order_book("USDCUSDT").await;
    let seen = spy.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].url.contains("symbol=USDCUSDT"), "{}", seen[0].url);
    assert_eq!(seen[0].timeout, Some(std::time::Duration::from_secs(binance_like().timeout_secs.unwrap_or(15))));
}

#[tokio::test]
async fn test_venue_http_error_text_is_unchanged() {
    // The Binance -1121 hint must survive the move off reqwest::StatusCode.
    let spy = Arc::new(Spy { seen: Mutex::new(vec![]), status: 400, body: r#"{"code":-1121,"msg":"Invalid symbol."}"#.into() });
    let client = ConfigurableExchangeClient::with_http(binance_like(), spy);
    let err = client.fetch_order_book("BADPAIR").await.unwrap_err().to_string();
    assert!(err.contains("HTTP 400 Bad Request"), "{err}");
    assert!(err.contains("Invalid symbol"), "{err}");
}
```

Use the trait method names the file already implements (`fetch_order_book` comes from `OrderBookClient`; import the trait in the test module).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib market::configurable_client`
Expected: compile error (`with_http` missing).

- [ ] **Step 3: Implement**

1. Replace `use reqwest::Client;` with `use crate::http::{HttpClient, Request};` and `use std::sync::Arc;`. Field `http: Arc<dyn HttpClient>`.
2. `new(descriptor)` → `Self::with_http(descriptor, crate::http::transport())`. `with_http` stores both.
3. Add `fn timeout(&self) -> Duration { Duration::from_secs(self.descriptor.timeout_secs.unwrap_or(15)) }`.
4. `format_http_api_error(venue: &str, status: u16, body: &str)`: at the top, `let status = reqwest::StatusCode::from_u16(status).map(|s| s.to_string()).unwrap_or_else(|_| status.to_string());` so the text stays `HTTP 400 Bad Request`.
5. In both request functions, build a `Request` instead of a `RequestBuilder`. GET:

```rust
let mut url = reqwest::Url::parse(&url)
    .map_err(|e| ScopeError::Chain(format!("{} invalid URL {url}: {e}", self.descriptor.name)))?;
if !params.is_empty() {
    url.query_pairs_mut().extend_pairs(params.iter());
}
let mut req = Request::get(url.as_str()).with_timeout(self.timeout());
for (k, v) in &self.descriptor.headers {
    req = req.with_header(k, v);
}
```

POST:

```rust
let body = endpoint
    .request_body
    .as_ref()
    .map(|t| self.interpolate_json(t, pair, &limit_str))
    .unwrap_or(Value::Null);
let mut req = Request::post_json(&url, body.to_string()).with_timeout(self.timeout());
for (k, v) in &self.descriptor.headers {
    req = req.with_header(k, v);
}
```

(In the OHLC function use `interpolate_json_full(t, pair, &limit_str, interval)` and `interpolate_value_full`, as today.) Then for both:

```rust
let resp = self.http.send(req).await?;
if !resp.is_success() {
    return Err(format_http_api_error(&self.descriptor.name, resp.status_code, &resp.body));
}
serde_json::from_str::<Value>(&resp.body).map_err(|e| {
    ScopeError::Chain(format!("{} JSON parse error: {}", self.descriptor.name, e))
})
```

A POST with no `request_body` sent no body before. Keep that: only call `post_json` when `request_body` is `Some`; otherwise build `Request { method: "POST".into(), body: None, ..Request::get(&url) }`.

6. Replace `#[derive(Debug, Clone)]` with `#[derive(Clone)]` and add:

```rust
impl std::fmt::Debug for ConfigurableExchangeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigurableExchangeClient").field("descriptor", &self.descriptor).finish_non_exhaustive()
    }
}
```

- [ ] **Step 4: Run the tests** (all 83 existing tests in the file must still pass; they use mockito servers through `base_url`, which now flow through `transport()` = native)

Run: `cargo test -p scope-bca-core --lib market::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-core/src/market/configurable_client.rs
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "fix(market): send venue requests through the shared transport

Venue calls used a private reqwest client and skipped Ghola."
```

### Task 5: Remaining self-building constructors

**Files:**
- Modify: `crates/scope-core/src/chains/dex.rs:365`, `chains/ethereum.rs:241, 323, 1319`, `chains/solana.rs:263`, `chains/tron.rs:171`, `compliance/datasource.rs:80, 89`
- Modify: `crates/scope-cli/src/cli/contract.rs:107`, `crates/scope-web/src/api/contract.rs:64`

**Interfaces:**
- Consumes: `crate::http::transport()` (core), `scope::http::transport()` (cli, web).
- Produces: no new names. Constructors that are `#[cfg(test)]` or `with_base_url`/`with_rpc_url`/`with_api_url` (test helpers pointing at mock servers) may keep `NativeHttpClient::new()` only if they are `#[cfg(any(test, feature = "test-support"))]`; otherwise change them too.

- [ ] **Step 1: Write the failing test** in `chains/dex.rs` tests:

```rust
#[test]
fn test_dex_client_new_uses_the_shared_transport() {
    let shared = crate::http::transport();
    let client = DexClient::new();
    assert!(std::sync::Arc::ptr_eq(&client.http, &shared));
}
```

(If the field is private to the module, the test lives in the same module, so it can read it.)

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p scope-bca-core --lib chains::dex::tests::test_dex_client_new_uses_the_shared_transport`
Expected: FAIL.

- [ ] **Step 3: Implement**

At each listed line, replace `Arc::new(crate::http::NativeHttpClient::new()…)` (and its `?`, `expect` or `map_err` tail) with `crate::http::transport()`. Where the old code returned `Result` only because of the client build, keep the `Result` signature (public API) and return `Ok`. In the two contract files, replace

```rust
let http_client = scope::http::NativeHttpClient::new()?;
```

with `let http_client = scope::http::transport();` and pass `http_client.as_ref()` where a `&dyn HttpClient` is expected.

For `with_base_url` (ethereum 271), `with_rpc_url` (solana 290), `with_api_url` (tron 198), `datasource.rs:89`: if the function is not already gated by `#[cfg(any(test, feature = "test-support"))]`, switch it to `transport()` as well.

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-core/src crates/scope-cli/src/cli/contract.rs crates/scope-web/src/api/contract.rs
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "fix(http): route DEX, chain, compliance and contract clients through the shared transport"
```

### Task 6: Bypass guard test

**Files:**
- Create: `crates/scope-core/tests/no_transport_bypass.rs`

**Interfaces:**
- Consumes: the workspace source tree (`CARGO_MANIFEST_DIR/../..`).
- Produces: a test that fails the build when a new bypass appears.

- [ ] **Step 1: Write the test**

```rust
//! Every HTTP request must use the shared transport (TUI test harness
//! design, section 2.2). This test fails when production code builds its
//! own client.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["reqwest::Client", "NativeHttpClient::new"];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            let name = p.file_name().unwrap().to_string_lossy();
            if name != "target" && name != "tests" {
                rs_files(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The production part of a file: everything before the first test module.
fn production_part(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

#[test]
fn test_no_http_client_is_built_outside_the_http_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join("crates");
    let http_dir = root.join("scope-core/src/http");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut hits = Vec::new();
    for f in files.iter().filter(|f| !f.starts_with(&http_dir)) {
        let src = std::fs::read_to_string(f).unwrap();
        for (n, line) in production_part(&src).lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if FORBIDDEN.iter().any(|pat| code.contains(pat)) {
                hits.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(hits.is_empty(), "HTTP client built outside scope::http:\n{}", hits.join("\n"));
}
```

`#[cfg(any(test, feature = "test-support"))]` helpers sit before the test module in some files. If the test reports them, move each into the file's test-support section or switch it to `transport()`; do not add an allow-list.

- [ ] **Step 2: Run it**

Run: `cargo test -p scope-bca-core --test no_transport_bypass`
Expected: PASS after Tasks 4 and 5. If it fails, the message lists each remaining site; fix them, then re-run.

- [ ] **Step 3: Prove it can fail**

Temporarily add `let _c = reqwest::Client::new();` to the top of `crates/scope-cli/src/cli/venues.rs` production code, run the test, confirm it FAILS and names that line, then remove the line.

- [ ] **Step 4: Run all gates, commit, push, open the PR**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/scope-core/tests/no_transport_bypass.rs
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "test(http): fail the build when code bypasses the shared transport"
GH_TOKEN=$(gh auth token --user robot-accomplice) git push -u origin feature/shared-http-transport
GH_TOKEN=$(gh auth token --user robot-accomplice) gh pr create --repo robot-accomplice/scope-blockchain-analysis --base develop --title "Route all HTTP through one shared transport" --body "Implements the TUI test harness design, section 2. Refs #47. Explicit injection is deferred to #59."
```

---

## Part 2: Replay transport (spec step 2). Branch `feature/replay-transport` (from `develop` after Part 1 merges).

### Task 7: Fixture key and secret removal (pure functions)

**Files:**
- Create: `crates/scope-core/src/http/replay/key.rs`
- Create: `crates/scope-core/src/http/replay/mod.rs` (`mod key; pub use key::*;` for now)
- Modify: `crates/scope-core/src/http/mod.rs` (`pub mod replay;`)

**Interfaces:**
- Produces:
  - `pub const SECRET_PARAMS: &[&str] = &["apikey", "api_key", "key", "token", "access_token"];`
  - `pub const SECRET_HEADERS: &[&str] = &["authorization", "x-api-key", "cookie", "set-cookie"];`
  - `pub fn clean_url(url: &str) -> String`: removes secret query parameters (case-insensitive), sorts the rest by name then value.
  - `pub fn body_sha256(body: Option<&str>) -> String`: hex SHA-256 of the body (empty string when `None`).
  - `pub fn fixture_key(method: &str, url: &str, body: Option<&str>) -> String`: `"{METHOD} {clean_url} {body_sha256}"`.
  - `pub fn fixture_path(dir: &Path, method: &str, url: &str, body: Option<&str>) -> PathBuf`: `dir/<host>/<sha256(fixture_key)>.json`. Host `unknown` when the URL has none.

- [ ] **Step 1: Write the failing tests** in `key.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_url_removes_secret_params_in_any_case() {
        let u = clean_url("https://api.etherscan.io/v2/api?module=account&ApiKey=SECRET123&address=0x1");
        assert!(!u.contains("SECRET123"), "{u}");
        assert!(!u.to_lowercase().contains("apikey"), "{u}");
        assert_eq!(u, "https://api.etherscan.io/v2/api?address=0x1&module=account");
    }

    #[test]
    fn test_param_order_does_not_change_the_key() {
        let a = fixture_key("GET", "https://x.io/p?b=2&a=1", None);
        let b = fixture_key("GET", "https://x.io/p?a=1&b=2", None);
        assert_eq!(a, b);
    }

    #[test]
    fn test_body_changes_the_key() {
        let a = fixture_key("POST", "https://rpc.io/", Some(r#"{"id":1}"#));
        let b = fixture_key("POST", "https://rpc.io/", Some(r#"{"id":2}"#));
        assert_ne!(a, b);
    }

    #[test]
    fn test_fixture_path_groups_by_host() {
        let p = fixture_path(std::path::Path::new("/f"), "GET", "https://api.binance.com/api/v3/depth?symbol=X", None);
        assert!(p.starts_with("/f/api.binance.com"), "{}", p.display());
        assert_eq!(p.extension().unwrap(), "json");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib http::replay`
Expected: compile error.

- [ ] **Step 3: Implement `key.rs`**

```rust
//! Fixture keys: a request's identity with secrets removed.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const SECRET_PARAMS: &[&str] = &["apikey", "api_key", "key", "token", "access_token"];
pub const SECRET_HEADERS: &[&str] = &["authorization", "x-api-key", "cookie", "set-cookie"];

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The URL with secret query parameters removed and the rest sorted.
pub fn clean_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    let mut pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !SECRET_PARAMS.contains(&k.to_lowercase().as_str()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.sort();
    if pairs.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    parsed.to_string()
}

/// Hex SHA-256 of the request body.
pub fn body_sha256(body: Option<&str>) -> String {
    hex_sha256(body.unwrap_or("").as_bytes())
}

/// The identity of a request.
pub fn fixture_key(method: &str, url: &str, body: Option<&str>) -> String {
    format!("{} {} {}", method.to_uppercase(), clean_url(url), body_sha256(body))
}

/// Where the fixture for a request lives.
pub fn fixture_path(dir: &Path, method: &str, url: &str, body: Option<&str>) -> PathBuf {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    dir.join(host).join(format!("{}.json", hex_sha256(fixture_key(method, url, body).as_bytes())))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-core --lib http::replay`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-core/src/http
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(http): fixture keys with secrets removed"
```

### Task 8: `ReplayHttpClient` (replay and record) with the secret scan

**Files:**
- Create: `crates/scope-core/src/http/replay/client.rs`
- Modify: `crates/scope-core/src/http/replay/mod.rs` (`mod client; pub use client::{Mode, ReplayHttpClient, Fixture, RecordedResponse};`)

**Interfaces:**
- Consumes: Task 7 functions, `HttpClient`, `Request`, `Response`.
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Mode { Replay, Record }`
  - `#[derive(Serialize, Deserialize)] pub struct Fixture { pub recorded: String, pub request: FixtureRequest, pub responses: Vec<RecordedResponse> }`, `FixtureRequest { method, url, body_sha256 }`, `RecordedResponse { status: u16, headers: BTreeMap<String,String>, body: String }`
  - `ReplayHttpClient::replay(dir: PathBuf) -> Self`
  - `ReplayHttpClient::record(dir: PathBuf, inner: Arc<dyn HttpClient>, secrets: Vec<String>) -> Self`

- [ ] **Step 1: Write the failing tests** in `client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Answers with the next body in order.
    struct Script(Mutex<Vec<&'static str>>);

    #[async_trait::async_trait]
    impl HttpClient for Script {
        async fn send(&self, _r: Request) -> Result<Response, ScopeError> {
            let body = self.0.lock().unwrap().remove(0);
            Ok(Response { status_code: 200, headers: [("content-type".into(), "application/json".into()), ("set-cookie".into(), "s=1".into())].into(), body: body.into() })
        }
    }

    #[tokio::test]
    async fn test_record_then_replay_returns_responses_in_order_then_repeats_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(Script(Mutex::new(vec!["one", "two"])));
        let rec = ReplayHttpClient::record(dir.path().into(), inner, vec![]);
        let url = "https://api.example.com/x?apikey=SECRET&a=1";
        assert_eq!(rec.send(Request::get(url)).await.unwrap().body, "one");
        assert_eq!(rec.send(Request::get(url)).await.unwrap().body, "two");

        let rep = ReplayHttpClient::replay(dir.path().into());
        let r1 = rep.send(Request::get(url)).await.unwrap();
        assert_eq!(r1.body, "one");
        assert!(!r1.headers.contains_key("set-cookie"));
        assert_eq!(rep.send(Request::get(url)).await.unwrap().body, "two");
        // Repeat mode outlasts the recording: keep serving the last response.
        assert_eq!(rep.send(Request::get(url)).await.unwrap().body, "two");
    }

    #[tokio::test]
    async fn test_replay_miss_names_the_request_and_never_uses_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let rep = ReplayHttpClient::replay(dir.path().into());
        let err = rep.send(Request::get("https://api.example.com/missing")).await.unwrap_err().to_string();
        assert!(err.contains("no fixture for GET https://api.example.com/missing"), "{err}");
    }

    #[tokio::test]
    async fn test_record_refuses_a_fixture_that_contains_a_configured_secret() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(Script(Mutex::new(vec![r#"{"echo":"MYKEY-0123456789"}"#])));
        let rec = ReplayHttpClient::record(dir.path().into(), inner, vec!["MYKEY-0123456789".into()]);
        let err = rec.send(Request::get("https://api.example.com/echo")).await.unwrap_err().to_string();
        assert!(err.contains("configured secret"), "{err}");
        assert!(walk(dir.path()).is_empty(), "no fixture file may be written");
    }

    #[tokio::test]
    async fn test_stored_url_has_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(Script(Mutex::new(vec!["{}"])));
        let rec = ReplayHttpClient::record(dir.path().into(), inner, vec![]);
        rec.send(Request::get("https://api.example.com/x?Token=abc123")).await.unwrap();
        let text: String = walk(dir.path()).iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
        assert!(!text.contains("abc123"), "{text}");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() { out.extend(walk(&p)); } else { out.push(p); }
        }
        out
    }
}
```

Add `tempfile = "3.10"` to scope-core `[dev-dependencies]` if it is missing.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib http::replay`
Expected: compile error.

- [ ] **Step 3: Implement `client.rs`**

```rust
//! Record/replay transport for the TUI test harness (design section 3).

use super::key::{body_sha256, clean_url, fixture_path, SECRET_HEADERS};
use crate::error::ScopeError;
use crate::http::{HttpClient, Request, Response};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { Replay, Record }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureRequest { pub method: String, pub url: String, pub body_sha256: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedResponse { pub status: u16, pub headers: BTreeMap<String, String>, pub body: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture { pub recorded: String, pub request: FixtureRequest, pub responses: Vec<RecordedResponse> }

pub struct ReplayHttpClient {
    mode: Mode,
    dir: PathBuf,
    inner: Option<Arc<dyn HttpClient>>,
    secrets: Vec<String>,
    /// Replay: how many responses of each fixture were served.
    served: Mutex<HashMap<PathBuf, usize>>,
    /// Record: fixtures written in this run (later calls append).
    written: Mutex<HashSet<PathBuf>>,
}

impl ReplayHttpClient {
    pub fn replay(dir: PathBuf) -> Self {
        Self { mode: Mode::Replay, dir, inner: None, secrets: vec![], served: Default::default(), written: Default::default() }
    }

    pub fn record(dir: PathBuf, inner: Arc<dyn HttpClient>, secrets: Vec<String>) -> Self {
        // Short values match too much text by chance; ignore them.
        let secrets = secrets.into_iter().filter(|s| s.len() >= 6).collect();
        Self { mode: Mode::Record, dir, inner: Some(inner), secrets, served: Default::default(), written: Default::default() }
    }

    fn miss(req: &Request) -> ScopeError {
        ScopeError::Network(format!(
            "no fixture for {} {}; run `just tui-record <scenario>`",
            req.method, clean_url(&req.url)
        ))
    }

    fn read(path: &std::path::Path) -> Option<Fixture> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }
}

#[async_trait]
impl HttpClient for ReplayHttpClient {
    async fn send(&self, req: Request) -> Result<Response, ScopeError> {
        let path = fixture_path(&self.dir, &req.method, &req.url, req.body.as_deref());
        match self.mode {
            Mode::Replay => {
                let fixture = Self::read(&path).ok_or_else(|| Self::miss(&req))?;
                let mut served = self.served.lock().expect("served lock");
                let n = served.entry(path).or_insert(0);
                let idx = (*n).min(fixture.responses.len().saturating_sub(1));
                *n += 1;
                let r = fixture.responses.get(idx).ok_or_else(|| Self::miss(&req))?;
                Ok(Response { status_code: r.status, headers: r.headers.clone().into_iter().collect(), body: r.body.clone() })
            }
            Mode::Record => {
                let inner = self.inner.as_ref().expect("record mode has an inner transport");
                let resp = inner.send(req.clone()).await?;
                let recorded = RecordedResponse {
                    status: resp.status_code,
                    headers: resp.headers.iter()
                        .filter(|(k, _)| !SECRET_HEADERS.contains(&k.to_lowercase().as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    body: resp.body.clone(),
                };
                let mut written = self.written.lock().expect("written lock");
                let mut fixture = if written.contains(&path) { Self::read(&path) } else { None }
                    .unwrap_or_else(|| Fixture {
                        recorded: chrono::Utc::now().to_rfc3339(),
                        request: FixtureRequest { method: req.method.to_uppercase(), url: clean_url(&req.url), body_sha256: body_sha256(req.body.as_deref()) },
                        responses: vec![],
                    });
                fixture.responses.push(recorded);
                let text = serde_json::to_string_pretty(&fixture)
                    .map_err(|e| ScopeError::Network(format!("cannot serialize fixture: {e}")))?;
                if self.secrets.iter().any(|s| text.contains(s.as_str())) {
                    return Err(ScopeError::Network(format!(
                        "fixture for {} {} would contain a configured secret; not written",
                        req.method, clean_url(&req.url)
                    )));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| ScopeError::Network(format!("cannot create {}: {e}", parent.display())))?;
                }
                std::fs::write(&path, text).map_err(|e| ScopeError::Network(format!("cannot write {}: {e}", path.display())))?;
                written.insert(path);
                Ok(resp)
            }
        }
    }
}
```

`Request` must be `Clone` (it derives `Clone` already).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-core --lib http::replay`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-core/src/http crates/scope-core/Cargo.toml
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(http): record/replay transport with a secret scan"
```

### Task 9: Select replay or record from the environment

**Files:**
- Modify: `crates/scope-core/src/http/shared.rs` (`build_with`)

**Interfaces:**
- Consumes: `ReplayHttpClient::{replay, record}`, `Config.chains.api_keys`, `Config.web.auth_token`, `Config.chains.{ethereum_rpc, bsc_rpc, aegis_rpc, solana_rpc, tron_api}`.
- Produces: `pub fn secret_values(config: &Config) -> Vec<String>` (api key values, the web token, and each RPC URL's password and path segments of 16 or more characters).

- [ ] **Step 1: Write the failing tests** in `shared.rs` tests:

```rust
fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
}

#[test]
fn test_replay_mode_needs_a_fixture_dir() {
    let err = build_with(&Config::default(), &env_of(&[("SCOPE_HTTP_MODE", "replay")])).err().unwrap();
    assert!(err.to_string().contains("SCOPE_HTTP_FIXTURES"), "{err}");
}

#[test]
fn test_replay_mode_selects_replay() {
    let built = build_with(&Config::default(), &env_of(&[("SCOPE_HTTP_MODE", "replay"), ("SCOPE_HTTP_FIXTURES", "/tmp/x")])).unwrap();
    assert_eq!(built.kind, Kind::Replay);
}

#[test]
fn test_record_mode_selects_record() {
    let built = build_with(&Config::default(), &env_of(&[("SCOPE_HTTP_MODE", "record"), ("SCOPE_HTTP_FIXTURES", "/tmp/x")])).unwrap();
    assert_eq!(built.kind, Kind::Record);
}

#[test]
fn test_unknown_mode_is_an_error() {
    let err = build_with(&Config::default(), &env_of(&[("SCOPE_HTTP_MODE", "playback"), ("SCOPE_HTTP_FIXTURES", "/tmp/x")])).err().unwrap();
    assert!(err.to_string().contains("playback"), "{err}");
}

#[test]
fn test_secret_values_include_keys_token_and_rpc_path() {
    let mut c = Config::default();
    c.chains.api_keys.insert("etherscan".into(), "ABCDEF123456".into());
    c.chains.ethereum_rpc = Some("https://mainnet.infura.io/v3/0123456789abcdef0123".into());
    let s = secret_values(&c);
    assert!(s.contains(&"ABCDEF123456".to_string()));
    assert!(s.contains(&"0123456789abcdef0123".to_string()));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-bca-core --lib http::shared`
Expected: FAIL (replay env ignored; `secret_values` missing).

- [ ] **Step 3: Implement**

At the top of `build_with`, replace `let _ = env;` with:

```rust
if let Some(mode) = env("SCOPE_HTTP_MODE") {
    let dir = env("SCOPE_HTTP_FIXTURES").map(std::path::PathBuf::from).ok_or_else(|| {
        ScopeError::Network("SCOPE_HTTP_MODE is set but SCOPE_HTTP_FIXTURES is not".into())
    })?;
    return match mode.as_str() {
        "replay" => Ok(Built { client: Arc::new(super::replay::ReplayHttpClient::replay(dir)), kind: Kind::Replay, notices: vec![] }),
        "record" => {
            // The normal transport (Ghola or native) does the real requests.
            let inner = build_with(config, &|_| None)?;
            Ok(Built {
                client: Arc::new(super::replay::ReplayHttpClient::record(dir, inner.client, secret_values(config))),
                kind: Kind::Record,
                notices: inner.notices,
            })
        }
        other => Err(ScopeError::Network(format!("unknown SCOPE_HTTP_MODE \"{other}\" (use replay or record)"))),
    };
}
```

(`build_with(config, &|_| None)` builds the normal transport for record mode; the empty env lookup stops it from selecting record again.)

Add:

```rust
/// Values that must never be written to a fixture.
pub fn secret_values(config: &Config) -> Vec<String> {
    let mut out: Vec<String> = config.chains.api_keys.values().cloned().collect();
    if let Some(t) = &config.web.auth_token {
        out.push(t.clone());
    }
    let rpcs = [&config.chains.ethereum_rpc, &config.chains.bsc_rpc, &config.chains.aegis_rpc, &config.chains.solana_rpc, &config.chains.tron_api];
    for url in rpcs.into_iter().flatten() {
        if let Ok(u) = reqwest::Url::parse(url) {
            if let Some(p) = u.password() {
                out.push(p.to_string());
            }
            out.extend(u.path_segments().into_iter().flatten().filter(|s| s.len() >= 16).map(str::to_string));
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-core --lib http::`
Expected: PASS.

- [ ] **Step 5: Manual end-to-end check** (the real binary, record then replay)

```bash
cargo build --bin scope
F=$(mktemp -d)
SCOPE_HTTP_MODE=record SCOPE_HTTP_FIXTURES=$F target/debug/scope market summary USDC --venue binance --format json > /tmp/rec.json
SCOPE_HTTP_MODE=replay SCOPE_HTTP_FIXTURES=$F target/debug/scope market summary USDC --venue binance --format json > /tmp/rep.json
diff <(jq 'del(.timestamp)' /tmp/rec.json) <(jq 'del(.timestamp)' /tmp/rep.json) && echo SAME
SCOPE_HTTP_MODE=replay SCOPE_HTTP_FIXTURES=$F target/debug/scope market summary USDC --venue okx; echo "exit $?"
```

Expected: `SAME`; the last command fails with `no fixture for GET https://www.okx.com/…` and exit 1.

- [ ] **Step 6: Gates, commit, push, PR**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/scope-core/src/http
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(http): select replay or record with SCOPE_HTTP_MODE"
GH_TOKEN=$(gh auth token --user robot-accomplice) git push -u origin feature/replay-transport
GH_TOKEN=$(gh auth token --user robot-accomplice) gh pr create --repo robot-accomplice/scope-blockchain-analysis --base develop --title "Record/replay HTTP transport" --body "Implements the TUI test harness design, section 3. Refs #47."
```

---

## Part 3: Harness crate (spec step 3). Branch `feature/tui-harness` (can start beside Parts 1 and 2; the offline tier needs Part 2 merged).

### Task 10: Crate and scenario model

**Files:**
- Modify: `Cargo.toml` (workspace `members` += `"crates/scope-tui-harness"`)
- Create: `crates/scope-tui-harness/Cargo.toml`
- Create: `crates/scope-tui-harness/src/lib.rs`, `src/scenario.rs`

**Interfaces:**
- Produces (`scenario.rs`):
  - `pub enum Tier { Offline, Live }` (serde `lowercase`)
  - `pub struct Terminal { pub cols: u16, pub rows: u16 }` (default 120×36)
  - `pub enum Step` (serde `untagged` is not used; each step is a one-key map): `Type(String)`, `Key(String)`, `WaitFor { text: String, timeout: Option<Duration> }`, `ExpectScreen { contains: Vec<String>, not_contains: Vec<String> }`, `ExpectJournal { argv_prefix: Vec<String>, outcome: Option<String>, error: Option<String> }`, `ExpectFile { path: String, contains: Vec<String> }`, `Resize { cols: u16, rows: u16 }`, `Snapshot(String)`, `ExpectExit(i32)`
  - `pub struct Scenario { pub name: String, pub tiers: Vec<Tier>, pub terminal: Terminal, pub timeout: Duration, pub config: Option<String>, pub masks: Vec<String>, pub steps: Vec<Step> }`
  - `pub fn load(path: &Path) -> anyhow::Result<Scenario>` (parses and validates)
  - `pub fn key_bytes(name: &str) -> anyhow::Result<Vec<u8>>`
  - Durations in YAML are strings like `15s`, `500ms`, `2m`.

- [ ] **Step 1: Create the crate**

`crates/scope-tui-harness/Cargo.toml`:

```toml
[package]
name = "scope-tui-harness"
description = "Runs YAML scenarios against the real scope TUI in a PTY"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[[bin]]
name = "scope-tui-harness"
path = "src/main.rs"

[dependencies]
scope_cli = { path = "../scope-cli", package = "scope-bca-cli" }
anyhow = "1"
clap = { version = "4.0", features = ["derive"] }
portable-pty = "0.9"
regex = "1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_yaml = "0.9"
tempfile = "3.10"
vt100 = "0.16"
```

`src/lib.rs`: `pub mod scenario;` (later tasks add modules). `src/main.rs`: `fn main() {}` for now.

- [ ] **Step 2: Write the failing tests** in `scenario.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
name: palette opens
terminal: { cols: 100, rows: 30 }
steps:
  - key: Ctrl-K
  - wait_for: { text: "Commands", timeout: 5s }
  - expect_screen: { contains: ["venues list"] }
  - key: Esc
  - expect_exit: 0
"#;

    #[test]
    fn test_parse_good_scenario() {
        let s = parse(GOOD).unwrap();
        assert_eq!(s.terminal.cols, 100);
        assert_eq!(s.tiers, vec![Tier::Offline, Tier::Live]);
        assert_eq!(s.timeout, Duration::from_secs(120));
        assert!(matches!(&s.steps[1], Step::WaitFor { timeout: Some(t), .. } if *t == Duration::from_secs(5)));
    }

    #[test]
    fn test_scenario_without_an_expectation_is_rejected() {
        let err = parse("name: x\nsteps:\n  - type: \"help\"\n  - key: Enter\n").unwrap_err().to_string();
        assert!(err.contains("no expect_"), "{err}");
    }

    #[test]
    fn test_unknown_key_name_is_rejected() {
        let err = parse("name: x\nsteps:\n  - key: Hyper-Q\n  - expect_exit: 0\n").unwrap_err().to_string();
        assert!(err.contains("Hyper-Q"), "{err}");
    }

    #[test]
    fn test_unknown_step_field_is_rejected() {
        assert!(parse("name: x\nsteps:\n  - typo: 1\n  - expect_exit: 0\n").is_err());
    }

    #[test]
    fn test_key_bytes() {
        assert_eq!(key_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_bytes("Ctrl-K").unwrap(), vec![0x0b]);
        assert_eq!(key_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("Alt-x").unwrap(), b"\x1bx");
        assert_eq!(key_bytes("F5").unwrap(), b"\x1b[15~");
    }
}
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p scope-tui-harness`
Expected: compile errors.

- [ ] **Step 4: Implement `scenario.rs`**

```rust
//! Scenario files (TUI test harness design, section 5).

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier { Offline, Live }

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Terminal { pub cols: u16, pub rows: u16 }

impl Default for Terminal {
    fn default() -> Self { Self { cols: 120, rows: 36 } }
}

fn parse_duration(s: &str) -> Result<Duration> {
    let (n, unit) = s.trim().split_at(s.trim().find(|c: char| !c.is_ascii_digit()).ok_or_else(|| anyhow!("duration {s:?} has no unit"))?);
    let n: u64 = n.parse().with_context(|| format!("duration {s:?}"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        _ => bail!("duration {s:?}: unit must be ms, s or m"),
    })
}

fn de_duration<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Duration, D::Error> {
    let s = String::deserialize(d)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}

fn de_opt_duration<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Duration>, D::Error> {
    Option::<String>::deserialize(d)?.map(|s| parse_duration(&s)).transpose().map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Type(String),
    Key(String),
    WaitFor { text: String, #[serde(default, deserialize_with = "de_opt_duration")] timeout: Option<Duration> },
    ExpectScreen { #[serde(default)] contains: Vec<String>, #[serde(default)] not_contains: Vec<String> },
    ExpectJournal { argv_prefix: Vec<String>, outcome: Option<String>, error: Option<String> },
    ExpectFile { path: String, #[serde(default)] contains: Vec<String> },
    Resize { cols: u16, rows: u16 },
    Snapshot(String),
    ExpectExit(i32),
}

impl Step {
    fn is_expectation(&self) -> bool {
        matches!(self, Step::ExpectScreen { .. } | Step::ExpectJournal { .. } | Step::ExpectFile { .. } | Step::Snapshot(_) | Step::ExpectExit(_))
    }
}

fn default_tiers() -> Vec<Tier> { vec![Tier::Offline, Tier::Live] }
fn default_timeout() -> Duration { Duration::from_secs(120) }

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<Tier>,
    #[serde(default)]
    pub terminal: Terminal,
    #[serde(default = "default_timeout", deserialize_with = "de_duration")]
    pub timeout: Duration,
    pub config: Option<String>,
    #[serde(default)]
    pub masks: Vec<String>,
    pub steps: Vec<Step>,
}

/// The bytes a terminal sends for a named key.
pub fn key_bytes(name: &str) -> Result<Vec<u8>> {
    let fixed: Option<&[u8]> = match name {
        "Enter" => Some(b"\r"), "Tab" => Some(b"\t"), "Esc" => Some(b"\x1b"), "Backspace" => Some(b"\x7f"),
        "Up" => Some(b"\x1b[A"), "Down" => Some(b"\x1b[B"), "Right" => Some(b"\x1b[C"), "Left" => Some(b"\x1b[D"),
        "PageUp" => Some(b"\x1b[5~"), "PageDown" => Some(b"\x1b[6~"),
        "F1" => Some(b"\x1bOP"), "F2" => Some(b"\x1bOQ"), "F3" => Some(b"\x1bOR"), "F4" => Some(b"\x1bOS"),
        "F5" => Some(b"\x1b[15~"), "F6" => Some(b"\x1b[17~"), "F7" => Some(b"\x1b[18~"), "F8" => Some(b"\x1b[19~"),
        "F9" => Some(b"\x1b[20~"), "F10" => Some(b"\x1b[21~"), "F11" => Some(b"\x1b[23~"), "F12" => Some(b"\x1b[24~"),
        _ => None,
    };
    if let Some(b) = fixed {
        return Ok(b.to_vec());
    }
    let one_letter = |rest: &str| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic());
    if let Some(rest) = name.strip_prefix("Ctrl-").filter(|r| one_letter(r)) {
        return Ok(vec![rest.as_bytes()[0].to_ascii_uppercase() & 0x1f]);
    }
    if let Some(rest) = name.strip_prefix("Alt-").filter(|r| one_letter(r)) {
        return Ok(vec![0x1b, rest.as_bytes()[0]]);
    }
    bail!("unknown key name {name:?}")
}

/// Parses and validates a scenario.
pub fn parse(yaml: &str) -> Result<Scenario> {
    let s: Scenario = serde_yaml::from_str(yaml)?;
    if !s.steps.iter().any(Step::is_expectation) {
        bail!("scenario {:?} has no expect_* or snapshot step; it cannot fail", s.name);
    }
    for step in &s.steps {
        if let Step::Key(k) = step {
            key_bytes(k)?;
        }
    }
    Ok(s)
}

/// Loads a scenario file.
pub fn load(path: &Path) -> Result<Scenario> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text).with_context(|| format!("in {}", path.display()))
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p scope-tui-harness`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/scope-tui-harness
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(harness): crate and scenario model"
```

### Task 11: Screen text, masks and snapshots

**Files:**
- Create: `crates/scope-tui-harness/src/screen.rs`
- Modify: `src/lib.rs` (`pub mod screen;`)

**Interfaces:**
- Produces:
  - `pub fn text(parser: &vt100::Parser) -> String`: rows joined by `\n`, trailing spaces removed.
  - `pub fn mask(text: &str, extra: &[String]) -> anyhow::Result<String>`
  - `pub enum SnapshotResult { Match, Written(PathBuf), Differs { expected: String, actual: String } }`
  - `pub fn compare_snapshot(dir: &Path, name: &str, masked: &str) -> anyhow::Result<SnapshotResult>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_reads_the_screen_and_trims() {
        let mut p = vt100::Parser::new(3, 10, 0);
        p.process(b"hi   \r\nthere");
        assert_eq!(text(&p), "hi\nthere\n");
    }

    #[test]
    fn test_mask_hides_times_dates_and_durations() {
        let m = mask("at 2026-10-12T09:30:00Z, 14:05:09, took 123ms and 1.2s", &[]).unwrap();
        assert_eq!(m, "at <TIME>, <TIME>, took <DUR> and <DUR>");
    }

    #[test]
    fn test_extra_mask() {
        assert_eq!(mask("age 3 days", &["age \\d+ days".into()]).unwrap(), "<MASK>");
    }

    #[test]
    fn test_missing_snapshot_is_written_and_not_a_match() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(compare_snapshot(d.path(), "a", "x\n").unwrap(), SnapshotResult::Written(_)));
        assert!(matches!(compare_snapshot(d.path(), "a", "x\n").unwrap(), SnapshotResult::Match));
        assert!(matches!(compare_snapshot(d.path(), "a", "y\n").unwrap(), SnapshotResult::Differs { .. }));
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p scope-tui-harness screen`
Expected: compile error.

- [ ] **Step 3: Implement `screen.rs`**

```rust
//! Screen text, masks and snapshots (design 5.2, 5.4).

use anyhow::Result;
use regex::Regex;
use std::path::{Path, PathBuf};

/// The screen as text: one line per row, trailing spaces removed.
pub fn text(parser: &vt100::Parser) -> String {
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let mut out = String::new();
    for line in screen.rows(0, cols).take(rows as usize) {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

const BUILT_IN: &[(&str, &str)] = &[
    (r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?", "<TIME>"),
    (r"\b\d{2}:\d{2}:\d{2}\b", "<TIME>"),
    (r"\b\d{4}-\d{2}-\d{2}\b", "<DATE>"),
    (r"\b\d+(\.\d+)?(ms|s)\b", "<DUR>"),
];

/// Replaces text that changes from run to run.
pub fn mask(text: &str, extra: &[String]) -> Result<String> {
    let mut out = text.to_string();
    for (pat, rep) in BUILT_IN {
        out = Regex::new(pat)?.replace_all(&out, *rep).into_owned();
    }
    for pat in extra {
        out = Regex::new(pat)?.replace_all(&out, "<MASK>").into_owned();
    }
    Ok(out)
}

#[derive(Debug)]
pub enum SnapshotResult { Match, Written(PathBuf), Differs { expected: String, actual: String } }

/// Compares `masked` with `<dir>/<name>.txt`, writing it when missing.
pub fn compare_snapshot(dir: &Path, name: &str, masked: &str) -> Result<SnapshotResult> {
    let path = dir.join(format!("{name}.txt"));
    match std::fs::read_to_string(&path) {
        Ok(expected) if expected == masked => Ok(SnapshotResult::Match),
        Ok(expected) => Ok(SnapshotResult::Differs { expected, actual: masked.to_string() }),
        Err(_) => {
            std::fs::create_dir_all(dir)?;
            std::fs::write(&path, masked)?;
            Ok(SnapshotResult::Written(path))
        }
    }
}
```

(`test_text_reads_the_screen_and_trims` expects one trailing `\n` after the last non-empty row; adjust the trim loop if `vt100` reports rows differently, keeping the test's expectation.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-tui-harness screen`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/scope-tui-harness
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(harness): screen text, masks and snapshots"
```

### Task 12: PTY session and step runner

**Files:**
- Create: `crates/scope-tui-harness/src/session.rs`, `src/run.rs`
- Modify: `src/lib.rs` (`pub mod session; pub mod run;`)

**Interfaces:**
- Consumes: `scenario::{Scenario, Step, Tier, key_bytes}`, `screen::{text, mask, compare_snapshot, SnapshotResult}`.
- Produces:
  - `session::Session::start(opts: &StartOpts) -> Result<Session>`; `StartOpts { binary: PathBuf, home: PathBuf, cols: u16, rows: u16, env: Vec<(String, String)> }`
  - `Session::send(&mut self, bytes: &[u8]) -> Result<()>`, `Session::resize(&mut self, cols, rows) -> Result<()>`, `Session::pump(&mut self, wait: Duration) -> Result<()>` (reads available output into the parser), `Session::screen(&self) -> String`, `Session::exit_status(&mut self) -> Option<i32>`, `Session::raw(&self) -> &[u8]`, `Session::kill(&mut self)`
  - `pub fn data_dir(home: &Path) -> PathBuf`: macOS `home/Library/Application Support/scope`, other unix `home/.local/share/scope`.
  - `run::Outcome { pub passed: bool, pub failure: Option<String>, pub steps: Vec<StepLog>, pub argv_seen: Vec<Vec<String>> }`, `StepLog { index, step: String, ms: u128, ok: bool }`
  - `run::run_scenario(s: &Scenario, tier: Tier, ctx: &RunCtx) -> Result<(Outcome, Session, tempfile::TempDir)>`; `RunCtx { binary: PathBuf, fixtures_root: PathBuf, snapshots_root: PathBuf, scenario_id: String }`

- [ ] **Step 1: Write the failing tests** in `run.rs` (these run the real binary; they build it first through `env!("CARGO_BIN_EXE_scope")` is not available across crates, so they read `SCOPE_BIN` or default to `<workspace>/target/debug/scope` and skip with a clear message if the file is missing):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::parse;

    fn ctx() -> Option<RunCtx> {
        let bin = std::env::var("SCOPE_BIN").map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/scope"));
        if !bin.exists() {
            eprintln!("skip: build scope first (cargo build --bin scope)");
            return None;
        }
        let d = tempfile::tempdir().unwrap().keep();
        Some(RunCtx { binary: bin, fixtures_root: d.join("fx"), snapshots_root: d.join("snap"), scenario_id: "t".into() })
    }

    #[test]
    fn test_session_command_runs_and_is_journaled() {
        let Some(c) = ctx() else { return };
        let s = parse(r#"
name: ctx shows the session
steps:
  - type: "ctx"
  - key: Enter
  - wait_for: { text: "(?i)chain", timeout: 10s }
  - expect_journal: { argv_prefix: [], outcome: session }
  - key: Ctrl-D
  - expect_exit: 0
"#).unwrap();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(out.passed, "{:?}", out.failure);
    }

    #[test]
    fn test_wait_for_times_out_with_the_screen_in_the_failure() {
        let Some(c) = ctx() else { return };
        let s = parse("name: never\nsteps:\n  - wait_for: { text: \"NEVER-APPEARS\", timeout: 1s }\n  - expect_exit: 0\n").unwrap();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        let f = out.failure.unwrap();
        assert!(f.contains("NEVER-APPEARS") && f.contains("screen:"), "{f}");
    }

    #[test]
    fn test_early_exit_fails_the_next_step_without_hanging() {
        let Some(c) = ctx() else { return };
        let s = parse("name: early\ntimeout: 20s\nsteps:\n  - key: Ctrl-D\n  - wait_for: { text: \"x\", timeout: 10s }\n  - expect_exit: 0\n").unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        assert!(out.failure.unwrap().contains("process exited"));
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
    }
}
```

`expect_journal` with an empty `argv_prefix` matches any record whose `outcome` matches; session commands record an empty `argv`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo build --bin scope && cargo test -p scope-tui-harness run`
Expected: compile error.

- [ ] **Step 3: Implement `session.rs`**

```rust
//! A scope TUI process in a PTY, with a vt100 screen model.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub struct StartOpts { pub binary: PathBuf, pub home: PathBuf, pub cols: u16, pub rows: u16, pub env: Vec<(String, String)> }

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    raw: Vec<u8>,
    status: Option<i32>,
}

/// Where scope keeps its journal under a given HOME.
pub fn data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/scope")
    } else {
        home.join(".local/share/scope")
    }
}

impl Session {
    pub fn start(o: &StartOpts) -> Result<Self> {
        let pair = native_pty_system().openpty(PtySize { rows: o.rows, cols: o.cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(&o.binary);
        cmd.args(["tui", "--no-banner"]);
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", &o.home);
        cmd.env("TZ", "UTC");
        cmd.env("LANG", "C.UTF-8");
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &o.env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).context("spawning scope")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self { master: pair.master, writer, child, rx, parser: vt100::Parser::new(o.rows, o.cols, 0), raw: Vec::new(), status: None })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        self.parser.set_size(rows, cols);
        Ok(())
    }

    /// Reads output for up to `wait`, feeding the screen model.
    pub fn pump(&mut self, wait: Duration) -> Result<()> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(chunk) => {
                    self.parser.process(&chunk);
                    self.raw.extend_from_slice(&chunk);
                }
                Err(_) if Instant::now() >= until => return Ok(()),
                Err(_) => {}
            }
        }
    }

    pub fn screen(&self) -> String { crate::screen::text(&self.parser) }
    pub fn raw(&self) -> &[u8] { &self.raw }

    /// The exit code once the process has ended.
    pub fn exit_status(&mut self) -> Option<i32> {
        if self.status.is_none() {
            if let Ok(Some(s)) = self.child.try_wait() {
                self.status = Some(s.exit_code() as i32);
            }
        }
        self.status
    }

    pub fn kill(&mut self) { let _ = self.child.kill(); }
}

impl Drop for Session {
    fn drop(&mut self) { self.kill(); }
}
```

- [ ] **Step 4: Implement `run.rs`**

```rust
//! Runs one scenario (design 4.2, 5.2).

use crate::scenario::{key_bytes, Scenario, Step, Tier};
use crate::screen::{compare_snapshot, mask, SnapshotResult};
use crate::session::{data_dir, Session, StartOpts};
use anyhow::Result;
use regex::Regex;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct RunCtx { pub binary: PathBuf, pub fixtures_root: PathBuf, pub snapshots_root: PathBuf, pub scenario_id: String }

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepLog { pub index: usize, pub step: String, pub ms: u128, pub ok: bool }

#[derive(Debug, Default)]
pub struct Outcome { pub passed: bool, pub failure: Option<String>, pub steps: Vec<StepLog>, pub argv_seen: Vec<Vec<String>> }

const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(150);

fn journal(home: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(data_dir(home).join("tui-events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn argv_of(rec: &serde_json::Value) -> Vec<String> {
    rec["argv"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn run_scenario(s: &Scenario, tier: Tier, ctx: &RunCtx) -> Result<(Outcome, Session, tempfile::TempDir)> {
    let home = tempfile::tempdir()?;
    let cfg_dir = home.path().join(".config/scope");
    std::fs::create_dir_all(&cfg_dir)?;
    std::fs::write(cfg_dir.join("config.yaml"), s.config.clone().unwrap_or_else(|| "{}".into()))?;
    let mut env = vec![];
    if tier == Tier::Offline {
        env.push(("SCOPE_HTTP_MODE".to_string(), "replay".to_string()));
        env.push(("SCOPE_HTTP_FIXTURES".to_string(), ctx.fixtures_root.join(&ctx.scenario_id).display().to_string()));
    }
    let mut sess = Session::start(&StartOpts { binary: ctx.binary.clone(), home: home.path().into(), cols: s.terminal.cols, rows: s.terminal.rows, env })?;
    let deadline = Instant::now() + s.timeout;
    let mut out = Outcome::default();
    let mut journal_seen = 0usize;

    for (i, step) in s.steps.iter().enumerate() {
        let started = Instant::now();
        let result = run_step(step, s, ctx, &mut sess, home.path(), &mut journal_seen, deadline);
        out.steps.push(StepLog { index: i, step: format!("{step:?}"), ms: started.elapsed().as_millis(), ok: result.is_ok() });
        if let Err(e) = result {
            out.failure = Some(format!("step {} {:?}: {e}\nscreen:\n{}", i + 1, step, sess.screen()));
            sess.kill();
            out.argv_seen = journal(home.path()).iter().map(argv_of).collect();
            return Ok((out, sess, home));
        }
    }
    out.passed = true;
    out.argv_seen = journal(home.path()).iter().filter(|r| r["outcome"] == "ok" || r["outcome"] == "error").map(argv_of).collect();
    Ok((out, sess, home))
}

fn exited(sess: &mut Session) -> Result<()> {
    match sess.exit_status() {
        Some(code) => anyhow::bail!("process exited with status {code}"),
        None => Ok(()),
    }
}

fn run_step(step: &Step, s: &Scenario, ctx: &RunCtx, sess: &mut Session, home: &std::path::Path, journal_seen: &mut usize, deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        anyhow::bail!("scenario timeout {:?} reached", s.timeout);
    }
    match step {
        Step::Type(text) => { exited(sess)?; sess.send(text.as_bytes())?; sess.pump(SETTLE)?; }
        Step::Key(k) => { exited(sess)?; sess.send(&key_bytes(k)?)?; sess.pump(SETTLE)?; }
        Step::Resize { cols, rows } => { exited(sess)?; sess.resize(*cols, *rows)?; sess.pump(SETTLE * 2)?; }
        Step::WaitFor { text, timeout } => {
            let re = Regex::new(text)?;
            let until = Instant::now() + timeout.unwrap_or(DEFAULT_STEP_TIMEOUT);
            loop {
                sess.pump(Duration::from_millis(100))?;
                if re.is_match(&sess.screen()) { break; }
                exited(sess)?;
                if Instant::now() >= until { anyhow::bail!("{text:?} did not appear"); }
            }
        }
        Step::ExpectScreen { contains, not_contains } => {
            sess.pump(SETTLE)?;
            let screen = sess.screen();
            for p in contains {
                if !Regex::new(p)?.is_match(&screen) { anyhow::bail!("screen does not match {p:?}"); }
            }
            for p in not_contains {
                if Regex::new(p)?.is_match(&screen) { anyhow::bail!("screen matches forbidden {p:?}"); }
            }
        }
        Step::ExpectJournal { argv_prefix, outcome, error } => {
            let recs = journal(home);
            let new = &recs[(*journal_seen).min(recs.len())..];
            let err_re = error.as_deref().map(Regex::new).transpose()?;
            let hit = new.iter().any(|r| {
                argv_of(r).starts_with(argv_prefix)
                    && outcome.as_deref().is_none_or(|o| r["outcome"] == o)
                    && err_re.as_ref().is_none_or(|re| r["error"].as_str().is_some_and(|e| re.is_match(e)))
            });
            *journal_seen = recs.len();
            if !hit { anyhow::bail!("no journal record matches; new records: {new:?}"); }
        }
        Step::ExpectFile { path, contains } => {
            let p = home.join(path);
            let text = std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
            for pat in contains {
                if !Regex::new(pat)?.is_match(&text) { anyhow::bail!("{} does not match {pat:?}", p.display()); }
            }
        }
        Step::Snapshot(name) => {
            sess.pump(SETTLE)?;
            let masked = mask(&sess.screen(), &s.masks)?;
            match compare_snapshot(&ctx.snapshots_root.join(&ctx.scenario_id), name, &masked)? {
                SnapshotResult::Match => {}
                SnapshotResult::Written(p) => anyhow::bail!("new snapshot written to {}; review it and run again", p.display()),
                SnapshotResult::Differs { expected, actual } => anyhow::bail!("snapshot {name} differs\nexpected:\n{expected}\nactual:\n{actual}"),
            }
        }
        Step::ExpectExit(code) => {
            let until = Instant::now() + DEFAULT_STEP_TIMEOUT;
            loop {
                sess.pump(Duration::from_millis(100))?;
                if let Some(c) = sess.exit_status() {
                    if c != *code { anyhow::bail!("exit status {c}, expected {code}"); }
                    break;
                }
                if Instant::now() >= until { anyhow::bail!("process did not exit"); }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo build --bin scope && cargo test -p scope-tui-harness`
Expected: PASS. If a test is flaky, raise its wait, never add a fixed sleep; record the cause in the commit message.

- [ ] **Step 6: Commit**

```bash
git add crates/scope-tui-harness
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(harness): PTY session and step runner"
```

### Task 13: Evidence, report, runner binary, first scenario, `just tui-test`

**Files:**
- Create: `crates/scope-tui-harness/src/evidence.rs`, `src/report.rs`, `src/main.rs` (replace the stub)
- Create: `tests/tui/scenarios/session-basics.yaml`
- Modify: `justfile` (recipe `tui-test`)
- Modify: `src/lib.rs` (`pub mod evidence; pub mod report;`)

**Interfaces:**
- Consumes: `run::{run_scenario, RunCtx, Outcome}`, `scenario::{load, Tier}`.
- Produces:
  - `evidence::save(dir: &Path, out: &Outcome, sess: &Session, home: &Path) -> Result<()>` writes `screen.txt`, `pty.raw`, `tui-events.jsonl`, `steps.jsonl`, `failure.txt`.
  - `report::Line { name, tier, result: Result3, ms }`, `enum Result3 { Pass, Fail, Skip }`; `report::print(lines: &[Line]) -> bool` (true when no offline failure).
  - Binary: `scope-tui-harness run [--tier offline|live] [--jobs N] [--keep] [--scenarios DIR] [--binary PATH] [NAME...]`. Exit 1 when an offline scenario fails. (The coverage check comes in the scenarios plan.)

- [ ] **Step 1: Write the failing test** in `report.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_failure_does_not_fail_the_run() {
        let lines = vec![
            Line { name: "a".into(), tier: Tier::Offline, result: Result3::Pass, ms: 10 },
            Line { name: "b".into(), tier: Tier::Live, result: Result3::Fail, ms: 10 },
        ];
        assert!(gate(&lines));
    }

    #[test]
    fn test_offline_failure_fails_the_run() {
        let lines = vec![Line { name: "a".into(), tier: Tier::Offline, result: Result3::Fail, ms: 10 }];
        assert!(!gate(&lines));
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p scope-tui-harness report`
Expected: compile error.

- [ ] **Step 3: Implement `report.rs`, `evidence.rs` and `main.rs`**

`report.rs`:

```rust
//! The run report (design 8.2).

use crate::scenario::Tier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result3 { Pass, Fail, Skip }

#[derive(Debug, Clone)]
pub struct Line { pub name: String, pub tier: Tier, pub result: Result3, pub ms: u128 }

/// True when no offline scenario failed. Live results never fail the run.
pub fn gate(lines: &[Line]) -> bool {
    !lines.iter().any(|l| l.tier == Tier::Offline && l.result == Result3::Fail)
}

/// Prints one line per scenario and a summary. Returns `gate(lines)`.
pub fn print(lines: &[Line]) -> bool {
    for l in lines {
        let r = match l.result { Result3::Pass => "PASS", Result3::Fail => "FAIL", Result3::Skip => "SKIP" };
        println!("{r}  {:<7} {:<45} {} ms", format!("{:?}", l.tier).to_lowercase(), l.name, l.ms);
    }
    let count = |t, r| lines.iter().filter(|l| l.tier == t && l.result == r).count();
    println!(
        "\noffline: {} pass, {} fail   live: {} pass, {} fail (not gating)",
        count(Tier::Offline, Result3::Pass), count(Tier::Offline, Result3::Fail),
        count(Tier::Live, Result3::Pass), count(Tier::Live, Result3::Fail)
    );
    gate(lines)
}
```

`evidence.rs`:

```rust
//! Evidence of a failed scenario (design 8.1).

use crate::run::Outcome;
use crate::session::{data_dir, Session};
use anyhow::Result;
use std::path::Path;

pub fn save(dir: &Path, out: &Outcome, sess: &Session, home: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("screen.txt"), sess.screen())?;
    std::fs::write(dir.join("pty.raw"), sess.raw())?;
    let events = data_dir(home).join("tui-events.jsonl");
    if events.exists() {
        std::fs::copy(&events, dir.join("tui-events.jsonl"))?;
    }
    let steps: Vec<String> = out.steps.iter().map(|s| serde_json::to_string(s).unwrap_or_default()).collect();
    std::fs::write(dir.join("steps.jsonl"), steps.join("\n"))?;
    std::fs::write(dir.join("failure.txt"), out.failure.clone().unwrap_or_default())?;
    Ok(())
}
```

`main.rs`:

```rust
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use scope_tui_harness::{evidence, report::{self, Line, Result3}, run::{run_scenario, RunCtx}, scenario::{self, Tier}};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Runs YAML scenarios against the real scope TUI")]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Clone, Copy, ValueEnum)]
enum TierArg { Offline, Live }

#[derive(Subcommand)]
enum Cmd {
    /// Run scenarios.
    Run {
        #[arg(long, value_enum, default_value = "offline")] tier: TierArg,
        #[arg(long, default_value_t = std::thread::available_parallelism().map_or(2, |n| n.get()))] jobs: usize,
        #[arg(long)] keep: bool,
        #[arg(long, default_value = "tests/tui/scenarios")] scenarios: PathBuf,
        #[arg(long, default_value = "target/debug/scope")] binary: PathBuf,
        /// Scenario names (file stems). All when empty.
        names: Vec<String>,
    },
}

fn main() -> Result<()> {
    let Cmd::Run { tier, jobs, keep, scenarios, binary, names } = Cli::parse().cmd;
    let tier = match tier { TierArg::Offline => Tier::Offline, TierArg::Live => Tier::Live };
    let run_id = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&scenarios)?.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .filter(|p| names.is_empty() || names.iter().any(|n| p.file_stem().is_some_and(|s| s == n.as_str())))
        .collect();
    files.sort();
    let lines = std::sync::Mutex::new(Vec::new());
    let queue = std::sync::Mutex::new(files);
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| loop {
                let Some(path) = queue.lock().unwrap().pop() else { break };
                let id = path.file_stem().unwrap().to_string_lossy().to_string();
                let line = match scenario::load(&path) {
                    Err(e) => { eprintln!("{id}: {e:#}"); Line { name: id, tier, result: Result3::Fail, ms: 0 } }
                    Ok(s) if !s.tiers.contains(&tier) => Line { name: id, tier, result: Result3::Skip, ms: 0 },
                    Ok(s) => {
                        let ctx = RunCtx { binary: binary.clone(), fixtures_root: "tests/tui/fixtures".into(), snapshots_root: "tests/tui/snapshots".into(), scenario_id: id.clone() };
                        let t = Instant::now();
                        match run_scenario(&s, tier, &ctx) {
                            Ok((out, sess, home)) => {
                                if !out.passed {
                                    let dir = PathBuf::from("target/tui-harness").join(&run_id).join(&id);
                                    let _ = evidence::save(&dir, &out, &sess, home.path());
                                    eprintln!("{id}: {}\n  evidence: {}", out.failure.clone().unwrap_or_default(), dir.display());
                                }
                                if keep { let _ = home.keep(); }
                                Line { name: id, tier, result: if out.passed { Result3::Pass } else { Result3::Fail }, ms: t.elapsed().as_millis() }
                            }
                            Err(e) => { eprintln!("{id}: {e:#}"); Line { name: id, tier, result: Result3::Fail, ms: t.elapsed().as_millis() } }
                        }
                    }
                };
                lines.lock().unwrap().push(line);
            });
        }
    });
    let mut lines = lines.into_inner().unwrap();
    lines.sort_by(|a, b| a.name.cmp(&b.name));
    std::process::exit(if report::print(&lines) { 0 } else { 1 });
}
```

(Live tier retry-once and drift come in the scenarios plan; the binary already accepts `--tier live`.)

`tests/tui/scenarios/session-basics.yaml`:

```yaml
name: session basics
tiers: [offline]
steps:
  - type: "ctx"
  - key: Enter
  - wait_for: { text: "(?i)chain", timeout: 10s }
  - expect_journal: { argv_prefix: [], outcome: session }
  - type: "help"
  - key: Enter
  - wait_for: { text: "market", timeout: 10s }
  - resize: { cols: 80, rows: 24 }
  - expect_screen: { not_contains: ["panicked"] }
  - key: Ctrl-D
  - expect_exit: 0
```

`justfile`:

```make
# Run the TUI harness offline tier (release gate). Builds scope first.
tui-test *ARGS:
    cargo build --bin scope
    cargo run -q -p scope-tui-harness -- run --tier offline {{ARGS}}
```

- [ ] **Step 4: Run the tests and the recipe**

Run: `cargo test -p scope-tui-harness && just tui-test`
Expected: tests PASS; the recipe prints `PASS  offline session-basics …` and `offline: 1 pass, 0 fail`, exit 0.

- [ ] **Step 5: Prove the gate can fail**

Change `wait_for: { text: "market" …` to `"NOT-ON-SCREEN"`, run `just tui-test`, confirm exit 1 and an evidence directory under `target/tui-harness/<run-id>/session-basics/` with all five files. Revert.

- [ ] **Step 6: Gates, commit, push, PR**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/scope-tui-harness tests/tui justfile
git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit -m "feat(harness): runner binary, evidence, report and just tui-test"
GH_TOKEN=$(gh auth token --user robot-accomplice) git push -u origin feature/tui-harness
GH_TOKEN=$(gh auth token --user robot-accomplice) gh pr create --repo robot-accomplice/scope-blockchain-analysis --base develop --title "TUI test harness: runner" --body "Implements the TUI test harness design, sections 4, 5, 8.1 and 8.2. Refs #47."
```

---

## Part 4: Scenarios, coverage, fixtures, live tier, release (spec steps 4 to 6)

Write this part's detailed plan when Parts 2 and 3 are merged. **Why:** its tasks are recorded fixtures and scenarios written against the real runner and replay transport; their exact step syntax, journal fields and fixture files exist only after those merge.

| Plan | Spec | Content |
|---|---|---|
| S | 6, 7 | Coverage check from `Cli::command()` with `coverage-exempt.yaml`; one offline scenario per leaf command and each TUI feature in 6.2; `just tui-record`; live tier with retry-once and API drift; `just tui-smoke`. |
| R | 8.3, 1 "First use" | `docs/RELEASE.md` steps; run against the 0.7.0 candidate; file defects; hand the report to the usability audit. |
