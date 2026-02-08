//! # CLI Integration Tests
//!
//! End-to-end tests for the Scope command-line interface.
//! These tests verify that the CLI binary works correctly
//! for various commands and argument combinations.

use assert_cmd::Command;
use predicates::prelude::*;

/// Returns a Command for the Scope binary.
#[allow(deprecated)] // TODO: Migrate to cargo::cargo_bin_cmd! when stable
fn scope_cmd() -> Command {
    Command::cargo_bin("scope").unwrap()
}

// ============================================================================
// Help and Version Tests
// ============================================================================

#[test]
fn test_help_output() {
    scope_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Scope Blockchain Analysis"))
        .stdout(predicate::str::contains("address"))
        .stdout(predicate::str::contains("tx"))
        .stdout(predicate::str::contains("portfolio"))
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("setup"));
}

#[test]
fn test_version_output() {
    scope_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_short_help() {
    scope_cmd()
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
    scope_cmd()
        .args(["address", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a blockchain address"))
        .stdout(predicate::str::contains("--chain"));
}

#[test]
fn test_tx_help() {
    scope_cmd()
        .args(["tx", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a transaction"))
        .stdout(predicate::str::contains("--trace"));
}

#[test]
fn test_portfolio_help() {
    scope_cmd()
        .args(["portfolio", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Portfolio management"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("list"));
}

#[test]
fn test_export_help() {
    scope_cmd()
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
    scope_cmd()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn test_invalid_subcommand() {
    scope_cmd()
        .arg("invalid")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn test_address_missing_address_arg() {
    scope_cmd()
        .arg("address")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_tx_missing_hash_arg() {
    scope_cmd()
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
    scope_cmd()
        .args(["addr", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a blockchain address"));
}

#[test]
fn test_transaction_alias() {
    scope_cmd()
        .args(["transaction", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyze a transaction"));
}

#[test]
fn test_port_alias() {
    scope_cmd()
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
    scope_cmd()
        .args(["-v", "address", "--help"])
        .assert()
        .success();
}

#[test]
fn test_multiple_verbose_flags() {
    scope_cmd()
        .args(["-vvv", "tx", "--help"])
        .assert()
        .success();
}

#[test]
fn test_config_option() {
    scope_cmd()
        .args(["--config", "/nonexistent/path.yaml", "portfolio", "list"])
        .assert()
        .success(); // Should succeed with defaults when config not found
}

#[test]
fn test_no_color_option() {
    scope_cmd()
        .args(["--no-color", "address", "--help"])
        .assert()
        .success();
}

// ============================================================================
// Address Command Tests
// ============================================================================

#[test]
fn test_address_invalid_format() {
    scope_cmd()
        .args(["address", "not-an-address"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid address"));
}

#[test]
fn test_address_missing_prefix() {
    scope_cmd()
        .args(["address", "742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("0x"));
}

#[test]
fn test_address_unsupported_chain() {
    scope_cmd()
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
    scope_cmd()
        .args(["tx", "not-a-hash"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid transaction hash"));
}

#[test]
fn test_tx_short_hash() {
    scope_cmd()
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

    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["portfolio", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty").or(predicate::str::contains("Portfolio")));
}

#[test]
fn test_portfolio_add_requires_address() {
    scope_cmd()
        .args(["portfolio", "add"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_portfolio_remove_requires_address() {
    scope_cmd()
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
    scope_cmd()
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

    scope_cmd()
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
    scope_cmd()
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
    scope_cmd()
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
