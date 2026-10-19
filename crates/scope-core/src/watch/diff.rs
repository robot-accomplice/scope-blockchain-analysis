//! Events between two snapshots: a breach or a warning that starts or ends.

use super::{RuleStatus, Snapshot};

/// A change in one cell between two ticks.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A cell failed; it did not fail on the tick before.
    BreachStarted {
        target: String,
        cell: String,
        message: String,
    },
    /// A cell that failed passes (or is no longer a failure).
    BreachCleared { target: String, cell: String },
    /// A cell warned; it did not warn on the tick before.
    WarnStarted {
        target: String,
        cell: String,
        message: String,
    },
    /// A cell that warned no longer warns.
    WarnCleared { target: String, cell: String },
}

impl Event {
    /// The banner text, for example `USDC@binance: spread_pct 3.4, needs <= 3`.
    pub fn banner(&self) -> String {
        match self {
            Event::BreachStarted {
                target, message, ..
            }
            | Event::WarnStarted {
                target, message, ..
            } => format!("{target}: {message}"),
            Event::BreachCleared { target, cell } => format!("{target}: {cell} passes again"),
            Event::WarnCleared { target, cell } => format!("{target}: {cell} no longer warns"),
        }
    }
}

/// The events from `prev` to `cur`. Cells match by target and cell label;
/// a target or cell new in `cur` compares with nothing (the first tick
/// compares with an empty snapshot).
pub fn diff(prev: &Snapshot, cur: &Snapshot) -> Vec<Event> {
    let before = |target: &str, cell: &str| {
        prev.targets
            .iter()
            .find(|t| t.target == target)
            .and_then(|t| t.cells.iter().find(|c| c.label == cell))
            .map(|c| &c.status)
    };
    let mut events = Vec::new();
    for t in &cur.targets {
        for c in &t.cells {
            let old = before(&t.target, &c.label);
            let was_fail = matches!(old, Some(RuleStatus::Fail(_)));
            let was_warn = matches!(old, Some(RuleStatus::Warn(_)));
            let target = t.target.clone();
            let cell = c.label.clone();
            match &c.status {
                RuleStatus::Fail(message) if !was_fail => events.push(Event::BreachStarted {
                    target,
                    cell,
                    message: message.clone(),
                }),
                RuleStatus::Warn(message) if !was_warn => events.push(Event::WarnStarted {
                    target,
                    cell,
                    message: message.clone(),
                }),
                RuleStatus::Fail(_) | RuleStatus::Warn(_) => {}
                // An error says nothing about the breach: health is unknown,
                // so the breach is not cleared.
                RuleStatus::Error(_) => {}
                RuleStatus::Pass | RuleStatus::NotApplicable(_) => {
                    if was_fail {
                        events.push(Event::BreachCleared { target, cell });
                    } else if was_warn {
                        events.push(Event::WarnCleared { target, cell });
                    }
                }
            }
            // A warning that became a failure ends as a warning too.
            if was_warn && matches!(c.status, RuleStatus::Fail(_)) {
                events.push(Event::WarnCleared {
                    target: t.target.clone(),
                    cell: c.label.clone(),
                });
            }
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::{Cell, TargetSnapshot};

    fn snap(status: RuleStatus) -> Snapshot {
        Snapshot {
            targets: vec![TargetSnapshot::new(
                "USDC@binance",
                vec![Cell {
                    label: "Spread".into(),
                    rule: None,
                    value: None,
                    status,
                }],
            )],
        }
    }

    fn fail() -> RuleStatus {
        RuleStatus::Fail("spread 3.4".into())
    }

    #[test]
    fn first_tick_compares_with_empty() {
        let e = diff(&Snapshot::default(), &snap(fail()));
        assert_eq!(
            e,
            vec![Event::BreachStarted {
                target: "USDC@binance".into(),
                cell: "Spread".into(),
                message: "spread 3.4".into()
            }]
        );
        assert_eq!(e[0].banner(), "USDC@binance: spread 3.4");
        assert!(diff(&Snapshot::default(), &snap(RuleStatus::Pass)).is_empty());
    }

    #[test]
    fn no_event_when_nothing_changes() {
        assert!(diff(&snap(fail()), &snap(fail())).is_empty());
        assert!(diff(&snap(RuleStatus::Pass), &snap(RuleStatus::Pass)).is_empty());
    }

    #[test]
    fn breach_clears_on_pass_and_on_na_but_not_on_error() {
        assert!(matches!(
            diff(&snap(fail()), &snap(RuleStatus::Pass))[..],
            [Event::BreachCleared { .. }]
        ));
        assert!(matches!(
            diff(&snap(fail()), &snap(RuleStatus::NotApplicable("n".into())))[..],
            [Event::BreachCleared { .. }]
        ));
        assert!(diff(&snap(fail()), &snap(RuleStatus::Error("e".into()))).is_empty());
    }

    #[test]
    fn warnings_start_clear_and_turn_into_breaches() {
        let warn = RuleStatus::Warn("ratio".into());
        assert!(matches!(
            diff(&snap(RuleStatus::Pass), &snap(warn.clone()))[..],
            [Event::WarnStarted { .. }]
        ));
        assert!(matches!(
            diff(&snap(warn.clone()), &snap(RuleStatus::Pass))[..],
            [Event::WarnCleared { .. }]
        ));
        assert!(matches!(
            diff(&snap(warn), &snap(fail()))[..],
            [Event::BreachStarted { .. }, Event::WarnCleared { .. }]
        ));
    }
}
