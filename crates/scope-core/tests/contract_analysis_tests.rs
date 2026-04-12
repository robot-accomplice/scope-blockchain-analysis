//! Integration tests for the contract analysis module: EOA detection,
//! error propagation, and HttpClient trait mock verification.

use async_trait::async_trait;
use scope::chains::{Balance, ChainClient, TokenBalance, Transaction};
use scope::error::{Result, ScopeError};
use scope::http;
use scope::http::HttpClient;
use std::sync::Mutex;

// ============================================================================
// Mock ChainClient that returns configurable code
// ============================================================================

struct MockChainClient {
    code_response: String,
}

impl MockChainClient {
    fn eoa() -> Self {
        Self {
            code_response: "0x".to_string(),
        }
    }

    fn contract(bytecode: &str) -> Self {
        Self {
            code_response: bytecode.to_string(),
        }
    }
}

#[async_trait]
impl ChainClient for MockChainClient {
    fn chain_name(&self) -> &str {
        "mock"
    }

    fn native_token_symbol(&self) -> &str {
        "MOCK"
    }

    async fn get_balance(&self, _address: &str) -> Result<Balance> {
        Ok(Balance {
            raw: "0".to_string(),
            formatted: "0.0".to_string(),
            decimals: 18,
            symbol: "MOCK".to_string(),
            usd_value: None,
        })
    }

    async fn enrich_balance_usd(&self, _balance: &mut Balance) {}

    async fn get_transaction(&self, _hash: &str) -> Result<Transaction> {
        Err(ScopeError::Chain("not implemented".to_string()))
    }

    async fn get_transactions(&self, _address: &str, _limit: u32) -> Result<Vec<Transaction>> {
        Ok(vec![])
    }

    async fn get_block_number(&self) -> Result<u64> {
        Ok(12345)
    }

    async fn get_token_balances(&self, _address: &str) -> Result<Vec<TokenBalance>> {
        Ok(vec![])
    }

    async fn get_code(&self, _address: &str) -> Result<String> {
        Ok(self.code_response.clone())
    }
}

// ============================================================================
// Mock HttpClient that records calls and returns canned responses
// ============================================================================

struct MockHttpClient {
    responses: Mutex<Vec<http::Response>>,
    recorded_urls: Mutex<Vec<String>>,
}

impl MockHttpClient {
    fn new(responses: Vec<http::Response>) -> Self {
        Self {
            responses: Mutex::new(responses),
            recorded_urls: Mutex::new(Vec::new()),
        }
    }

    fn urls_called(&self) -> Vec<String> {
        self.recorded_urls.lock().unwrap().clone()
    }
}

#[async_trait]
impl http::HttpClient for MockHttpClient {
    async fn send(
        &self,
        request: http::Request,
    ) -> std::result::Result<http::Response, ScopeError> {
        self.recorded_urls.lock().unwrap().push(request.url.clone());

        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(http::Response {
                status_code: 404,
                headers: std::collections::HashMap::new(),
                body: r#"{"error":"no more mock responses"}"#.to_string(),
            })
        } else {
            Ok(responses.remove(0))
        }
    }
}

// ============================================================================
// Tests: analyze_contract with EOA returns appropriate error
// ============================================================================

#[tokio::test]
async fn analyze_contract_eoa_returns_error() {
    let chain_client = MockChainClient::eoa();
    let http_client = MockHttpClient::new(vec![]);

    let result = scope::contract::analyze_contract(
        "0x1234567890abcdef1234567890abcdef12345678",
        "ethereum",
        &chain_client,
        &http_client,
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("EOA") || msg.contains("externally owned"),
        "Error should mention EOA, got: {}",
        msg
    );
}

// ============================================================================
// Tests: error types are properly propagated
// ============================================================================

#[tokio::test]
async fn analyze_contract_chain_error_propagates() {
    // A chain client that errors on get_code
    struct FailingChainClient;

    #[async_trait]
    impl ChainClient for FailingChainClient {
        fn chain_name(&self) -> &str {
            "fail"
        }
        fn native_token_symbol(&self) -> &str {
            "FAIL"
        }
        async fn get_balance(&self, _: &str) -> Result<Balance> {
            Err(ScopeError::Chain("balance error".into()))
        }
        async fn enrich_balance_usd(&self, _: &mut Balance) {}
        async fn get_transaction(&self, _: &str) -> Result<Transaction> {
            Err(ScopeError::Chain("tx error".into()))
        }
        async fn get_transactions(&self, _: &str, _: u32) -> Result<Vec<Transaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> Result<u64> {
            Ok(0)
        }
        async fn get_token_balances(&self, _: &str) -> Result<Vec<TokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _: &str) -> Result<String> {
            Err(ScopeError::Chain("RPC unavailable".to_string()))
        }
    }

    let chain_client = FailingChainClient;
    let http_client = MockHttpClient::new(vec![]);

    let result = scope::contract::analyze_contract(
        "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "ethereum",
        &chain_client,
        &http_client,
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("RPC unavailable"));
}

// ============================================================================
// Tests: HttpClient mock records calls correctly
// ============================================================================

#[tokio::test]
async fn mock_http_client_records_requests() {
    let mock = MockHttpClient::new(vec![
        http::Response {
            status_code: 200,
            headers: std::collections::HashMap::new(),
            body: r#"{"result":[]}"#.to_string(),
        },
        http::Response {
            status_code: 200,
            headers: std::collections::HashMap::new(),
            body: r#"{"ok":true}"#.to_string(),
        },
    ]);

    // Send two requests through the mock
    let resp1 = mock
        .send(http::Request::get("https://api.example.com/first"))
        .await
        .unwrap();
    assert_eq!(resp1.status_code, 200);
    assert!(resp1.body.contains("result"));

    let resp2 = mock
        .send(http::Request::post_json(
            "https://api.example.com/second",
            r#"{"query":"test"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp2.status_code, 200);
    assert!(resp2.body.contains("ok"));

    // Verify recorded URLs
    let urls = mock.urls_called();
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0], "https://api.example.com/first");
    assert_eq!(urls[1], "https://api.example.com/second");
}

#[tokio::test]
async fn mock_http_client_returns_404_when_exhausted() {
    let mock = MockHttpClient::new(vec![]);

    let resp = mock
        .send(http::Request::get("https://api.example.com/empty"))
        .await
        .unwrap();
    assert_eq!(resp.status_code, 404);
}

// ============================================================================
// Tests: analyze_contract with valid bytecode proceeds through pipeline
// ============================================================================

#[tokio::test]
async fn analyze_contract_with_bytecode_runs_pipeline() {
    let chain_client = MockChainClient::contract("0x6080604052");

    // Provide mock responses for the source code fetch and external info
    // The source fetch will fail (404) which is fine — analysis still completes
    let http_client = MockHttpClient::new(vec![
        // Source code fetch attempt
        http::Response {
            status_code: 404,
            headers: std::collections::HashMap::new(),
            body: r#"{"status":"0","result":"Contract source code not verified"}"#.to_string(),
        },
        // External info fetch attempt (GitHub search)
        http::Response {
            status_code: 404,
            headers: std::collections::HashMap::new(),
            body: "{}".to_string(),
        },
    ]);

    let result = scope::contract::analyze_contract(
        "0x1234567890abcdef1234567890abcdef12345678",
        "ethereum",
        &chain_client,
        &http_client,
    )
    .await;

    // Should succeed even without verified source
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let analysis = result.unwrap();
    assert_eq!(
        analysis.address,
        "0x1234567890abcdef1234567890abcdef12345678"
    );
    assert_eq!(analysis.chain, "ethereum");
    assert!(!analysis.is_verified); // Source fetch returned 404
    assert!(analysis.security_score <= 100);

    // HttpClient should have been called at least once
    let urls = http_client.urls_called();
    assert!(!urls.is_empty(), "HttpClient should have been called");
}
