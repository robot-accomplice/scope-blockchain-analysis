//! The shared transport is process-global, so these checks run in their
//! own test binary.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_installs_once_and_transport_returns_it() {
    assert_eq!(shared::describe(), "none");
    let first = shared::init(&Config::default()).unwrap();
    assert_eq!(shared::describe(), "native");
    // A second init does not rebuild and reports no notices.
    let mut cfg = Config::default();
    cfg.ghola.enabled = true;
    let second = shared::init(&cfg).unwrap();
    assert!(second.notices.is_empty());
    assert!(std::sync::Arc::ptr_eq(&first.client, &second.client));
    assert!(std::sync::Arc::ptr_eq(&first.client, &shared::transport()));
}
