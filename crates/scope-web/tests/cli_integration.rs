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
        .stdout(predicate::str::contains("address-book"))
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("insights"));
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
fn test_address_book_help() {
    scope_cmd()
        .args(["address-book", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Address book management"))
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
        .stdout(predicate::str::contains("Address book management"));
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
fn test_config_option_address_book() {
    scope_cmd()
        .args(["--config", "/nonexistent/path.yaml", "address-book", "list"])
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
// Address Book Command Tests
// ============================================================================

#[test]
fn test_address_book_list_empty() {
    // Use a temp directory to avoid polluting real config
    let temp_dir = tempfile::tempdir().unwrap();

    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["address-book", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty").or(predicate::str::contains("Address book")));
}

#[test]
fn test_address_book_add_requires_address() {
    scope_cmd()
        .args(["address-book", "add"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_address_book_remove_requires_address() {
    scope_cmd()
        .args(["address-book", "remove"])
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
        .stderr(
            predicate::str::contains("--address").or(predicate::str::contains("--address-book")),
        );
}

// ============================================================================
// Setup Command Output Tests
// ============================================================================

#[test]
fn test_setup_status_output() {
    let temp_dir = tempfile::tempdir().unwrap();
    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["setup", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scope Configuration Status"))
        .stdout(predicate::str::contains("Config file"))
        .stdout(predicate::str::contains("API Keys"));
}

// ============================================================================
// Subcommand Help Output Tests (coverage for discover, market, token-health, etc.)
// ============================================================================

#[test]
fn test_discover_help() {
    scope_cmd()
        .args(["discover", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Discover").or(predicate::str::contains("discover")))
        .stdout(predicate::str::contains("--chain").or(predicate::str::contains("--source")));
}

#[test]
fn test_market_help() {
    scope_cmd()
        .args(["market", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("market"))
        .stdout(predicate::str::contains("summary"));
}

#[test]
fn test_token_health_help() {
    scope_cmd()
        .args(["token-health", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("token"))
        .stdout(predicate::str::contains("--chain"))
        .stdout(predicate::str::contains("--with-market"));
}

#[test]
fn test_crawl_help() {
    scope_cmd()
        .args(["crawl", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Crawl").or(predicate::str::contains("crawl")))
        .stdout(predicate::str::contains("--chain"));
}

#[test]
fn test_report_help() {
    scope_cmd()
        .args(["report", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("report"))
        .stdout(predicate::str::contains("batch"));
}

#[test]
fn test_compliance_help() {
    scope_cmd()
        .args(["compliance", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("risk"))
        .stdout(predicate::str::contains("trace"))
        .stdout(predicate::str::contains("analyze"));
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

// ============================================================================
// Global --ai Flag Tests (markdown output for agent consumption)
// ============================================================================

#[test]
fn test_ai_flag_accepted() {
    // --ai should be accepted by the CLI; address-book list emits to stdout
    let temp_dir = tempfile::tempdir().unwrap();
    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["--ai", "address-book", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Address book")
                .or(predicate::str::contains("address book"))
                .or(predicate::str::contains("empty")),
        );
}

// ============================================================================
// Insights Command Tests
// ============================================================================

#[test]
fn test_insights_help() {
    scope_cmd()
        .args(["insights", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("insights").or(predicate::str::contains("insight")))
        .stdout(predicate::str::contains("target"))
        .stdout(predicate::str::contains("chain"));
}

#[test]
fn test_insights_requires_target() {
    scope_cmd()
        .arg("insights")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_insight_alias() {
    scope_cmd()
        .args(["insight", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("target"));
}

// ============================================================================
// Contract Analysis Tests
// ============================================================================

#[test]
fn test_contract_help() {
    scope_cmd()
        .args(["contract", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("contract"))
        .stdout(predicate::str::contains("address"))
        .stdout(predicate::str::contains("chain"));
}

#[test]
fn test_contract_alias() {
    scope_cmd()
        .args(["ct", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("address"));
}

#[test]
fn test_contract_requires_address() {
    scope_cmd()
        .arg("contract")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

// ============================================================================
// Help Display & Typo Suggestion Tests
// ============================================================================

#[test]
fn test_help_shows_examples() {
    scope_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("scope address"))
        .stdout(predicate::str::contains("Documentation:"));
}

#[test]
fn test_address_help_shows_examples() {
    scope_cmd()
        .args(["address", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("scope address 0x742d"));
}

#[test]
fn test_typo_suggestion() {
    scope_cmd()
        .arg("adress")
        .assert()
        .failure()
        .stderr(predicate::str::contains("similar"));
}

// ============================================================================
// Shell Completions Tests
// ============================================================================

#[test]
fn test_completions_help() {
    scope_cmd()
        .args(["completions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shell"))
        .stdout(predicate::str::contains("completions"));
}

#[test]
fn test_completions_bash() {
    scope_cmd()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scope"));
}

#[test]
fn test_completions_zsh() {
    scope_cmd()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("compdef"));
}

#[test]
fn test_completions_fish() {
    scope_cmd()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

// ============================================================================
// Global --ai Flag Tests (markdown output for agent consumption)
// ============================================================================

#[test]
fn test_ai_flag_with_setup_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["--ai", "setup", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scope Configuration"));
}

// ============================================================================
// Web Command Tests
// ============================================================================

#[test]
fn test_web_help() {
    scope_cmd()
        .args(["web", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("web"))
        .stdout(predicate::str::contains("--port"))
        .stdout(predicate::str::contains("--daemon"))
        .stdout(predicate::str::contains("--stop"));
}

#[test]
fn test_serve_alias() {
    scope_cmd()
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("web").or(predicate::str::contains("serve")))
        .stdout(predicate::str::contains("--port"));
}

#[test]
fn test_web_stop_no_daemon() {
    let temp_dir = tempfile::tempdir().unwrap();
    scope_cmd()
        .env("HOME", temp_dir.path())
        .args(["web", "--stop"])
        .assert()
        .success();
}

#[test]
fn test_web_default_port() {
    // Verify help shows default port
    scope_cmd()
        .args(["web", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("8080"));
}

#[test]
fn test_web_default_bind() {
    scope_cmd()
        .args(["web", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1"));
}

// ============================================================================
// Report Command Tests
// ============================================================================

#[test]
fn test_report_batch_help() {
    scope_cmd()
        .args(["report", "batch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("batch"));
}

// ============================================================================
// Compliance Subcommand Tests
// ============================================================================

#[test]
fn test_compliance_risk_help() {
    scope_cmd()
        .args(["compliance", "risk", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("risk"))
        .stdout(predicate::str::contains("address"));
}

#[test]
fn test_compliance_trace_help() {
    scope_cmd()
        .args(["compliance", "trace", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trace"));
}

#[test]
fn test_compliance_analyze_help() {
    scope_cmd()
        .args(["compliance", "analyze", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("analyze"));
}

// ============================================================================
// Monitor Command Tests
// ============================================================================

#[test]
fn test_monitor_help() {
    scope_cmd()
        .args(["monitor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("monitor").or(predicate::str::contains("Monitor")))
        .stdout(predicate::str::contains("--chain"));
}

#[test]
fn test_mon_alias() {
    scope_cmd().args(["mon", "--help"]).assert().success();
}

// ============================================================================
// Additional Alias Tests
// ============================================================================

#[test]
fn test_disc_alias() {
    scope_cmd()
        .args(["disc", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("discover").or(predicate::str::contains("Discover")));
}

#[test]
fn test_health_alias() {
    scope_cmd().args(["health", "--help"]).assert().success();
}

#[test]
fn test_config_alias() {
    scope_cmd()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("setup")
                .or(predicate::str::contains("Setup"))
                .or(predicate::str::contains("Configure")),
        );
}

// ============================================================================
// Version in Help Tests
// ============================================================================

#[test]
fn test_help_shows_version() {
    scope_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "v{}",
            env!("CARGO_PKG_VERSION")
        )));
}

// ============================================================================
// Interactive Command Tests
// ============================================================================

#[test]
fn test_interactive_help() {
    scope_cmd()
        .args(["interactive", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("interactive").or(predicate::str::contains("Interactive")),
        );
}

#[test]
fn test_shell_alias() {
    scope_cmd().args(["shell", "--help"]).assert().success();
}
