//! When `SCOPE_HTTP_MODE` selects replay but the transport cannot be built,
//! a client used before `init` must panic, not go to the network. Own
//! binary: the global starts empty here. The environment is injected.

use scope::http::shared;

#[test]
fn test_fallback_panics_when_the_selected_transport_cannot_be_built() {
    // SCOPE_HTTP_FIXTURES is missing, so replay cannot be built.
    let env = |k: &str| (k == "SCOPE_HTTP_MODE").then(|| "replay".to_string());
    let result = std::panic::catch_unwind(|| shared::transport_with(&env));
    let payload = result.err().expect("transport_with must panic");
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(msg.contains("SCOPE_HTTP_FIXTURES"), "{msg}");
    // Nothing was installed in place of replay.
    assert_eq!(shared::describe(), "none");
}
