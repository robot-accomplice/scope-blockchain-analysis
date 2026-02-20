//! Native HTTP client using `reqwest` directly.
//!
//! This is the default transport when the Ghola sidecar is not configured
//! or unavailable. All requests go directly to the target endpoint.

use super::{HttpClient, Request, Response};
use crate::error::ScopeError;
use async_trait::async_trait;
use std::time::Duration;

/// Standard `reqwest`-based HTTP client used when the Ghola sidecar is
/// disabled or unavailable.
pub struct NativeHttpClient {
    client: reqwest::Client,
}

impl NativeHttpClient {
    /// Creates a new native HTTP client with a 30-second timeout.
    pub fn new() -> Result<Self, ScopeError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ScopeError::Network(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl HttpClient for NativeHttpClient {
    async fn send(&self, request: Request) -> Result<Response, ScopeError> {
        let method: reqwest::Method = request
            .method
            .parse()
            .map_err(|e| ScopeError::Network(format!("invalid HTTP method: {e}")))?;

        let mut builder = self.client.request(method, &request.url);

        for (k, v) in &request.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let resp = builder.send().await?;
        let status = resp.status().as_u16();

        let mut headers = std::collections::HashMap::new();
        for (k, v) in resp.headers() {
            headers.insert(k.to_string(), v.to_str().unwrap_or("").to_string());
        }

        let body = resp.text().await?;

        Ok(Response {
            status_code: status,
            headers,
            body,
        })
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_client_creation() {
        let client = NativeHttpClient::new();
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_send_get_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/test")
            .with_status(200)
            .with_header("x-test", "hello")
            .with_body(r#"{"ok":true}"#)
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::get(&format!("{}/test", server.url()));
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 200);
        assert!(resp.is_success());
        assert!(resp.body.contains("\"ok\":true"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_post_json_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api")
            .match_header("content-type", "application/json")
            .match_body(r#"{"key":"val"}"#)
            .with_status(201)
            .with_body(r#"{"created":true}"#)
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::post_json(
            &format!("{}/api", server.url()),
            r#"{"key":"val"}"#,
        );
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 201);
        assert!(resp.is_success());
        let parsed: serde_json::Value = resp.json().unwrap();
        assert_eq!(parsed["created"], true);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_with_custom_headers() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/auth")
            .match_header("authorization", "Bearer xyz")
            .match_header("accept", "application/json")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::get(&format!("{}/auth", server.url()))
            .with_header("Authorization", "Bearer xyz")
            .with_header("Accept", "application/json");
        let resp = client.send(req).await.unwrap();

        assert!(resp.is_success());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_non_success_status() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/missing")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::get(&format!("{}/missing", server.url()));
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 404);
        assert!(!resp.is_success());
        assert_eq!(resp.body, "not found");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_send_server_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/err")
            .with_status(500)
            .with_body("internal error")
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::get(&format!("{}/err", server.url()));
        let resp = client.send(req).await.unwrap();

        assert_eq!(resp.status_code, 500);
        assert!(!resp.is_success());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_response_headers_collected() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/headers")
            .with_status(200)
            .with_header("x-custom", "value123")
            .with_body("")
            .create_async()
            .await;

        let client = NativeHttpClient::new().unwrap();
        let req = Request::get(&format!("{}/headers", server.url()));
        let resp = client.send(req).await.unwrap();

        assert_eq!(
            resp.headers.get("x-custom").map(String::as_str),
            Some("value123")
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_empty_method_returns_error() {
        let client = NativeHttpClient::new().unwrap();
        let mut req = Request::get("https://example.com");
        req.method = String::new();
        let result = client.send(req).await;
        assert!(result.is_err());
    }
}
