//! The validator as teacher (AGENT-011 §5, P4).
//!
//! A rejection that says *what is wrong* costs a round trip. A rejection that
//! says *what to write instead* costs none, and on a small tier that difference
//! is most of the budget. So every error carries a stable `code`, a `fix` (a
//! concrete edit) and a `see` (a guide anchor).
//!
//! # Why the fixes live in a table rather than at the throw site
//!
//! Two reasons. The first is that a fix is documentation and drifts like
//! documentation: one place to read is one place to keep true, and
//! `every_code_has_a_fix_and_an_anchor` fails the build when a code appears
//! without one. The second is the escalation in [`Teacher`]: the third rejection
//! on the same `path` within a run returns a worked example instead of a third
//! restatement, and that decision needs to see the history, not the throw site.
//!
//! The escalation is the cheapest understanding available on a small tier: a
//! model that has misread a rule twice does not need the rule a third time, it
//! needs a correct instance to copy.

use std::collections::HashMap;

/// `(code, fix, see, worked example)` for every refusal the language can issue.
///
/// The example is what the third rejection on one path returns; `None` where a
/// single line cannot demonstrate the fix.
#[rustfmt::skip]
const TEACHING: &[(&str, &str, &str, Option<&str>)] = &[
    ("unknown_feature",
     "name a catalogued family — `list_features(query=\"…\")` shows them with their parameters",
     "guide#features.catalogue",
     Some("entry long when rsi(close, 14) < 30")),
    ("feature_needs_parameters",
     "give the family its parameters: `ema(close, 21)`, not `ema`",
     "guide#features.catalogue",
     Some("input trend = ema(close, 50)")),
    ("parameter_out_of_range",
     "choose a parameter inside the family's declared range; the range is part of the name's meaning",
     "guide#features.catalogue",
     Some("entry long when rsi(close, 14) < 30")),
    ("source_not_close",
     "every catalogued family is defined on the close; use the named high/low families instead",
     "guide#features.catalogue",
     Some("entry long when close > donchian_upper(close, 55)")),
    ("distance_without_unit",
     "write `1.5 %` — the unit of an exit distance is never implied",
     "guide#exits.units",
     Some("exit stop 1.5 %")),
    ("unit_not_representable",
     "write the distance in percent; a 1.3 exit rule stores a fraction of entry and nothing else",
     "guide#exits.units",
     Some("exit stop 2 %")),
    ("unknown_unit",
     "the unit of an exit distance is `%`",
     "guide#exits.units",
     Some("exit target 4 %")),
    ("exit_distance_not_positive",
     "a distance must be inside (0, 100) percent, and a time exit at least one bar",
     "guide#exits.units",
     Some("exit time 48 bars")),
    ("missing_exits",
     "add a real exit, or say `exit risk_layer_only` — silence and delegation are different claims",
     "guide#exits.required",
     Some("exit risk_layer_only")),
    ("exits_contradict",
     "`exit risk_layer_only` means there are no exit rules; delete it or delete the others",
     "guide#exits.required",
     None),
    ("expression_too_long",
     "bind part of the expression as an input, then reference the input",
     "guide#inputs.binding",
     Some("input trend = ema(close, 50)")),
    ("when_names_expression",
     "a `when` names a condition node; bind the expression as an input and reference it",
     "guide#inputs.binding",
     None),
    ("state_predicate_in_condition",
     "position state belongs in an `exit` clause, which the runtime evaluates against the position",
     "guide#exits.state",
     Some("exit time 48 bars")),
    ("function_not_representable",
     "rewrite without it, or compute it as a catalogued feature",
     "guide#grammar.functions",
     Some("entry long when between(rsi(close, 14), 40, 60)")),
    ("crossing_under_or",
     "write one `entry` clause per alternative so each crossing owns its own rising edge",
     "guide#grammar.crossings",
     Some("entry long when crossed_above(ema(close, 7), ema(close, 21))")),
    ("crossing_under_not",
     "`not crossed_above(…)` is not a downward crossing; write `crossed_below(…)`",
     "guide#grammar.crossings",
     Some("entry long when crossed_below(ema(close, 7), ema(close, 21))")),
    ("rising_needs_close",
     "`rising`/`falling` are defined on the close only",
     "guide#grammar.crossings",
     Some("entry long when rising(close, 5)")),
    ("unsupported_size",
     "v2.0 sizes a position as `size fixed <quantity>`",
     "guide#size",
     Some("size fixed 0.01")),
    ("unknown_param",
     "declare the parameter before referencing it",
     "guide#params",
     Some("param fast int 7 range 2 50")),
    ("param_range_inverted", "write the low bound first", "guide#params", Some("param fast int 7 range 2 50")),
    ("param_default_outside_range", "move the default inside the range, or widen the range", "guide#params", Some("param fast int 7 range 2 50")),
    ("malformed_param", "write `param <name> int|number <default> range <lo> <hi>`", "guide#params", Some("param fast int 7 range 2 50")),
    ("malformed_input", "write `input <name> = <expression>`", "guide#inputs.binding", Some("input trend = ema(close, 50)")),
    ("duplicate_input", "give the second binding a different name", "guide#inputs.binding", None),
    ("malformed_entry", "write `entry long when <condition>`", "guide#entries", Some("entry long when rsi(close, 14) < 30")),
    ("malformed_exit", "check the exit forms in the guide", "guide#exits.units", Some("exit stop 2 %")),
    ("malformed_risk", "write `risk max_position 0.5`", "guide#risk", Some("risk max_position 0.5")),
    ("malformed_clause", "each clause takes exactly the words the guide lists", "guide#grammar.clauses", None),
    ("malformed_call", "check the brackets and commas", "guide#grammar.calls", Some("entry long when ema(close, 7) > ema(close, 21)")),
    ("malformed_expression", "a condition compares two values", "guide#grammar.expressions", Some("entry long when rsi(close, 14) < 30")),
    ("unknown_clause", "every line starts with a clause keyword, or `#` for a comment", "guide#grammar.clauses", None),
    ("missing_clause", "add the missing clause", "guide#grammar.clauses", None),
    ("dangling_comment", "a comment describes the clause below it", "guide#comments", None),
    ("empty_expression", "write a condition", "guide#grammar.expressions", Some("entry long when rsi(close, 14) < 30")),
    ("trailing_tokens", "check for a missing operator or an extra bracket", "guide#grammar.expressions", None),
    ("bad_operator", "equality is `==`; SLv2 has no assignment inside an expression", "guide#grammar.expressions", None),
    ("bad_character", "expressions use numbers, feature calls, bar fields, comparisons and `and`/`or`/`not`", "guide#grammar.expressions", None),
    ("warmup_exceeds_window",
     "start the window earlier, or shorten the longest lookback — `ema(close, 200)` needs 1,000 bars of lead-in and `ema(close, 100)` needs 500",
     "guide#features.warmup",
     None),
    ("loosening",
     "refused: risk limits are tighten-only against the platform gate",
     "guide#risk",
     None),
    ("never_fires",
     "loosen the threshold the blocking profile names, or check the sign of the comparison",
     "guide#preview",
     None),
    ("holdout_overlap",
     "move the window to end before the sealed holdout begins; a preview reads exactly what a backtest could read",
     "guide#preview.cutoff",
     None),
];

/// The concrete edit for an error code.
#[must_use]
pub fn fix_for(code: &str) -> Option<&'static str> {
    TEACHING.iter().find(|(c, ..)| *c == code).map(|(_, fix, ..)| *fix)
}

/// The guide anchor for an error code.
#[must_use]
pub fn see_for(code: &str) -> Option<&'static str> {
    TEACHING.iter().find(|(c, ..)| *c == code).map(|(_, _, see, _)| *see)
}

/// A worked example for an error code, where one line can show the fix.
#[must_use]
pub fn example_for(code: &str) -> Option<&'static str> {
    TEACHING.iter().find(|(c, ..)| *c == code).and_then(|(.., ex)| *ex)
}

/// Every code the table knows, for the conformance test and the guide generator.
#[must_use]
pub fn codes() -> Vec<&'static str> {
    TEACHING.iter().map(|(c, ..)| *c).collect()
}

/// How many rejections on one `path` before a worked example is added.
pub const ESCALATE_AFTER: u32 = 3;

/// Counts rejections per path within one run, so the third on a path escalates.
///
/// Per *run*, not per call: the point is that this model, on this task, has now
/// failed at this clause three times. A per-call counter would never reach three
/// and a global one would escalate for someone else's mistake.
#[derive(Debug, Default)]
pub struct Teacher {
    seen: HashMap<String, u32>,
}

impl Teacher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a rejection at `path` with `code`, and return the worked example
    /// if this is the [`ESCALATE_AFTER`]-th one there.
    pub fn record(&mut self, path: &str, code: &str) -> Option<&'static str> {
        let n = self.seen.entry(path.to_string()).or_default();
        *n += 1;
        if *n >= ESCALATE_AFTER {
            example_for(code)
        } else {
            None
        }
    }

    /// How many times `path` has been rejected in this run.
    #[must_use]
    pub fn count(&self, path: &str) -> u32 {
        self.seen.get(path).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_appears_once_and_carries_a_fix_and_an_anchor() {
        let mut seen = std::collections::BTreeSet::new();
        for (code, fix, see, _) in TEACHING {
            assert!(seen.insert(*code), "duplicate code `{code}`");
            assert!(!fix.is_empty(), "`{code}` has no fix");
            assert!(see.starts_with("guide#"), "`{code}` anchor `{see}` is not a guide anchor");
        }
    }

    /// The six cases AGENT-011 §5 names by hand. If the table loses one, the
    /// spec's acceptance criterion 4 stops holding without anything else failing.
    #[test]
    fn the_six_error_cases_the_spec_names_all_have_fixes() {
        for code in ["unknown_feature", "distance_without_unit", "when_names_expression", "loosening", "warmup_exceeds_window", "never_fires"] {
            assert!(fix_for(code).is_some(), "{code}");
            assert!(see_for(code).is_some(), "{code}");
        }
    }

    #[test]
    fn a_fix_says_what_to_write_rather_than_what_is_wrong() {
        // A fix that only restates the problem is the failure this table exists
        // to prevent. Every fix names an edit: a verb, or a literal to copy.
        for (code, fix, ..) in TEACHING {
            let f = fix.to_lowercase();
            let actionable = ["write", "add", "delete", "rewrite", "move", "choose", "give", "declare", "bind", "name", "call", "use", "check", "start", "loosen", "put", "refused"]
                .iter()
                .any(|v| f.contains(v));
            assert!(actionable, "`{code}` fix is not an instruction: {fix}");
        }
    }

    #[test]
    fn the_third_rejection_on_one_path_returns_a_worked_example() {
        let mut t = Teacher::new();
        assert_eq!(t.record("entry[0]", "unknown_feature"), None);
        assert_eq!(t.record("entry[0]", "unknown_feature"), None);
        assert_eq!(t.record("entry[0]", "unknown_feature"), Some("entry long when rsi(close, 14) < 30"));
        assert_eq!(t.count("entry[0]"), 3);
    }

    /// Escalation is per path. Three different clauses each failing once is a
    /// model making three ordinary mistakes, not one it cannot understand.
    #[test]
    fn rejections_on_different_paths_do_not_escalate_each_other() {
        let mut t = Teacher::new();
        assert_eq!(t.record("entry[0]", "unknown_feature"), None);
        assert_eq!(t.record("entry[1]", "unknown_feature"), None);
        assert_eq!(t.record("exits[0]", "distance_without_unit"), None);
        assert_eq!(t.count("entry[0]"), 1);
    }
}
