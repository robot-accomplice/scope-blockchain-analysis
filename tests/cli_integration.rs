//! # CLI Integration Tests
//!
//! End-to-end tests for the BCC command-line interface.
//! These tests verify that the CLI binary works correctly
//! for various commands and argument combinations.

use assert_cmd::Command;
use predicates::prelude::*;

/// Returns a Command for the BCC binary.
#[allow(deprecated)] // TODO: Migrate to cargo::cargo_bin_cmd! when stable
fn bcc() -> Command {
    Command::cargo_bin("bcc").unwrap()
}

// ============================================================================
// Help and Version Tests
// ============================================================================

#[test]
fn test_help_output() {
    bcc()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Blockchain Crawler CLI"))
        .stdout(predicate::str::contains("address"))
        .stdout(predicate::str::contains("tx"))
        .stdout(predicate::str::contains("portfolio"))
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("setup"));
}

#[test]
fn test_version_output() {
    bcc()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_short_help() {
    bcc()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

// ============================================================================
// Subcommand Help Tests
// ============================================================================

#[test]
fn test_address_help() {
    bcc()
        .args(["address", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a blockchain address"))
        .stdout(predicate::str::contains("--chain"));
}

#[test]
fn test_tx_help() {
    bcc()
        .args(["tx", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a transaction"))
        .stdout(predicate::str::contains("--trace"));
}

#[test]
fn test_portfolio_help() {
    bcc()
        .args(["portfolio", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Portfolio management"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("list"));
}

#[test]
fn test_export_help() {
    bcc()
        .args(["export", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Export analysis data"))
        .stdout(predicate::str::contains("--output"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_no_subcommand_shows_help() {
    bcc()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn test_invalid_subcommand() {
    bcc()
        .arg("invalid")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn test_address_missing_address_arg() {
    bcc()
        .arg("address")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_tx_missing_hash_arg() {
    bcc()
        .arg("tx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

// ============================================================================
// Command Alias Tests
// ============================================================================

#[test]
fn test_addr_alias() {
    bcc()
        .args(["addr", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a blockchain address"));
}

#[test]
fn test_transaction_alias() {
    bcc()
        .args(["transaction", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a transaction"));
}

#[test]
fn test_port_alias() {
    bcc()
        .args(["port", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Portfolio management"));
}

// ============================================================================
// Global Option Tests
// ============================================================================

#[test]
fn test_verbose_flag() {
    // Verbose flag should be accepted before subcommand
    bcc().args(["-v", "address", "--help"]).assert().success();
}

#[test]
fn test_multiple_verbose_flags() {
    bcc().args(["-vvv", "tx", "--help"]).assert().success();
}

#[test]
fn test_config_option() {
    bcc()
        .args(["--config", "/nonexistent/path.yaml", "portfolio", "list"])
        .assert()
        .success(); // Should succeed with defaults when config not found
}

#[test]
fn test_no_color_option() {
    bcc()
        .args(["--no-color", "address", "--help"])
        .assert()
        .success();
}

// ============================================================================
// Address Command Tests
// ============================================================================

#[test]
fn test_address_invalid_format() {
    bcc()
        .args(["address", "not-an-address"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid address"));
}

#[test]
fn test_address_missing_prefix() {
    bcc()
        .args(["address", "742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("0x"));
}

#[test]
fn test_address_unsupported_chain() {
    bcc()
        .args([
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--chain",
            "bitcoin",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unsupported chain"));
}

// ============================================================================
// Transaction Command Tests
// ============================================================================

#[test]
fn test_tx_invalid_hash() {
    bcc()
        .args(["tx", "not-a-hash"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid transaction hash"));
}

#[test]
fn test_tx_short_hash() {
    bcc()
        .args(["tx", "0xabc123"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("66 characters"));
}

// ============================================================================
// Portfolio Command Tests
// ============================================================================

#[test]
fn test_portfolio_list_empty() {
    // Use a temp directory to avoid polluting real config
    let temp_dir = tempfile::tempdir().unwrap();

    bcc()
        .env("HOME", temp_dir.path())
        .args(["portfolio", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty").or(predicate::str::contains("Portfolio")));
}

#[test]
fn test_portfolio_add_requires_address() {
    bcc()
        .args(["portfolio", "add"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_portfolio_remove_requires_address() {
    bcc()
        .args(["portfolio", "remove"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

// ============================================================================
// Export Command Tests
// ============================================================================

#[test]
fn test_export_requires_output() {
    bcc()
        .args([
            "export",
            "--address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--output"));
}

#[test]
fn test_export_requires_source() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join("output.json");

    bcc()
        .args(["export", "--output", output.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--address").or(predicate::str::contains("--portfolio")));
}

// ============================================================================
// Output Format Tests
// ============================================================================

#[test]
fn test_address_json_format_option() {
    bcc()
        .args([
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--format",
            "json",
            "--help",
        ])
        .assert()
        .success();
}

#[test]
fn test_address_csv_format_option() {
    bcc()
        .args([
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--format",
            "csv",
            "--help",
        ])
        .assert()
        .success();
}
