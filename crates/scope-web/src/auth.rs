//! Optional bearer token authentication middleware.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

/// Middleware that validates bearer tokens when configured.
///
/// If `expected_token` is `None`, all requests pass through (no-op).
/// If `expected_token` is `Some`, requests must include a matching
/// `Authorization: Bearer <token>` header or receive a 401.
pub async fn auth_middleware(
    request: Request<Body>,
    next: Next,
    expected_token: Option<String>,
) -> Result<Response, StatusCode> {
    // No token configured — pass through
    let Some(expected) = expected_token else {
        return Ok(next.run(request).await);
    };

    // Check Authorization header
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];
            // Use constant-time comparison to prevent timing attacks
            if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
                Ok(next.run(request).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Constant-time byte comparison to prevent timing side channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, middleware, routing::get};
    use tower::ServiceExt;

    /// Helper: build a tiny router that uses our auth middleware.
    fn test_app(token: Option<String>) -> Router {
        let handler = get(|| async { "ok" });
        Router::new()
            .route("/api/test", handler)
            .layer(middleware::from_fn(
                move |req: Request<Body>, next: Next| {
                    let t = token.clone();
                    async move { auth_middleware(req, next, t).await }
                },
            ))
    }

    #[tokio::test]
    async fn test_no_token_configured_passes() {
        let app = test_app(None);
        let req = Request::builder()
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_correct_token_passes() {
        let app = test_app(Some("secret123".into()));
        let req = Request::builder()
            .uri("/api/test")
            .header("authorization", "Bearer secret123")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_wrong_token_rejected() {
        let app = test_app(Some("secret123".into()));
        let req = Request::builder()
            .uri("/api/test")
            .header("authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_missing_auth_header_rejected() {
        let app = test_app(Some("secret123".into()));
        let req = Request::builder()
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_malformed_auth_header_rejected() {
        let app = test_app(Some("secret123".into()));

        // "Basic" instead of "Bearer"
        let req = Request::builder()
            .uri("/api/test")
            .header("authorization", "Basic secret123")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_unequal() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}
