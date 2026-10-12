//! A record transport installed before `init` was built from the default
//! config, so it could not scan for the configured secrets. `init` must
//! refuse it, even though record is the wanted mode too. Own binary: the
//! global starts empty here. The environment is injected.

use scope::config::Config;
use scope::http::shared;

#[test]
fn test_init_refuses_a_record_transport_built_before_the_config() {
    let fixtures = tempfile::tempdir().unwrap();
    let dir = fixtures.path().to_string_lossy().into_owned();
    let env = move |k: &str| match k {
        "SCOPE_HTTP_MODE" => Some("record".to_string()),
        "SCOPE_HTTP_FIXTURES" => Some(dir.clone()),
        _ => None,
    };
    let _early = shared::transport_with(&env);
    assert_eq!(shared::describe(), "record");
    let err = shared::init_with(&Config::default(), &env)
        .err()
        .expect("init must refuse the early record transport");
    let msg = err.to_string();
    assert!(
        msg.contains("record mode started before the config was loaded"),
        "{msg}"
    );
}
