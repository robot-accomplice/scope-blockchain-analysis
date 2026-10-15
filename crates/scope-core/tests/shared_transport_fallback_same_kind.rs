//! Fallback to native is not a mismatch when config also selects native,
//! so `init` must stay silent. Own binary: the global starts empty here.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_after_fallback_is_silent_when_config_selects_native() {
    let _early = shared::transport();
    let built = shared::init(&Config::default()).unwrap();
    assert_eq!(built.kind, shared::Kind::Native);
    assert!(built.notices.is_empty(), "{:?}", built.notices);
}
