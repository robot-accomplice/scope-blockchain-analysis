//! Rules (`metric op value`) and their evaluation.

use crate::metrics::MetricId;
use std::fmt;

/// A comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Op {
    /// Every operator, longest symbol first (so `<=` parses before `<`).
    const BY_LENGTH: [Op; 6] = [Op::Le, Op::Ge, Op::Eq, Op::Ne, Op::Lt, Op::Gt];

    /// The operator's symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "==",
            Op::Ne => "!=",
        }
    }

    /// The operator with this symbol.
    pub fn from_symbol(s: &str) -> Option<Op> {
        Op::BY_LENGTH.iter().copied().find(|op| op.symbol() == s)
    }

    /// Whether `value op limit` holds. `==` and `!=` allow a relative
    /// difference of 1e-9, so a value read back from text still matches.
    pub fn holds(self, value: f64, limit: f64) -> bool {
        let equal = (value - limit).abs() <= 1e-9 * value.abs().max(limit.abs()).max(1.0);
        match self {
            Op::Lt => value < limit,
            Op::Le => value <= limit || equal,
            Op::Gt => value > limit,
            Op::Ge => value >= limit || equal,
            Op::Eq => equal,
            Op::Ne => !equal,
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A metric threshold: the target passes the rule when `metric op value`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub metric: MetricId,
    pub op: Op,
    pub value: f64,
}

impl Rule {
    /// Parses `<metric> <op> <value>`, for example `holders >= 1000`. The
    /// spaces are optional.
    pub fn parse(text: &str) -> Result<Rule, String> {
        let text = text.trim();
        let (at, op) = text
            .char_indices()
            .find_map(|(i, _)| {
                Op::BY_LENGTH
                    .iter()
                    .find(|op| text[i..].starts_with(op.symbol()))
                    .map(|op| (i, *op))
            })
            .ok_or_else(|| format!("`{text}` has no operator; use one of < <= > >= == !="))?;
        let name = text[..at].trim();
        let value_text = text[at + op.symbol().len()..].trim();
        let metric = MetricId::from_name(name).ok_or_else(|| {
            let names: Vec<&str> = MetricId::ALL.iter().map(|m| m.name()).collect();
            format!(
                "unknown metric `{name}`; the metrics are {}",
                names.join(", ")
            )
        })?;
        let value: f64 = value_text
            .replace('_', "")
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite())
            .ok_or_else(|| format!("`{value_text}` is not a number"))?;
        Ok(Rule { metric, op, value })
    }

    /// Judges `value` against the rule. No value is an error: health is
    /// unknown, never a pass.
    pub fn eval(&self, value: Option<f64>) -> RuleStatus {
        match value {
            None => RuleStatus::Error(format!("{}: no value", self.metric.name())),
            Some(v) if self.op.holds(v, self.value) => RuleStatus::Pass,
            Some(v) => RuleStatus::Fail(format!(
                "{} {}, needs {} {}",
                self.metric.name(),
                trim_number(v),
                self.op,
                trim_number(self.value)
            )),
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.metric.name(),
            self.op,
            trim_number(self.value)
        )
    }
}

/// A number as short as it reads back: `1000`, `0.5`, `3.4`.
fn trim_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The result of one rule or preset check on one tick.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleStatus {
    Pass,
    /// A preset check warned (P2).
    Warn(String),
    Fail(String),
    /// The check does not apply here, for example spread on an AMM book.
    NotApplicable(String),
    /// The value could not be read: health is unknown.
    Error(String),
}

impl RuleStatus {
    /// The status's short name: `pass`, `warn`, `fail`, `n/a`, `error`.
    pub fn name(&self) -> &'static str {
        match self {
            RuleStatus::Pass => "pass",
            RuleStatus::Warn(_) => "warn",
            RuleStatus::Fail(_) => "fail",
            RuleStatus::NotApplicable(_) => "n/a",
            RuleStatus::Error(_) => "error",
        }
    }

    /// The message, when the status has one.
    pub fn message(&self) -> Option<&str> {
        match self {
            RuleStatus::Pass => None,
            RuleStatus::Warn(m)
            | RuleStatus::Fail(m)
            | RuleStatus::NotApplicable(m)
            | RuleStatus::Error(m) => Some(m),
        }
    }
}

/// A target's state, in the order the grid sorts rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetStatus {
    /// At least one rule failed.
    Breached,
    /// No failure, no error, at least one warning.
    Warned,
    /// No failure, but at least one rule could not be read.
    Unknown,
    /// Every rule that applies passed.
    Passed,
}

impl TargetStatus {
    /// The status's name as the grid shows it.
    pub fn name(self) -> &'static str {
        match self {
            TargetStatus::Breached => "breached",
            TargetStatus::Warned => "warned",
            TargetStatus::Unknown => "unknown",
            TargetStatus::Passed => "passed",
        }
    }
}

/// A target's state from its rule statuses. `n/a` never counts against it.
pub fn target_status(statuses: &[RuleStatus]) -> TargetStatus {
    let any = |f: fn(&RuleStatus) -> bool| statuses.iter().any(f);
    if any(|s| matches!(s, RuleStatus::Fail(_))) {
        TargetStatus::Breached
    } else if any(|s| matches!(s, RuleStatus::Error(_))) {
        TargetStatus::Unknown
    } else if any(|s| matches!(s, RuleStatus::Warn(_))) {
        TargetStatus::Warned
    } else {
        TargetStatus::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_below_at_and_above_the_limit() {
        // (op, below, at, above)
        let table = [
            (Op::Lt, true, false, false),
            (Op::Le, true, true, false),
            (Op::Gt, false, false, true),
            (Op::Ge, false, true, true),
            (Op::Eq, false, true, false),
            (Op::Ne, true, false, true),
        ];
        for (op, below, at, above) in table {
            assert_eq!(op.holds(0.9, 1.0), below, "{op} below");
            assert_eq!(op.holds(1.0, 1.0), at, "{op} at");
            assert_eq!(op.holds(1.1, 1.0), above, "{op} above");
        }
        assert!(Op::Eq.holds(0.1 + 0.2, 0.3), "float noise is equal");
    }

    #[test]
    fn rules_parse_with_and_without_spaces_and_print_back() {
        for (text, metric, op, value) in [
            ("holders >= 1000", MetricId::Holders, Op::Ge, 1000.0),
            ("spread_pct<0.5", MetricId::SpreadPct, Op::Lt, 0.5),
            (
                "liquidity_usd > 1_000_000",
                MetricId::LiquidityUsd,
                Op::Gt,
                1e6,
            ),
            ("risk_score != 10", MetricId::RiskScore, Op::Ne, 10.0),
            ("mid_price == 1", MetricId::Mid, Op::Eq, 1.0),
            (
                "native_balance <= -1",
                MetricId::NativeBalance,
                Op::Le,
                -1.0,
            ),
        ] {
            let r = Rule::parse(text).unwrap();
            assert_eq!((r.metric, r.op, r.value), (metric, op, value), "{text}");
            assert_eq!(Rule::parse(&r.to_string()).unwrap(), r);
        }
        assert_eq!(
            Rule::parse("holders >= 1000").unwrap().to_string(),
            "holders >= 1000"
        );
    }

    #[test]
    fn bad_rules_say_why() {
        assert!(
            Rule::parse("holders 1000")
                .unwrap_err()
                .contains("no operator")
        );
        let e = Rule::parse("holdrs > 1").unwrap_err();
        assert!(
            e.contains("unknown metric `holdrs`") && e.contains("holders"),
            "{e}"
        );
        assert!(
            Rule::parse("holders > lots")
                .unwrap_err()
                .contains("not a number")
        );
        assert!(
            Rule::parse("holders > inf")
                .unwrap_err()
                .contains("not a number")
        );
    }

    #[test]
    fn eval_gives_pass_fail_and_error_for_no_value() {
        let r = Rule::parse("spread_pct <= 3").unwrap();
        assert_eq!(r.eval(Some(1.0)), RuleStatus::Pass);
        assert_eq!(
            r.eval(Some(3.4)),
            RuleStatus::Fail("spread_pct 3.4, needs <= 3".into())
        );
        assert_eq!(
            r.eval(None),
            RuleStatus::Error("spread_pct: no value".into())
        );
    }

    #[test]
    fn target_status_rules() {
        use RuleStatus::*;
        let s = |v: &[RuleStatus]| target_status(v);
        assert_eq!(
            s(&[Pass, Fail("f".into()), Error("e".into())]),
            TargetStatus::Breached
        );
        assert_eq!(
            s(&[Pass, Error("e".into()), Warn("w".into())]),
            TargetStatus::Unknown
        );
        assert_eq!(s(&[Pass, Warn("w".into())]), TargetStatus::Warned);
        assert_eq!(s(&[NotApplicable("n".into())]), TargetStatus::Passed);
        assert_eq!(s(&[]), TargetStatus::Passed);
        let mut order = vec![
            TargetStatus::Passed,
            TargetStatus::Unknown,
            TargetStatus::Breached,
            TargetStatus::Warned,
        ];
        order.sort();
        assert_eq!(
            order,
            [
                TargetStatus::Breached,
                TargetStatus::Warned,
                TargetStatus::Unknown,
                TargetStatus::Passed
            ]
        );
    }
}
