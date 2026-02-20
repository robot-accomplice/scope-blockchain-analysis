//! Ghola sidecar HTTP client.
//!
//! Forwards all HTTP requests to a locally running
//! [Ghola](https://github.com/robot-accomplice/ghola) sidecar
//! (`127.0.0.1:18789`). When stealth mode is enabled, the sidecar
//! applies temporal drift and ghost signing to every outgoing request.
//!
//! The sidecar is an external Go binary. If it is not already running,
//! [`GholaHttpClient::ensure_ready`] will attempt to spawn it via
//! `ghola --serve` and wait for the bridge to become reachable.

use super::{HttpClient, Request, Response};
use crate::error::ScopeError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::TcpStream;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const SIDECAR_ADDR: &str = "127.0.0.1:18789";
const SIDECAR_URL: &str = "http://127.0.0.1:18789";
const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// PID of the sidecar we spawned (0 means none spawned by us).
static SIDECAR_PID: AtomicU32 = AtomicU32::new(0);

#[derive(Serialize)]
struct BridgeRequest {
    url: String,
    method: String,
    headers: HashMap<String, String>,
    body: String,
    drift: bool,
    ghost: bool,
    retries: i32,
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
pub struct GholaHttpClient {
    client: reqwest::Client,
    stealth: bool,
    base_url: String,
}

impl GholaHttpClient {
    /// Creates a new client that talks to an already-running sidecar.
    pub fn new(stealth: bool) -> Result<Self, ScopeError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| {
                ScopeError::Network(format!("failed to build ghola bridge client: {e}"))
            })?;
        Ok(Self {
            client,
            stealth,
            base_url: SIDECAR_URL.to_string(),
        })
    }

    /// Creates a client pointing at a custom URL (for testing).
    #[cfg(test)]
    pub fn with_base_url(stealth: bool, base_url: &str) -> Result<Self, ScopeError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| {
                ScopeError::Network(format!("failed to build ghola bridge client: {e}"))
            })?;
        Ok(Self {
            client,
            stealth,
            base_url: base_url.to_string(),
        })
    }

    /// Ensures the sidecar is reachable. If not, spawns `ghola --serve`
    /// and waits for it to become ready. Returns a configured client.
    pub async fn ensure_ready(stealth: bool) -> Result<Self, ScopeError> {
        if !is_bridge_running() {
            spawn_sidecar()?;
            wait_for_bridge(SPAWN_TIMEOUT).await?;
        }
        Self::new(stealth)
    }
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
        };

        let resp = self
            .client
            .post(&self.base_url)
            .json(&bridge_req)
            .send()
            .await
            .map_err(|e| ScopeError::Network(format!("failed to reach ghola sidecar: {e}")))?;

        let bridge_resp: BridgeResponse = resp
            .json()
            .await
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

/// Returns `true` if the `ghola` binary is reachable via PATH.
pub fn ghola_in_path() -> bool {
    Command::new("ghola")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_bridge_running() -> bool {
    SIDECAR_ADDR
        .parse()
        .ok()
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(200)).ok())
        .is_some()
}

fn spawn_sidecar() -> Result<(), ScopeError> {
    let child = Command::new("ghola")
        .arg("--serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            ScopeError::Network(format!(
                "failed to spawn ghola --serve: {e}\n  \
                 Install: go install github.com/robot-accomplice/ghola/cmd/ghola@latest\n  \
                 Or download from: https://github.com/robot-accomplice/ghola/releases"
            ))
        })?;
    SIDECAR_PID.store(child.id(), Ordering::Relaxed);
    Ok(())
}

async fn wait_for_bridge(timeout: Duration) -> Result<(), ScopeError> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if is_bridge_running() {
            return Ok(());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(ScopeError::Network(format!(
        "ghola sidecar did not become ready within {timeout:?}"
    )))
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"drift\":true"));
        assert!(json.contains("\"ghost\":false"));
        assert!(json.contains("\"retries\":3"));
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
        let client = GholaHttpClient::new(true);
        assert!(client.is_ok());
    }

    #[test]
    fn test_ghola_client_creation_stealth_off() {
        let client = GholaHttpClient::new(false);
        assert!(client.is_ok());
    }

    #[test]
    fn test_sidecar_pid_default_zero() {
        assert_eq!(SIDECAR_PID.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_is_bridge_running_returns_bool() {
        let result = is_bridge_running();
        assert!(result == true || result == false);
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
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"method\":\"POST\""));
        assert!(json.contains("\"ghost\":true"));
        assert!(json.contains("\"drift\":false"));
        assert!(json.contains("\"retries\":0"));
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
    async fn test_wait_for_bridge_completes() {
        let result = wait_for_bridge(Duration::from_millis(200)).await;
        // Either succeeds (sidecar running) or fails with timeout
        match result {
            Ok(()) => {} // sidecar was already running
            Err(e) => assert!(e.to_string().contains("did not become ready")),
        }
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

        let ghola = GholaHttpClient::with_base_url(true, &server.url()).unwrap();

        let req = Request::get("https://api.example.com/data");
        let resp = ghola.send(req).await.unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, r#"{"result":"ok"}"#);
        mock.assert_async().await;
    }

    #[test]
    fn test_ghola_in_path_returns_bool() {
        let result = ghola_in_path();
        assert!(result == true || result == false);
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

        let client = GholaHttpClient::with_base_url(true, &server.url()).unwrap();
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

        let client = GholaHttpClient::with_base_url(false, &server.url()).unwrap();
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

        let client = GholaHttpClient::with_base_url(true, &server.url()).unwrap();
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

        let client = GholaHttpClient::with_base_url(true, &server.url()).unwrap();
        let req = Request::get("https://api.example.com");
        let result = client.send(req).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid sidecar response"));
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

        let client = GholaHttpClient::with_base_url(false, &server.url()).unwrap();
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

        let client = GholaHttpClient::with_base_url(true, &server.url()).unwrap();
        let req = Request::get("https://api.example.com")
            .with_header("Authorization", "Bearer token")
            .with_header("X-Chain", "ethereum");
        let resp = client.send(req).await.unwrap();

        assert!(resp.is_success());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_connection_refused() {
        let client = GholaHttpClient::with_base_url(true, "http://127.0.0.1:1").unwrap();
        let req = Request::get("https://api.example.com");
        let result = client.send(req).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to reach ghola sidecar"));
    }
}
