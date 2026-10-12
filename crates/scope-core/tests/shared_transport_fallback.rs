//! Review Focus 1: a client used before `init` gets native, and `init`
//! then says that the configured transport was not applied, once.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_after_fallback_reports_that_config_was_ignored() {
    let _early = shared::transport();
    assert_eq!(shared::describe(), "native");
    let mut cfg = Config::default();
    cfg.ghola.enabled = true;
    let built = shared::init(&cfg).unwrap();
    assert_eq!(built.kind, shared::Kind::Native);
    assert_eq!(shared::describe(), "native");
    assert!(
        built.notices.iter().any(|n| n.contains("not applied")),
        "{:?}",
        built.notices
    );
    // The notice is reported once.
    let again = shared::init(&cfg).unwrap();
    assert!(again.notices.is_empty(), "{:?}", again.notices);
}
