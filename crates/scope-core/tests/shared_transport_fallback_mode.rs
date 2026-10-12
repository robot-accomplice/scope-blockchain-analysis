//! A client used before `init` must get the transport `SCOPE_HTTP_MODE`
//! selects, not native: replay never uses the network (TUI test harness
//! design, section 3.1). Own binary: the global starts empty here. The
//! environment is injected, so the test sets no process variables.

use scope::config::Config;
use scope::http::{Request, shared};

#[tokio::test]
async fn test_fallback_before_init_installs_replay_from_the_environment() {
    let fixtures = tempfile::tempdir().unwrap();
    let dir = fixtures.path().to_string_lossy().into_owned();
    let env = move |k: &str| match k {
        "SCOPE_HTTP_MODE" => Some("replay".to_string()),
        "SCOPE_HTTP_FIXTURES" => Some(dir.clone()),
        _ => None,
    };
    let early = shared::transport_with(&env);
    assert_eq!(shared::describe(), "replay");
    // An empty fixture dir: the request is a replay miss, not a network call.
    let err = early
        .send(Request::get("http://127.0.0.1:9/never"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no fixture"), "{err}");
    // init with the same environment keeps it and has nothing to report.
    let built = shared::init_with(&Config::default(), &env).unwrap();
    assert_eq!(built.kind, shared::Kind::Replay);
    assert!(built.notices.is_empty(), "{:?}", built.notices);
}
