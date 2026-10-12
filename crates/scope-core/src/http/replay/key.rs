//! Fixture keys: a request's identity with secrets removed.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const SECRET_PARAMS: &[&str] = &["apikey", "api_key", "key", "token", "access_token"];
pub const SECRET_HEADERS: &[&str] = &["authorization", "x-api-key", "cookie", "set-cookie"];

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
    format!(
        "{} {} {}",
        method.to_uppercase(),
        clean_url(url),
        body_sha256(body)
    )
}

/// Where the fixture for a request lives.
pub fn fixture_path(dir: &Path, method: &str, url: &str, body: Option<&str>) -> PathBuf {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    dir.join(host).join(format!(
        "{}.json",
        hex_sha256(fixture_key(method, url, body).as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_url_removes_secret_params_in_any_case() {
        let u = clean_url(
            "https://api.etherscan.io/v2/api?module=account&ApiKey=SECRET123&address=0x1",
        );
        assert!(!u.contains("SECRET123"), "{u}");
        assert!(!u.to_lowercase().contains("apikey"), "{u}");
        assert_eq!(
            u,
            "https://api.etherscan.io/v2/api?address=0x1&module=account"
        );
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
        let p = fixture_path(
            std::path::Path::new("/f"),
            "GET",
            "https://api.binance.com/api/v3/depth?symbol=X",
            None,
        );
        assert!(p.starts_with("/f/api.binance.com"), "{}", p.display());
        assert_eq!(p.extension().unwrap(), "json");
    }
}
