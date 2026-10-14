use super::*;

/// Forces a compile error when a role is added without a place in
/// [`Role::ALL`]: the match must name every variant.
fn listed_in_all(role: Role) -> bool {
    match role {
        Role::Accent
        | Role::Key
        | Role::Ok
        | Role::Warn
        | Role::Err
        | Role::Info
        | Role::Secondary
        | Role::Neutral
        | Role::Selected
        | Role::Match
        | Role::Link => Role::ALL.contains(&role),
    }
}

#[test]
fn every_role_is_listed_once() {
    for (i, role) in Role::ALL.iter().enumerate() {
        assert!(listed_in_all(*role));
        assert!(!Role::ALL[i + 1..].contains(role), "{role:?} listed twice");
    }
}

#[test]
fn every_role_but_neutral_looks_different_from_plain_text() {
    // A role carries meaning only if it is visible. Neutral is the plain
    // style on purpose.
    for role in Role::ALL {
        assert_eq!(tone(role) == Tone::PLAIN, role == Role::Neutral, "{role:?}");
    }
}

#[test]
fn tones_match_the_design_table() {
    // docs/design/2026-10-13-tui-color-theme.md, "Roles".
    let c = |color| Some(color);
    let table = [
        (Role::Accent, c(AnsiColor::Cyan), false, false, false, false),
        (Role::Key, c(AnsiColor::Magenta), true, false, false, false),
        (Role::Ok, c(AnsiColor::Green), false, false, false, false),
        (Role::Warn, c(AnsiColor::Yellow), false, false, false, false),
        (Role::Err, c(AnsiColor::Red), false, false, false, false),
        (Role::Info, c(AnsiColor::Blue), false, false, false, false),
        (Role::Secondary, None, false, true, false, false),
        (Role::Neutral, None, false, false, false, false),
        (
            Role::Selected,
            c(AnsiColor::Cyan),
            false,
            false,
            false,
            true,
        ),
        (Role::Match, c(AnsiColor::Yellow), false, false, false, true),
        (Role::Link, None, false, false, true, false),
    ];
    assert_eq!(table.len(), Role::ALL.len());
    for (role, color, bold, dim, underline, reverse) in table {
        let want = Tone {
            color,
            bold,
            dim,
            underline,
            reverse,
        };
        assert_eq!(tone(role), want, "{role:?}");
    }
}

#[test]
fn strong_adds_bold_and_keeps_the_rest() {
    let strong = tone(Role::Warn).strong();
    assert!(strong.bold);
    assert_eq!(strong.color, tone(Role::Warn).color);
}

#[test]
fn schemes_color_up_and_down_and_leave_flat_plain() {
    let up_down = |s| {
        (
            scheme_tone(s, Direction::Up),
            scheme_tone(s, Direction::Down),
        )
    };
    let colored = |c| Tone {
        color: Some(c),
        ..Tone::PLAIN
    };
    assert_eq!(
        up_down(Scheme::GreenRed),
        (colored(AnsiColor::Green), colored(AnsiColor::Red))
    );
    // Blue/orange is for red-green color blindness: down must not be red.
    assert_eq!(
        up_down(Scheme::BlueOrange),
        (colored(AnsiColor::Blue), colored(AnsiColor::Yellow))
    );
    assert_eq!(up_down(Scheme::Monochrome), (Tone::PLAIN, Tone::PLAIN));
    for scheme in [Scheme::GreenRed, Scheme::BlueOrange, Scheme::Monochrome] {
        assert_eq!(scheme_tone(scheme, Direction::Flat), Tone::PLAIN);
    }
}

#[test]
fn monochrome_keeps_a_roles_effects_but_not_its_color() {
    for role in Role::ALL {
        let mono = scheme_role_tone(Scheme::Monochrome, role);
        assert_eq!(mono.color, None, "{role:?}");
        assert_eq!(
            mono,
            Tone {
                color: None,
                ..tone(role)
            },
            "{role:?}"
        );
        assert_eq!(
            scheme_role_tone(Scheme::GreenRed, role),
            tone(role),
            "{role:?}"
        );
        assert_eq!(
            scheme_role_tone(Scheme::BlueOrange, role),
            tone(role),
            "{role:?}"
        );
    }
}
