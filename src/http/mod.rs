//! # HTTP Transport Abstraction
//!
//! Provides a trait-based abstraction over HTTP clients, allowing
//! transparent switching between direct `reqwest` calls and the
//! [Ghola](https://github.com/robot-accomplice/ghola) sidecar proxy.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::http::{HttpClient, NativeHttpClient, Request};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let client = NativeHttpClient::new()?;
//! let resp = client.send(Request::get("https://api.example.com/data")).await?;
//! println!("status={}, body_len={}", resp.status_code, resp.body.len());
//! # Ok(())
//! # }
//! ```
//!
//! When the Ghola sidecar is enabled via `config.yaml`, the same
//! `HttpClient` interface routes requests through `127.0.0.1:18789`,
//! adding temporal drift, ghost signing, and chain-aware headers.

pub mod ghola;
pub mod native;

pub use ghola::GholaHttpClient;
pub use native::NativeHttpClient;

use async_trait::async_trait;
use std::collections::HashMap;

use crate::error::ScopeError;

/// Generic HTTP request passed through the transport abstraction.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

impl Request {
    /// Convenience constructor for a GET request.
    pub fn get(url: &str) -> Self {
        Self {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Convenience constructor for a POST request with a JSON body.
    pub fn post_json(url: &str, body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            url: url.to_string(),
            method: "POST".to_string(),
            headers,
            body: Some(body.into()),
        }
    }

    /// Adds a header to the request.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

/// Generic HTTP response returned by any transport implementation.
#[derive(Debug, Clone)]
pub struct Response {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Response {
    /// Returns `true` if the response status is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    /// Deserializes the body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_str(&self.body)
    }
}

/// Trait implemented by both the native reqwest client and the ghola
/// sidecar client. Scope selects the implementation at startup based
/// on the `ghola.enabled` flag in `config.yaml`.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Sends an HTTP request and returns the response.
    async fn send(&self, request: Request) -> Result<Response, ScopeError>;
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_get() {
        let req = Request::get("https://example.com");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://example.com");
        assert!(req.body.is_none());
        assert!(req.headers.is_empty());
    }

    #[test]
    fn test_request_post_json() {
        let req = Request::post_json("https://example.com/api", r#"{"key":"value"}"#);
        assert_eq!(req.method, "POST");
        assert_eq!(req.body.as_deref(), Some(r#"{"key":"value"}"#));
        assert_eq!(
            req.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
    }

    #[test]
    fn test_request_with_header() {
        let req = Request::get("https://example.com")
            .with_header("Authorization", "Bearer token123")
            .with_header("Accept", "application/json");
        assert_eq!(req.headers.len(), 2);
        assert_eq!(
            req.headers.get("Authorization").map(String::as_str),
            Some("Bearer token123")
        );
    }

    #[test]
    fn test_response_is_success() {
        assert!(
            Response {
                status_code: 200,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
        assert!(
            Response {
                status_code: 299,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
        assert!(
            !Response {
                status_code: 404,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
        assert!(
            !Response {
                status_code: 500,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
    }

    #[test]
    fn test_response_json() {
        let resp = Response {
            status_code: 200,
            headers: HashMap::new(),
            body: r#"{"name":"test","value":42}"#.to_string(),
        };
        let parsed: serde_json::Value = resp.json().unwrap();
        assert_eq!(parsed["name"], "test");
        assert_eq!(parsed["value"], 42);
    }

    #[test]
    fn test_response_json_error() {
        let resp = Response {
            status_code: 200,
            headers: HashMap::new(),
            body: "not json".to_string(),
        };
        let result: serde_json::Result<serde_json::Value> = resp.json();
        assert!(result.is_err());
    }

    #[test]
    fn test_request_debug_formatting() {
        let req = Request::get("https://example.com");
        let debug = format!("{:?}", req);
        assert!(debug.contains("GET"));
        assert!(debug.contains("example.com"));
    }

    #[test]
    fn test_request_clone() {
        let req = Request::post_json("https://example.com", r#"{"a":1}"#)
            .with_header("X-Test", "yes");
        let cloned = req.clone();
        assert_eq!(cloned.method, "POST");
        assert_eq!(cloned.url, "https://example.com");
        assert_eq!(cloned.body, Some(r#"{"a":1}"#.to_string()));
        assert_eq!(
            cloned.headers.get("X-Test").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            cloned.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
    }

    #[test]
    fn test_response_debug_formatting() {
        let resp = Response {
            status_code: 404,
            headers: HashMap::new(),
            body: "not found".to_string(),
        };
        let debug = format!("{:?}", resp);
        assert!(debug.contains("404"));
        assert!(debug.contains("not found"));
    }

    #[test]
    fn test_response_clone() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "text/plain".to_string());
        let resp = Response {
            status_code: 200,
            headers,
            body: "hello".to_string(),
        };
        let cloned = resp.clone();
        assert_eq!(cloned.status_code, 200);
        assert_eq!(cloned.body, "hello");
        assert_eq!(
            cloned.headers.get("content-type").map(String::as_str),
            Some("text/plain")
        );
    }

    #[test]
    fn test_response_boundary_success() {
        assert!(
            !Response {
                status_code: 199,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
        assert!(
            Response {
                status_code: 200,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
        assert!(
            !Response {
                status_code: 300,
                headers: HashMap::new(),
                body: String::new()
            }
            .is_success()
        );
    }

    #[test]
    fn test_request_header_overwrite() {
        let req = Request::get("https://example.com")
            .with_header("X-Key", "first")
            .with_header("X-Key", "second");
        assert_eq!(
            req.headers.get("X-Key").map(String::as_str),
            Some("second")
        );
    }

    #[test]
    fn test_response_json_typed() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct TestData {
            name: String,
            count: u32,
        }

        let resp = Response {
            status_code: 200,
            headers: HashMap::new(),
            body: r#"{"name":"test","count":5}"#.to_string(),
        };
        let parsed: TestData = resp.json().unwrap();
        assert_eq!(
            parsed,
            TestData {
                name: "test".to_string(),
                count: 5
            }
        );
    }

    #[test]
    fn test_post_json_empty_body() {
        let req = Request::post_json("https://example.com", "");
        assert_eq!(req.body, Some(String::new()));
        assert_eq!(req.method, "POST");
    }
}
