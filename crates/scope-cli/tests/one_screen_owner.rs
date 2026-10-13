//! One module owns the full screen: `cli/screen.rs`. It alone turns raw
//! mode, the alternate screen and mouse capture on and off, and sets the
//! restore hook, once. The crash log sets one hook of its own.
//! `ratatui::init` is banned: it adds a panic hook on every call, so a
//! resume per suspend would stack them.

mod common;

use common::{Allowed, audit, cli_dir, rs_sources};

const WATCHED: &[&str] = &[
    "ratatui::init",
    "ratatui::try_init",
    "ratatui::restore",
    "ratatui::try_restore",
    "enable_raw_mode",
    "disable_raw_mode",
    "EnterAlternateScreen",
    "LeaveAlternateScreen",
    "EnableMouseCapture",
    "DisableMouseCapture",
    "panic::set_hook",
];

const fn screen(call: &'static str, reason: &'static str) -> Allowed {
    Allowed {
        file: "screen.rs",
        call,
        count: 1,
        reason,
    }
}

const ALLOWED: &[Allowed] = &[
    screen("enable_raw_mode", "enter"),
    screen("EnterAlternateScreen", "enter"),
    screen("EnableMouseCapture", "enter, for the monitor"),
    screen("DisableMouseCapture", "restore"),
    screen(
        "ratatui::restore",
        "restore: raw mode and alternate screen off",
    ),
    screen("panic::set_hook", "the restore hook, run once"),
    Allowed {
        file: "tui/crash.rs",
        call: "panic::set_hook",
        count: 1,
        reason: "the crash-log hook, installed once",
    },
];

#[test]
fn only_the_screen_module_takes_the_screen() {
    let found = audit(&rs_sources(&cli_dir()), WATCHED, ALLOWED);
    assert!(
        found.is_empty(),
        "take and give back the screen with cli::screen:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_second_hook_or_a_ratatui_init_is_flagged() {
    let src = [
        "std::panic::set_hook(a);",
        "std::panic::set_hook(b);",
        "let t = ratatui::init();",
    ]
    .join("\n");
    let found = audit(&[("screen.rs".into(), src)], WATCHED, ALLOWED);
    assert!(
        found.contains(&"screen.rs: ratatui::init 1 times, allowed 0 [screen.rs:3]".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(
            &"screen.rs: panic::set_hook 2 times, allowed 1 (the restore hook, run once) \
              [screen.rs:1, screen.rs:2]"
                .to_string()
        ),
        "{found:?}"
    );
}
