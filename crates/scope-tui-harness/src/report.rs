//! The run report (design 8.2).

use crate::scenario::Tier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result3 {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub name: String,
    pub tier: Tier,
    pub result: Result3,
    pub ms: u128,
}

/// True when no offline scenario failed. Live results never fail the run.
pub fn gate(lines: &[Line]) -> bool {
    !lines
        .iter()
        .any(|l| l.tier == Tier::Offline && l.result == Result3::Fail)
}

/// Prints one line per scenario and a summary. Returns `gate(lines)`.
pub fn print(lines: &[Line]) -> bool {
    for l in lines {
        let r = match l.result {
            Result3::Pass => "PASS",
            Result3::Fail => "FAIL",
            Result3::Skip => "SKIP",
        };
        println!(
            "{r}  {:<7} {:<45} {} ms",
            format!("{:?}", l.tier).to_lowercase(),
            l.name,
            l.ms
        );
    }
    let count = |t, r| {
        lines
            .iter()
            .filter(|l| l.tier == t && l.result == r)
            .count()
    };
    println!(
        "\noffline: {} pass, {} fail   live: {} pass, {} fail (not gating)",
        count(Tier::Offline, Result3::Pass),
        count(Tier::Offline, Result3::Fail),
        count(Tier::Live, Result3::Pass),
        count(Tier::Live, Result3::Fail)
    );
    gate(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_failure_does_not_fail_the_run() {
        let lines = vec![
            Line {
                name: "a".into(),
                tier: Tier::Offline,
                result: Result3::Pass,
                ms: 10,
            },
            Line {
                name: "b".into(),
                tier: Tier::Live,
                result: Result3::Fail,
                ms: 10,
            },
        ];
        assert!(gate(&lines));
    }

    #[test]
    fn test_offline_failure_fails_the_run() {
        let lines = vec![Line {
            name: "a".into(),
            tier: Tier::Offline,
            result: Result3::Fail,
            ms: 10,
        }];
        assert!(!gate(&lines));
    }
}
