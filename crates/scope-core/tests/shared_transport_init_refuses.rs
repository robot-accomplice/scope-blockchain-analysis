//! Backstop: if native was installed before `init` and `SCOPE_HTTP_MODE`
//! then selects replay or record, `init` must fail, not keep native. Own
//! binary: the global starts empty here. The environment is injected.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_fails_when_mode_wants_replay_or_record_but_native_fell_back() {
    let _early = shared::transport_with(&|_| None);
    assert_eq!(shared::describe(), "native");
    let fixtures = tempfile::tempdir().unwrap();
    let dir = fixtures.path().to_string_lossy().into_owned();
    for mode in ["replay", "record"] {
        let env = |k: &str| match k {
            "SCOPE_HTTP_MODE" => Some(mode.to_string()),
            "SCOPE_HTTP_FIXTURES" => Some(dir.clone()),
            _ => None,
        };
        let err = shared::init_with(&Config::default(), &env).err().unwrap();
        let msg = err.to_string();
        assert!(msg.contains(&format!("SCOPE_HTTP_MODE={mode}")), "{msg}");
        assert!(!msg.contains("from config"), "{msg}");
    }
    // Native stays installed; a config-only init is still accepted.
    assert_eq!(shared::describe(), "native");
    assert!(shared::init_with(&Config::default(), &|_| None).is_ok());
}
