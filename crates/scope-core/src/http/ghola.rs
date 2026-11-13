//! Ghola sidecar HTTP client.
//!
//! Forwards all HTTP requests to a locally running
//! [Ghola](https://github.com/robot-accomplice/ghola) sidecar
//! (`127.0.0.1:18789`). When stealth mode is enabled, the sidecar
//! applies temporal drift and ghost signing to every outgoing request.
//!
//! The sidecar is an external Go binary. When nothing answers on that
//! address, `http::shared::build` starts it with [`start_sidecar`]
//! (`<binary> --serve`, the binary found by `ghola_install::find`). The
//! sidecar it started is kept here until [`stop_spawned_sidecar`] stops it;
//! the caller decides when (the CLI asks at exit).

use super::{HttpClient, Request, Response};
use crate::error::ScopeError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

pub(crate) const SIDECAR_ADDR: &str = "127.0.0.1:18789";
const SIDECAR_URL: &str = "http://127.0.0.1:18789";
/// How long a started sidecar has to answer.
pub(crate) const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Connect, write and read timeout of the `GET /health` probe.
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
/// The bridge token is this many hex characters.
const TOKEN_HEX_LEN: usize = 64;
/// The HTTP status with which the bridge refuses a missing or wrong token.
const STATUS_UNAUTHORIZED: u16 = 401;

/// The sidecar this process started and has not stopped.
static SPAWNED: Mutex<Option<Child>> = Mutex::new(None);

#[derive(Serialize)]
struct BridgeRequest {
    url: String,
    method: String,
    headers: HashMap<String, String>,
    body: String,
    drift: bool,
    ghost: bool,
    retries: i32,
    buffer_size: u32,
}

#[derive(Deserialize)]
struct BridgeResponse {
    status_code: u16,
    headers: HashMap<String, String>,
    body: String,
    #[serde(default)]
    error: String,
}

/// HTTP client that forwards requests to the Ghola sidecar bridge
/// running on `127.0.0.1:18789`. When `stealth` is `true`, the bridge
/// applies temporal drift and ghost signing to every request.
///
/// Every request carries the bridge token as a bearer token. The bridge
/// writes a new token to a file each time it starts, so after a 401 the
/// client reads the file once more and sends the request once more. When
/// no token file exists, requests go without a token.
pub struct GholaHttpClient {
    client: reqwest::Client,
    stealth: bool,
    buffer_size: u32,
    base_url: String,
    token_file: PathBuf,
    /// `None` when no token file existed: an older bridge, which checks none.
    token: RwLock<Option<String>>,
}

impl GholaHttpClient {
    /// Creates a new client that talks to an already-running sidecar. It
    /// reads the bridge token from `token_file` now.
    ///
    /// # Errors
    ///
    /// Returns `Network` when the token file exists but cannot be read or
    /// does not hold a token, or when the HTTP client cannot be built. The token value is
    /// never part of an error.
    pub fn new(stealth: bool, buffer_size: u32, token_file: &Path) -> Result<Self, ScopeError> {
        Self::connect(
            stealth,
            buffer_size,
            SIDECAR_URL,
            token_file,
            Duration::from_secs(30),
        )
    }

    /// Creates a client pointing at a custom URL and token file (for testing).
    #[cfg(test)]
    pub fn with_base_url(
        stealth: bool,
        buffer_size: u32,
        base_url: &str,
        token_file: &Path,
    ) -> Result<Self, ScopeError> {
        Self::connect(
            stealth,
            buffer_size,
            base_url,
            token_file,
            Duration::from_secs(5),
        )
    }

    fn connect(
        stealth: bool,
        buffer_size: u32,
        base_url: &str,
        token_file: &Path,
        timeout: Duration,
    ) -> Result<Self, ScopeError> {
        let token = read_token(token_file)?;
        if token.is_none() {
            // A bridge that writes no token file has no token check.
            tracing::info!("no bridge token file; sending without token");
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| {
                ScopeError::Network(format!("failed to build ghola bridge client: {e}"))
            })?;
        Ok(Self {
            client,
            stealth,
            buffer_size,
            base_url: base_url.to_string(),
            token_file: token_file.to_path_buf(),
            token: RwLock::new(token),
        })
    }

    fn current_token(&self) -> Option<String> {
        self.token.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Posts `bridge_req`, with `token` as a bearer token when there is one.
    /// Returns the bridge's HTTP status and body.
    async fn post(
        &self,
        bridge_req: &BridgeRequest,
        token: Option<&str>,
    ) -> Result<(u16, Vec<u8>), ScopeError> {
        let mut builder = self.client.post(&self.base_url).json(bridge_req);
        if let Some(token) = token {
            builder = builder.bearer_auth(token);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| ScopeError::Network(format!("failed to reach ghola sidecar: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| ScopeError::Network(format!("invalid sidecar response: {e}")))?;
        Ok((status, body.to_vec()))
    }
}

/// The `error` field of a bridge reply, or an empty string when the body
/// is not a bridge reply.
fn refusal_reason(body: &[u8]) -> String {
    serde_json::from_slice::<BridgeResponse>(body)
        .map(|r| r.error)
        .unwrap_or_default()
}

/// The error for a bridge reply whose HTTP status is not 2xx. The bridge
/// puts the reason in the `error` field of the body. `token_file` is the
/// path that was read and what was found there; it is a path, not a secret.
/// It helps when ghola and scope-bca resolve the config folder differently.
fn refusal(status: u16, body: &[u8], token_file: Option<(&Path, &str)>) -> ScopeError {
    let reason = refusal_reason(body);
    let mut message = format!("sidecar refused the request ({status})");
    if !reason.is_empty() {
        message.push_str(&format!(": {reason}"));
    }
    if let Some((path, state)) = token_file {
        message.push_str(&format!(" (token file: {}, {state})", path.display()));
    }
    ScopeError::Network(message)
}

#[async_trait]
impl HttpClient for GholaHttpClient {
    async fn send(&self, request: Request) -> Result<Response, ScopeError> {
        let bridge_req = BridgeRequest {
            url: request.url,
            method: request.method,
            headers: request.headers,
            body: request.body.unwrap_or_default(),
            drift: self.stealth,
            ghost: self.stealth,
            retries: 0,
            buffer_size: self.buffer_size,
        };

        let token = self.current_token();
        let (mut status, mut body) = self.post(&bridge_req, token.as_deref()).await?;
        // What the last look at the token file found; the 401 error names it.
        let mut file_state = if token.is_some() { "read" } else { "not found" };

        if status == STATUS_UNAUTHORIZED {
            tracing::warn!(
                status,
                reason = %refusal_reason(&body),
                "ghola bridge refused the request; reading the token file again"
            );
            // The bridge makes a new token at each start. Read it once more
            // and send the request once more; a second 401 is final. With
            // no token file there is nothing new to send.
            match read_token(&self.token_file) {
                Ok(Some(fresh)) => {
                    file_state = "read";
                    let changed = Some(&fresh) != token.as_ref();
                    *self.token.write().unwrap_or_else(|e| e.into_inner()) = Some(fresh.clone());
                    tracing::info!(changed, "token re-read; sending the request once more");
                    (status, body) = self.post(&bridge_req, Some(&fresh)).await?;
                    tracing::info!(status, "request after the token re-read finished");
                }
                Ok(None) => {
                    file_state = "not found";
                    tracing::info!("token re-read: no token file; no retry");
                }
                Err(e) => {
                    file_state = "not usable";
                    tracing::warn!(error = %e, "token re-read failed; no retry");
                }
            }
        }

        if !(200..300).contains(&status) {
            tracing::warn!(status, reason = %refusal_reason(&body), "ghola bridge refused the request");
            return Err(refusal(
                status,
                &body,
                (status == STATUS_UNAUTHORIZED).then_some((self.token_file.as_path(), file_state)),
            ));
        }

        let bridge_resp: BridgeResponse = serde_json::from_slice(&body)
            .map_err(|e| ScopeError::Network(format!("invalid sidecar response: {e}")))?;

        if !bridge_resp.error.is_empty() {
            return Err(ScopeError::Network(format!(
                "sidecar error: {}",
                bridge_resp.error
            )));
        }

        Ok(Response {
            status_code: bridge_resp.status_code,
            headers: bridge_resp.headers,
            body: bridge_resp.body,
        })
    }
}

/// Reads the bridge token from `path`. `Ok(None)` means there is no file:
/// a bridge that writes none has no token check. The token is trimmed and
/// must be [`TOKEN_HEX_LEN`] hex characters. An error names the path, never
/// the content.
fn read_token(path: &Path) -> Result<Option<String>, ScopeError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ScopeError::Network(format!(
                "cannot read the ghola bridge token at {}: {e}",
                path.display()
            )));
        }
    };
    let token = raw.trim();
    if token.len() != TOKEN_HEX_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ScopeError::Network(format!(
            "the ghola bridge token file {} does not hold {TOKEN_HEX_LEN} hex characters",
            path.display()
        )));
    }
    Ok(Some(token.to_owned()))
}

/// True when the sidecar answers `GET /health` with status 200.
pub(crate) fn is_bridge_running() -> bool {
    probe_health(SIDECAR_ADDR)
}

/// Sends a raw `GET /health` to `addr` (also used as the Host header) and
/// returns true only for status 200. A plain TCP connect is not enough: the
/// port opens before the bridge is ready, and a listener that is not a
/// bridge must not count.
fn probe_health(addr: &str) -> bool {
    let Ok(sock) = addr.parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&sock, PROBE_TIMEOUT) else {
        return false;
    };
    if stream.set_read_timeout(Some(PROBE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(PROBE_TIMEOUT)).is_err()
    {
        return false;
    }
    let probe = format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let mut status_line = [0u8; 12]; // "HTTP/1.1 200"
    stream.write_all(probe.as_bytes()).is_ok()
        && stream.read_exact(&mut status_line).is_ok()
        && status_line.starts_with(b"HTTP/1.")
        && &status_line[9..] == b"200"
}

fn spawned() -> std::sync::MutexGuard<'static, Option<Child>> {
    SPAWNED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts `binary --serve` and waits up to `timeout` for the sidecar to
/// answer on the sidecar's address (`127.0.0.1:18789`). Returns its PID. The sidecar is kept until
/// [`stop_spawned_sidecar`]; it is not stopped when this process exits.
///
/// It blocks the calling thread while it waits.
///
/// # Errors
///
/// Returns `IoError` when the binary cannot run; `Network` when it exits
/// or does not answer in time (it is stopped then).
pub fn start_sidecar(binary: &Path, timeout: Duration) -> Result<u32, ScopeError> {
    let mut child = Command::new(binary)
        .arg("--serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            ScopeError::io_context(e, format!("failed to start {} --serve", binary.display()))
        })?;
    let start = Instant::now();
    loop {
        if is_bridge_running() {
            let pid = child.id();
            *spawned() = Some(child);
            return Ok(pid);
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ScopeError::Network(format!(
                "{} --serve exited ({status}) before it answered on {SIDECAR_ADDR}",
                binary.display()
            )));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ScopeError::Network(format!(
                "the ghola sidecar did not answer on {SIDECAR_ADDR} within {timeout:?}"
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// The PID of the sidecar this process started, while it runs.
pub fn spawned_sidecar() -> Option<u32> {
    let mut guard = spawned();
    match guard.as_mut().map(Child::try_wait) {
        Some(Ok(None)) => guard.as_ref().map(Child::id),
        _ => {
            *guard = None;
            None
        }
    }
}

/// Stops the sidecar this process started. Returns its PID, or `None`
/// when this process started none.
pub fn stop_spawned_sidecar() -> Option<u32> {
    let mut child = spawned().take()?;
    let _ = child.kill();
    let _ = child.wait();
    Some(child.id())
}

/// Keeps `child` as the sidecar this process started, so tests elsewhere
/// (the CLI's exit question) can stop it without a sidecar port.
#[cfg(any(test, feature = "test-support"))]
pub fn adopt_sidecar(child: Child) {
    *spawned() = Some(child);
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::http::test_stub::{retry_while_busy, script};

    const TOKEN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TOKEN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const REFUSAL_BODY: &str =
        r#"{"status_code":0,"headers":{},"body":"","error":"missing or wrong bridge token"}"#;
    const OK_BODY: &str = r#"{"status_code":200,"headers":{},"body":"ok","error":""}"#;

    /// A token file in `dir` holding `content`. Tests use a temp folder, never
    /// the real config folder.
    fn write_token(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("bridge-token");
        std::fs::write(&path, content).unwrap();
        path
    }

    /// A client for `url` with a temp token file that holds `TOKEN_A`. The
    /// folder lives as long as the returned guard.
    fn test_client(
        stealth: bool,
        buffer_size: u32,
        url: &str,
    ) -> (GholaHttpClient, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_token(dir.path(), TOKEN_A);
        let client = GholaHttpClient::with_base_url(stealth, buffer_size, url, &path).unwrap();
        (client, dir)
    }

    #[test]
    fn test_bridge_request_serialization() {
        let req = BridgeRequest {
            url: "https://example.com".to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: String::new(),
            drift: true,
            ghost: false,
            retries: 3,
            buffer_size: 4096,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"drift\":true"));
        assert!(json.contains("\"ghost\":false"));
        assert!(json.contains("\"retries\":3"));
        assert!(json.contains("\"buffer_size\":4096"));
    }

    #[test]
    fn test_bridge_response_deserialization() {
        let json = r#"{"status_code":200,"headers":{},"body":"ok","error":""}"#;
        let resp: BridgeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, "ok");
        assert!(resp.error.is_empty());
    }

    #[test]
    fn test_bridge_response_with_error() {
        let json = r#"{"status_code":0,"headers":{},"body":"","error":"connection refused"}"#;
        let resp: BridgeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error, "connection refused");
    }

    #[test]
    fn test_bridge_response_missing_error_field() {
        let json = r#"{"status_code":200,"headers":{},"body":"data"}"#;
        let resp: BridgeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.is_empty());
    }

    #[test]
    fn test_ghola_client_creation() {
        let (client, _dir) = test_client(true, 4096, SIDECAR_URL);
        assert!(client.stealth);
        assert_eq!(client.current_token().as_deref(), Some(TOKEN_A));
    }

    #[test]
    fn test_ghola_client_creation_stealth_off() {
        let (client, _dir) = test_client(false, 4096, SIDECAR_URL);
        assert!(!client.stealth);
    }

    /// Serializes the tests that use the spawned sidecar or its port.
    static SIDECAR: Mutex<()> = Mutex::new(());

    fn sidecar_lock() -> std::sync::MutexGuard<'static, ()> {
        SIDECAR.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_no_sidecar_is_spawned_by_default() {
        let _lock = sidecar_lock();
        assert_eq!(spawned_sidecar(), None);
        assert_eq!(stop_spawned_sidecar(), None);
    }

    #[test]
    fn test_start_sidecar_reports_a_missing_binary() {
        let _lock = sidecar_lock();
        let dir = tempfile::tempdir().unwrap();
        let err =
            start_sidecar(&dir.path().join("no-ghola"), Duration::from_millis(50)).unwrap_err();
        assert!(err.to_string().contains("--serve"), "{err}");
        assert_eq!(spawned_sidecar(), None);
    }

    /// A process that runs until it is stopped.
    fn long_running() -> Child {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "60", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("60");
            c
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn test_an_adopted_sidecar_is_kept_until_stopped() {
        let _lock = sidecar_lock();
        let child = long_running();
        let pid = child.id();
        adopt_sidecar(child);
        assert_eq!(spawned_sidecar(), Some(pid));
        assert_eq!(stop_spawned_sidecar(), Some(pid));
        assert_eq!(spawned_sidecar(), None);
        assert_eq!(stop_spawned_sidecar(), None);
    }

    #[test]
    fn test_a_sidecar_that_exited_is_forgotten() {
        let _lock = sidecar_lock();
        let mut child = long_running();
        child.kill().unwrap();
        child.wait().unwrap();
        adopt_sidecar(child);
        assert_eq!(spawned_sidecar(), None);
        assert_eq!(stop_spawned_sidecar(), None);
    }

    /// [`start_sidecar`], retried while Linux says the fake script is busy.
    #[cfg(unix)]
    fn start_retrying(ghola: &Path, timeout: Duration) -> Result<u32, ScopeError> {
        retry_while_busy(|| start_sidecar(ghola, timeout))
    }

    #[cfg(unix)]
    #[test]
    fn test_start_sidecar_reports_a_sidecar_that_exits() {
        let _lock = sidecar_lock();
        if is_bridge_running() {
            eprintln!("skipped: something answers on {SIDECAR_ADDR}");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ghola = script(dir.path(), "ghola", "[ \"$1\" = --serve ] && exit 3");
        let err = start_retrying(&ghola, Duration::from_secs(10)).unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
        assert_eq!(spawned_sidecar(), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_start_sidecar_stops_a_sidecar_that_never_answers() {
        let _lock = sidecar_lock();
        if is_bridge_running() {
            eprintln!("skipped: something answers on {SIDECAR_ADDR}");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ghola = script(dir.path(), "ghola", "exec sleep 60");
        let started = Instant::now();
        let err = start_retrying(&ghola, Duration::from_millis(300)).unwrap_err();
        assert!(err.to_string().contains("did not answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(spawned_sidecar(), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_start_sidecar_keeps_a_sidecar_that_answers() {
        let _lock = sidecar_lock();
        // The sidecar's port, held here. The probe needs `GET /health` -> 200,
        // so a thread answers each probe until the test ends; an open port alone
        // does not count as a running bridge.
        let Ok(port) = std::net::TcpListener::bind(SIDECAR_ADDR) else {
            eprintln!("skipped: {SIDECAR_ADDR} is in use");
            return;
        };
        port.set_nonblocking(true).unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let answering = {
            let done = done.clone();
            std::thread::spawn(move || {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    match port.accept() {
                        Ok((mut conn, _)) => {
                            let _ = conn.set_nonblocking(false);
                            let mut request = [0u8; 512];
                            let _ = conn.read(&mut request);
                            let _ = conn.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}",
                            );
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })
        };
        let dir = tempfile::tempdir().unwrap();
        let ghola = script(dir.path(), "ghola", "exec sleep 60");
        let started = start_retrying(&ghola, Duration::from_secs(5));
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        answering.join().unwrap();
        let pid = started.unwrap();
        assert_eq!(spawned_sidecar(), Some(pid));
        assert_eq!(stop_spawned_sidecar(), Some(pid));
        assert_eq!(spawned_sidecar(), None);
    }

    #[test]
    fn test_is_bridge_running_returns_bool() {
        // Smoke test: the result depends on the host; the probe must not panic.
        let _ = is_bridge_running();
    }

    #[test]
    fn test_bridge_request_full_serialization() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer tk".to_string());
        let req = BridgeRequest {
            url: "https://api.test.com/v1".to_string(),
            method: "POST".to_string(),
            headers,
            body: r#"{"data":1}"#.to_string(),
            drift: false,
            ghost: true,
            retries: 0,
            buffer_size: 8192,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"method\":\"POST\""));
        assert!(json.contains("\"ghost\":true"));
        assert!(json.contains("\"drift\":false"));
        assert!(json.contains("\"retries\":0"));
        assert!(json.contains("\"buffer_size\":8192"));
        assert!(json.contains("Authorization"));
    }

    #[test]
    fn test_bridge_response_roundtrip() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let json = serde_json::json!({
            "status_code": 201,
            "headers": headers,
            "body": r#"{"id":42}"#,
            "error": ""
        });
        let resp: BridgeResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.status_code, 201);
        assert_eq!(resp.body, r#"{"id":42}"#);
        assert!(resp.error.is_empty());
        assert_eq!(
            resp.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }

    #[tokio::test]
    async fn test_send_to_mock_sidecar() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status_code":200,"headers":{},"body":"{\"result\":\"ok\"}","error":""}"#,
            )
            .create_async()
            .await;

        let (ghola, _dir) = test_client(true, 4096, &server.url());

        let req = Request::get("https://api.example.com/data");
        let resp = ghola.send(req).await.unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, r#"{"result":"ok"}"#);
        mock.assert_async().await;
    }

    #[test]
    fn test_sidecar_constants() {
        assert_eq!(SIDECAR_ADDR, "127.0.0.1:18789");
        assert_eq!(SIDECAR_URL, "http://127.0.0.1:18789");
        assert_eq!(SPAWN_TIMEOUT, Duration::from_secs(5));
        assert_eq!(POLL_INTERVAL, Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_send_success_via_mock() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":200,"headers":{"x-test":"yes"},"body":"{\"data\":42}","error":""}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(true, 4096, &server.url());
        let req = Request::get("https://api.example.com/v1");
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 200);
        assert!(resp.is_success());
        assert_eq!(resp.body, r#"{"data":42}"#);
        assert_eq!(resp.headers.get("x-test").map(String::as_str), Some("yes"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_with_stealth_off() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":200,"headers":{},"body":"ok","error":""}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(false, 4096, &server.url());
        let req = Request::post_json("https://api.example.com", r#"{"q":1}"#);
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, "ok");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_bridge_error_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":0,"headers":{},"body":"","error":"upstream timeout"}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(true, 4096, &server.url());
        let req = Request::get("https://api.example.com");
        let result = client.send(req).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("upstream timeout"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_invalid_json_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body("not valid json at all")
            .create_async()
            .await;

        let (client, _dir) = test_client(true, 4096, &server.url());
        let req = Request::get("https://api.example.com");
        let result = client.send(req).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid sidecar response")
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_non_success_bridge_status() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":429,"headers":{},"body":"rate limited","error":""}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(false, 4096, &server.url());
        let req = Request::get("https://api.example.com");
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 429);
        assert!(!resp.is_success());
        assert_eq!(resp.body, "rate limited");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_with_custom_headers() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":200,"headers":{},"body":"{}","error":""}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(true, 4096, &server.url());
        let req = Request::get("https://api.example.com")
            .with_header("Authorization", "Bearer token")
            .with_header("X-Chain", "ethereum");
        let resp = client.send(req).await.unwrap();

        assert!(resp.is_success());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_connection_refused() {
        let (client, _dir) = test_client(true, 4096, "http://127.0.0.1:1");
        let req = Request::get("https://api.example.com");
        let result = client.send(req).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to reach ghola sidecar")
        );
    }

    #[tokio::test]
    async fn test_send_includes_buffer_size() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"buffer_size":8192}"#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status_code":200,"headers":{},"body":"ok","error":""}"#)
            .create_async()
            .await;

        let (client, _dir) = test_client(true, 8192, &server.url());
        let req = Request::get("https://api.example.com");
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 200);
        mock.assert_async().await;
    }

    #[test]
    fn test_custom_buffer_size_stored() {
        let (client, _dir) = test_client(true, 16384, SIDECAR_URL);
        assert_eq!(client.buffer_size, 16384);
    }

    #[test]
    fn test_default_buffer_size() {
        let (client, _dir) = test_client(false, 4096, SIDECAR_URL);
        assert_eq!(client.buffer_size, 4096);
    }

    // The bridge answers 401 to a request without its token, so the token
    // must travel in the Authorization header, with the Bearer scheme.
    #[tokio::test]
    async fn test_send_sends_the_token_as_a_bearer_token() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("authorization", format!("Bearer {TOKEN_A}").as_str())
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;
        let (client, _dir) = test_client(true, 4096, &server.url());
        client.send(Request::get("https://x.test")).await.unwrap();
        mock.assert_async().await;
    }

    // The bridge makes a new token at each start. A client that built before
    // a restart must read the file again, once, and succeed with the new token.
    #[tokio::test]
    async fn test_a_401_rereads_the_token_and_retries_once() {
        let mut server = mockito::Server::new_async().await;
        let old = server
            .mock("POST", "/")
            .match_header("authorization", format!("Bearer {TOKEN_A}").as_str())
            .with_status(401)
            .with_body(REFUSAL_BODY)
            .expect(1)
            .create_async()
            .await;
        let new = server
            .mock("POST", "/")
            .match_header("authorization", format!("Bearer {TOKEN_B}").as_str())
            .with_status(200)
            .with_body(OK_BODY)
            .expect(2)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_token(dir.path(), TOKEN_A);
        let client = GholaHttpClient::with_base_url(false, 4096, &server.url(), &path).unwrap();
        write_token(dir.path(), &format!("{TOKEN_B}\n"));

        let resp = client.send(Request::get("https://x.test")).await.unwrap();
        assert_eq!(resp.body, "ok");

        // The new token is kept: the next request needs no second 401.
        client.send(Request::get("https://x.test")).await.unwrap();
        assert_eq!(client.current_token().as_deref(), Some(TOKEN_B));
        old.assert_async().await;
        new.assert_async().await;
    }

    // A second 401 is final. Without this limit a wrong token loops forever.
    // The error must carry the bridge's reason, not a parse error.
    #[tokio::test]
    async fn test_a_second_401_returns_the_reason_and_stops() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body(REFUSAL_BODY)
            .expect(2)
            .create_async()
            .await;
        let (client, dir) = test_client(true, 4096, &server.url());
        let err = client
            .send(Request::get("https://x.test"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("sidecar refused the request (401): missing or wrong bridge token"),
            "{err}"
        );
        // The path of the file that was read is in the error. When ghola and
        // scope-bca resolve the config folder differently, it is the only clue.
        let path = dir.path().join("bridge-token");
        assert!(
            err.contains(&format!("(token file: {}, read)", path.display())),
            "{err}"
        );
        mock.assert_async().await;
    }

    // A refusal other than 401 is not a token problem: no re-read, no retry.
    // The reason still reaches the user.
    #[tokio::test]
    async fn test_other_refusals_report_the_reason_without_a_retry() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(403)
            .with_body(r#"{"status_code":0,"headers":{},"body":"","error":"host not allowed"}"#)
            .expect(1)
            .create_async()
            .await;
        let (client, _dir) = test_client(true, 4096, &server.url());
        let err = client
            .send(Request::get("https://x.test"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("(403): host not allowed"), "{err}");
        mock.assert_async().await;
    }

    // When the token file is gone after a 401, there is no new token to
    // send. The client must not loop or send the old token again. The error
    // names the path and says it was not found.
    #[tokio::test]
    async fn test_a_401_with_a_missing_token_file_fails_without_a_retry() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body(REFUSAL_BODY)
            .expect(1)
            .create_async()
            .await;
        let (client, dir) = test_client(true, 4096, &server.url());
        let path = dir.path().join("bridge-token");
        std::fs::remove_file(&path).unwrap();
        let err = client
            .send(Request::get("https://x.test"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("(token file: {}, not found)", path.display())),
            "{err}"
        );
        mock.assert_async().await;
    }

    // A missing token file means a bridge that writes none (an older ghola),
    // and such a bridge checks no token. Ghola mode must keep working for it:
    // the build succeeds and the request carries no Authorization header.
    #[tokio::test]
    async fn test_a_missing_token_file_sends_no_authorization_header() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("absent");
        let client = GholaHttpClient::with_base_url(true, 4096, &server.url(), &absent).unwrap();
        assert_eq!(client.current_token(), None);
        client.send(Request::get("https://x.test")).await.unwrap();
        mock.assert_async().await;
    }

    // A bridge restarted into a new version writes its file after the client
    // was built without one. The 401 must lead to a re-read and a retry with
    // the new token.
    #[tokio::test]
    async fn test_a_token_file_that_appears_after_a_401_is_used_for_the_retry() {
        let mut server = mockito::Server::new_async().await;
        let refused = server
            .mock("POST", "/")
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(401)
            .with_body(REFUSAL_BODY)
            .expect(1)
            .create_async()
            .await;
        let accepted = server
            .mock("POST", "/")
            .match_header("authorization", format!("Bearer {TOKEN_B}").as_str())
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge-token");
        let client = GholaHttpClient::with_base_url(true, 4096, &server.url(), &path).unwrap();
        write_token(dir.path(), TOKEN_B);
        client.send(Request::get("https://x.test")).await.unwrap();
        refused.assert_async().await;
        accepted.assert_async().await;
    }

    // A token file that exists but holds no token would fail every request
    // or send a bad header. The build must fail at once with an error that
    // names the file, so shared.rs can fall back to native.
    #[test]
    fn test_an_empty_or_malformed_token_fails_the_build() {
        let dir = tempfile::tempdir().unwrap();

        let bad = [
            "".to_string(),
            "   \n".to_string(),
            "abc123".to_string(),
            format!("{TOKEN_A}0"),
            format!("{}g", &TOKEN_A[1..]),
            format!("{} {}", &TOKEN_A[..32], &TOKEN_A[32..]),
        ];
        for content in bad {
            let path = write_token(dir.path(), &content);
            let err = GholaHttpClient::new(true, 4096, &path).err().unwrap();
            assert!(err.to_string().contains("64 hex characters"), "{err}");
        }
    }

    // A token file ends with a newline when an editor or a shell wrote it.
    // The newline must not become part of the header value.
    #[test]
    fn test_read_token_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_token(dir.path(), &format!("  {TOKEN_B}\r\n"));
        assert_eq!(read_token(&path).unwrap().as_deref(), Some(TOKEN_B));
    }

    // The token is a credential. No error string may hold it, whether the
    // file is bad, the bridge refuses it, or the bridge is not there.
    #[tokio::test]
    async fn test_the_token_never_appears_in_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut errors = Vec::new();

        // A file with a token-like value that is not valid.
        let bad = format!("{}z", &TOKEN_A[1..]);
        let path = write_token(dir.path(), &bad);
        let err = GholaHttpClient::new(true, 1, &path).err().unwrap();
        errors.push((bad, err.to_string()));

        // The bridge refuses a good token twice.
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body(REFUSAL_BODY)
            .create_async()
            .await;
        let path = write_token(dir.path(), TOKEN_A);
        let client = GholaHttpClient::with_base_url(true, 1, &server.url(), &path).unwrap();
        let refused = client
            .send(Request::get("https://x.test"))
            .await
            .unwrap_err();
        errors.push((TOKEN_A.to_string(), refused.to_string()));

        // No bridge listens.
        let client = GholaHttpClient::with_base_url(true, 1, "http://127.0.0.1:1", &path).unwrap();
        let down = client
            .send(Request::get("https://x.test"))
            .await
            .unwrap_err();
        errors.push((TOKEN_A.to_string(), down.to_string()));

        for (token, err) in errors {
            assert!(!err.contains(&token), "token in error: {err}");
            assert!(!err.contains(&token[..16]), "token prefix in error: {err}");
        }
    }

    /// Serves one canned reply to each connection and sends the requests it
    /// read on the returned channel. Returns the address as `ip:port`.
    fn serve_replies(reply: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (addr, rx)
    }

    // The port opens before the bridge is ready, so only a 200 answer to
    // GET /health counts. The probe must also send the Host header the
    // bridge checks, and close the connection.
    #[test]
    fn test_probe_health_true_only_for_200() {
        let (addr, requests) =
            serve_replies("HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}");
        assert!(probe_health(&addr));
        let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(request.starts_with("GET /health HTTP/1.1\r\n"), "{request}");
        assert!(request.contains(&format!("Host: {addr}\r\n")), "{request}");
        assert!(request.contains("Connection: close\r\n"), "{request}");
    }

    // A listener that answers 404 is a bridge without /health, or not a
    // bridge at all. Neither may count as running.
    #[test]
    fn test_probe_health_false_for_404_and_other_statuses() {
        for reply in [
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n",
            "not http at all, but long enough",
        ] {
            let (addr, _requests) = serve_replies(reply);
            assert!(!probe_health(&addr), "{reply}");
        }
    }

    // Nothing listens, or the listener never answers: the probe says "not
    // running" within its short timeouts and does not hang.
    #[test]
    fn test_probe_health_false_without_a_listener_or_an_answer() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = free.local_addr().unwrap().to_string();
        drop(free);
        assert!(!probe_health(&addr));

        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let started = Instant::now();
        assert!(!probe_health(&silent.local_addr().unwrap().to_string()));
        assert!(started.elapsed() < Duration::from_secs(2));

        assert!(!probe_health("not an address"));
    }

    // Why: tests must never read or write the user's real config folder.
    #[test]
    fn test_token_file_is_in_the_test_sandbox() {
        let path = crate::paths::ghola_bridge_token_file().unwrap();
        assert!(path.ends_with(Path::new("ghola").join("bridge-token")));
        assert!(
            path.to_string_lossy().contains("ghola-config"),
            "{}",
            path.display()
        );
    }
}
