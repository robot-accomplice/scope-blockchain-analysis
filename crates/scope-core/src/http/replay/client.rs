//! Record/replay transport for the TUI test harness (design section 3).

use super::key::{SECRET_HEADERS, body_sha256, clean_url, fixture_path};
use crate::error::ScopeError;
use crate::http::{HttpClient, Request, Response};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Fewer characters than this match text by chance, so a secret this short is not scanned for.
const MIN_SECRET_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Replay,
    Record,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureRequest {
    pub method: String,
    pub url: String,
    pub body_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub recorded: String,
    pub request: FixtureRequest,
    pub responses: Vec<RecordedResponse>,
}

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
        Self {
            mode: Mode::Replay,
            dir,
            inner: None,
            secrets: vec![],
            served: Default::default(),
            written: Default::default(),
        }
    }

    pub fn record(dir: PathBuf, inner: Arc<dyn HttpClient>, secrets: Vec<String>) -> Self {
        let secrets = secrets
            .into_iter()
            .filter(|s| s.len() >= MIN_SECRET_LEN)
            .collect();
        Self {
            mode: Mode::Record,
            dir,
            inner: Some(inner),
            secrets,
            served: Default::default(),
            written: Default::default(),
        }
    }

    fn miss(req: &Request) -> ScopeError {
        ScopeError::Network(format!(
            "no fixture for {} {}; run `just tui-record <scenario>`",
            req.method,
            clean_url(&req.url)
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
                Ok(Response {
                    status_code: r.status,
                    headers: r.headers.clone().into_iter().collect(),
                    body: r.body.clone(),
                })
            }
            Mode::Record => {
                let inner = self
                    .inner
                    .as_ref()
                    .expect("record mode has an inner transport");
                let resp = inner.send(req.clone()).await?;
                let recorded = RecordedResponse {
                    status: resp.status_code,
                    headers: resp
                        .headers
                        .iter()
                        .filter(|(k, _)| !SECRET_HEADERS.contains(&k.to_lowercase().as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    body: resp.body.clone(),
                };
                let mut written = self.written.lock().expect("written lock");
                let mut fixture = if written.contains(&path) {
                    Self::read(&path)
                } else {
                    None
                }
                .unwrap_or_else(|| Fixture {
                    recorded: chrono::Utc::now().to_rfc3339(),
                    request: FixtureRequest {
                        method: req.method.to_uppercase(),
                        url: clean_url(&req.url),
                        body_sha256: body_sha256(req.body.as_deref()),
                    },
                    responses: vec![],
                });
                fixture.responses.push(recorded);
                let text = serde_json::to_string_pretty(&fixture)
                    .map_err(|e| ScopeError::Network(format!("cannot serialize fixture: {e}")))?;
                if self.secrets.iter().any(|s| text.contains(s.as_str())) {
                    return Err(ScopeError::Network(format!(
                        "fixture for {} {} would contain a configured secret; not written",
                        req.method,
                        clean_url(&req.url)
                    )));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        ScopeError::Network(format!("cannot create {}: {e}", parent.display()))
                    })?;
                }
                std::fs::write(&path, text).map_err(|e| {
                    ScopeError::Network(format!("cannot write {}: {e}", path.display()))
                })?;
                written.insert(path);
                Ok(resp)
            }
        }
    }
}

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
            Ok(Response {
                status_code: 200,
                headers: [
                    ("content-type".into(), "application/json".into()),
                    ("set-cookie".into(), "s=1".into()),
                ]
                .into(),
                body: body.into(),
            })
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
        let err = rep
            .send(Request::get("https://api.example.com/missing"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no fixture for GET https://api.example.com/missing"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn test_record_refuses_a_fixture_that_contains_a_configured_secret() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(Script(Mutex::new(vec![r#"{"echo":"MYKEY-0123456789"}"#])));
        let rec =
            ReplayHttpClient::record(dir.path().into(), inner, vec!["MYKEY-0123456789".into()]);
        let err = rec
            .send(Request::get("https://api.example.com/echo"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("configured secret"), "{err}");
        assert!(
            walk(dir.path()).is_empty(),
            "no fixture file may be written"
        );
    }

    #[tokio::test]
    async fn test_stored_url_has_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(Script(Mutex::new(vec!["{}"])));
        let rec = ReplayHttpClient::record(dir.path().into(), inner, vec![]);
        rec.send(Request::get("https://api.example.com/x?Token=abc123"))
            .await
            .unwrap();
        let text: String = walk(dir.path())
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect();
        assert!(!text.contains("abc123"), "{text}");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
