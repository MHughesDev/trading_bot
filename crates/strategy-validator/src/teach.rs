//! The validator teaches (AGENT-011 §5; Set S task S-5.3).
//!
//! # Why a rejection that only says "no" is expensive
//!
//! A small model that is told `rsi14 is not a feature` has three moves: guess
//! another spelling, give up on the feature, or give up on the strategy. It
//! cannot tell which, because the rejection describes the error and not the
//! edit. Every one of those costs a turn, and on a tight exposure budget a turn
//! spent re-guessing a name is a turn not spent on the task.
//!
//! So each code carries a `fix` — a concrete edit, in the caller's own terms —
//! and a `see` anchor into the authoring guide for the reader who wants the
//! rule behind it. The two are different audiences on purpose: the fix is for
//! the model that wants to proceed, the anchor for the person reading the
//! transcript afterwards wondering whether the rule is right.
//!
//! # The table is the only place a fix is written
//!
//! Not in the message. A fix embedded in a message is a fix no test can check
//! for and no renderer can present differently, and the first thing that happens
//! to it is a reword that drops the suggestion. Keyed by code, so
//! `every_code_the_validator_constructs_can_teach` can walk the whole table and
//! assert nothing rejects without saying what to do instead.

/// What to do about one error, and where the rule lives.
///
/// `see` is a guide anchor, not a URL: the guide is served by
/// `get_authoring_guide` and rendered in several places, so a link would be
/// right in one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Teaching {
    pub code: &'static str,
    pub fix: &'static str,
    pub see: Option<&'static str>,
}

/// Every code the validator can emit, with the edit that resolves it.
///
/// The first six are AGENT-011 §5's table verbatim in intent. The rest are the
/// codes this crate actually constructs — a code with no row here fails
/// `every_code_the_validator_constructs_can_teach`, which is the test that stops
/// this table from covering the spec's examples and nothing else.
pub const TEACHINGS: &[Teaching] = &[
    // ── AGENT-011 §5's six ───────────────────────────────────────────────────
    Teaching {
        code: "unknown_feature",
        fix: "call `list_features(query=\"…\")` and write the `example` form it gives you. A \
              family with parameters does not resolve from its bare name: `rsi` is not a \
              feature, `rsi_14` is.",
        see: Some("guide#features"),
    },
    Teaching {
        code: "distance_without_unit",
        fix: "give the distance a unit: `1.5 atr`, `1.5 %` or `1.5 sigma` from entry. A bare \
              number is ambiguous across instruments at different price scales, which is the \
              one thing a swept parameter must not be.",
        see: Some("guide#exits.units"),
    },
    Teaching {
        code: "when_names_expression",
        fix: "`when` names a node, not an expression. Add a `condition` node holding the \
              expression and put its `id` in `when`.",
        see: Some("guide#nodes"),
    },
    Teaching {
        code: "loosening",
        fix: "refused — the platform gate is tighten-only. A revision may narrow the risk \
              envelope and never widen it; if the wider envelope is the point, that is a new \
              strategy rather than an edit to this one.",
        see: Some("guide#risk"),
    },
    Teaching {
        code: "warmup_exceeds_window",
        fix: "start the window earlier, or shorten the longest lookback — `ema_200` to \
              `ema_100` halves the warm-up. `list_features` reports `warmup_bars` per family, \
              and `preview_signals` says how many bars actually got evaluated.",
        see: Some("guide#warmup"),
    },
    Teaching {
        code: "never_fires",
        fix: "loosen the threshold, or check the sign of the comparison. `preview_signals` \
              reports `never_fires_because`, which names which of the reasons it was rather \
              than leaving you to guess.",
        see: Some("guide#preview_signals"),
    },
    // ── the codes this crate constructs ──────────────────────────────────────
    Teaching {
        code: "signal_has_no_action",
        fix: "the conditions work and nothing is wired to trade them: add an action whose               `on_signal` names the signal your node emits. This is the only reason a preview               reports no entries whose fix is in `actions` rather than in `nodes`.",
        see: Some("guide#actions"),
    },
    Teaching {
        code: "unknown_definition_version",
        fix: "set `definition_version` to a version inside the supported range. 1.0 is the \
              base, 1.1 adds the `model_forecast` node, 1.2 adds typed `parameters`, and 1.3 \
              adds the `exits` block and 1.4 adds the `unit` on an exit distance — 1.4 is \
              the newest, and the compiler emits 1.3 unless a strategy uses a \
              volatility-scaled distance. \
              A version above the range is refused rather than read as the nearest one below.",
        see: Some("guide#definition"),
    },
    Teaching {
        code: "empty_strategy_id",
        fix: "give the strategy a slug: lowercase, underscores, and a version suffix — \
              `ema_cross_v1`. It is what evidence cites it by.",
        see: Some("guide#definition"),
    },
    Teaching {
        code: "duplicate_node_id",
        fix: "node ids are unique within a definition; rename one of the pair. `when` and \
              `on_signal` resolve by id, so a duplicate makes the reference ambiguous.",
        see: Some("guide#nodes"),
    },
    Teaching {
        code: "unknown_signal",
        fix: "`on_signal` must name a signal some node emits. Check the `emit` value on your \
              signal nodes — a mismatch here is why a strategy previews entries and takes \
              none.",
        see: Some("guide#actions"),
    },
    Teaching {
        code: "exits_required",
        fix: "a 1.3 definition declares what closes a position. Use `{\"mode\": \
              \"risk_layer_only\"}` if the risk layer really is the whole exit logic — that \
              is a legitimate choice and it has to be said out loud, because a strategy that \
              *forgot* exits looks identical from the outside.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "exits_empty_rules",
        fix: "a `rules` policy with no rules is `risk_layer_only` wearing a disguise. Add a \
              rule, or declare `risk_layer_only` — the two must not hash alike.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "exit_fraction_out_of_range",
        fix: "a stop, target or trailing distance is a positive fraction of entry — `0.02` is \
              2%. Never an absolute price: the rule has to survive a sweep across instruments \
              at different price scales.",
        see: Some("guide#exits.units"),
    },
    Teaching {
        code: "exit_distance_not_positive",
        fix: "a distance in `atr` or `sigma` is a positive multiple — `1.5 atr(14)` is one and a \
              half average true ranges. There is no upper bound, because a 5-ATR stop is wide \
              rather than wrong; zero or negative puts the stop on the wrong side of the entry.",
        see: Some("guide#exits.units"),
    },
    Teaching {
        code: "exit_unit_period_out_of_range",
        fix: "give the unit a lookback the feature engine computes: `atr(14)`, `sigma(20)`. The \
              period is what selects the `atr_14` / `rolling_std_20` feature, so `atr(0)` is not \
              a missing lookback — it is a feature name nothing serves.",
        see: Some("guide#exits.units"),
    },
    Teaching {
        code: "exit_unit_version_too_low",
        fix: "raise `definition_version` to 1.4, which is the version that has the `unit` field, \
              or write the distance as a fraction of entry. Do not hand-convert an ATR distance \
              into a fraction: the fraction is right on the bar you computed it and wrong on the \
              next one, and the two look identical in storage.",
        see: Some("guide#exits.units"),
    },
    Teaching {
        code: "exit_holding_bars_zero",
        fix: "`max_holding_bars` closes after that many bars, so it must be at least 1. Zero \
              would close the position on the bar it opened.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "exits_version_too_low",
        fix: "raise `definition_version` to 1.3, or remove the `exits` block. A runtime reading               the document at the declared version ignores the exits and trades the entries               anyway, which is the worst of the three outcomes.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "exit_rule_shadowed",
        fix: "delete one of the pair. Rules are evaluated in order and the first match wins, so               the looser of two rules of the same kind can never fire — keeping both hides               which one is live.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "exit_signal_name_empty",
        fix: "give `on_signal` the name a signal node emits. An empty name matches nothing, so               the position has no exit at all while appearing to have one.",
        see: Some("guide#exits"),
    },
    Teaching {
        code: "parameter_out_of_range",
        fix: "the sample violates the parameter's declared `min`/`max`. Either the sample is \
              wrong or the range is — and the range is what the sweep is checked against, so \
              widening it is a decision rather than a fix.",
        see: Some("guide#parameters"),
    },
    Teaching {
        code: "constraint_violated",
        fix: "a cross-parameter constraint failed, e.g. `param('fast') < param('slow')`. Fix \
              the sample; the constraint is refused before a Run exists precisely so this \
              costs nothing.",
        see: Some("guide#parameters"),
    },
    Teaching {
        code: "bad_expression",
        fix: "the expression does not parse. The grammar is comparisons over `feature('…')` \
              and `bar('…')` — no functions, no arithmetic beyond the comparison itself. \
              Anything more expressive belongs in a feature.",
        see: Some("guide#expressions"),
    },
];

/// The teaching for one code, if there is one.
#[must_use]
pub fn teaching(code: &str) -> Option<&'static Teaching> {
    TEACHINGS.iter().find(|t| t.code == code)
}

/// A worked example for a clause, offered after repeated rejection.
///
/// AGENT-011 §5: three rejections on the same `path` within a run additionally
/// return a worked example, because the cheapest understanding gain on a small
/// tier is seeing a correct instance rather than another rule.
///
/// Keyed on the **leading segment** of the path — `exits.rules[2]` and
/// `exits.rules[0]` are the same clause failing twice, and keying on the whole
/// path would treat them as different problems and never reach three.
#[must_use]
pub fn example_for_path(path: &str) -> Option<&'static str> {
    let clause = path.split(['.', '[']).next().unwrap_or(path);
    Some(match clause {
        "exits" => {
            "\"exits\": {\"mode\": \"rules\", \"rules\": [\
             {\"rule\": \"stop_loss\", \"fraction\": \"0.02\"}, \
             {\"rule\": \"max_holding_bars\", \"bars\": 48}]}"
        }
        "nodes" => {
            "\"nodes\": [\
             {\"id\": \"n1\", \"type\": \"condition\", \"expr\": \"feature('ema_10') > feature('ema_30')\"}, \
             {\"id\": \"n2\", \"type\": \"signal\", \"when\": \"n1\", \"emit\": \"long\"}]"
        }
        "actions" => {
            "\"actions\": [{\"on_signal\": \"long\", \"type\": \"place_order\", \
             \"order\": {\"side\": \"buy\", \"size_mode\": \"fixed\", \"size\": \"1\"}}]"
        }
        "inputs" => {
            "\"inputs\": [{\"lane\": \"market.bars.1m\", \"instrument\": \"$bound_at_init\"}, \
             {\"lane\": \"features.technical\", \"instrument\": \"$bound_at_init\", \
             \"features\": [\"ema_10\", \"ema_30\"]}]"
        }
        "parameters" => {
            "\"parameters\": {\"fast\": {\"type\": \"int\", \"min\": 5, \"max\": 50, \
             \"default\": 10}}, \"constraints\": [\"param('fast') < param('slow')\"]"
        }
        "risk_overrides" => "\"risk_overrides\": {\"max_position\": \"0.5\"}",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AGENT-011 §5's six codes are all present. They are the acceptance.
    #[test]
    fn the_six_spec_codes_all_teach() {
        for code in [
            "unknown_feature",
            "distance_without_unit",
            "when_names_expression",
            "loosening",
            "warmup_exceeds_window",
            "never_fires",
        ] {
            let t = teaching(code)
                .unwrap_or_else(|| panic!("AGENT-011 §5 lists `{code}` and the table has no row"));
            assert!(!t.fix.is_empty(), "`{code}` has an empty fix");
        }
    }

    /// No code appears twice, or `teaching` returns whichever came first and the
    /// second row is invisible.
    #[test]
    fn no_code_is_listed_twice() {
        let mut seen: Vec<&str> = Vec::new();
        for t in TEACHINGS {
            assert!(!seen.contains(&t.code), "`{}` is listed twice", t.code);
            seen.push(t.code);
        }
    }

    /// A fix says what to *do*. A row that restates the error teaches nothing,
    /// and this is the failure mode a table like this drifts into.
    #[test]
    fn every_fix_is_an_instruction_and_not_a_restatement() {
        for t in TEACHINGS {
            assert!(
                t.fix.len() > 30,
                "`{}`'s fix is {} chars — too short to be an edit somebody can make",
                t.code,
                t.fix.len()
            );
            assert!(
                !t.fix.starts_with("is not") && !t.fix.starts_with("must"),
                "`{}`'s fix opens by restating the rule rather than naming the edit: {}",
                t.code,
                t.fix
            );
        }
    }

    /// The example is keyed on the clause, so repeated failures inside one
    /// clause reach three.
    #[test]
    fn indexed_paths_within_one_clause_share_an_example() {
        let a = example_for_path("exits.rules[0]").expect("exits has an example");
        let b = example_for_path("exits.rules[2]").expect("exits has an example");
        let c = example_for_path("exits").expect("exits has an example");
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    /// Every worked example is valid JSON in context, which is the only thing
    /// that makes it worth showing to a model that will copy it.
    #[test]
    fn every_example_parses_as_part_of_a_definition() {
        for path in [
            "exits", "nodes", "actions", "inputs", "parameters", "risk_overrides",
        ] {
            let ex = example_for_path(path).expect("a declared clause has an example");
            let wrapped = format!("{{{ex}}}");
            serde_json::from_str::<serde_json::Value>(&wrapped).unwrap_or_else(|e| {
                panic!("the example for `{path}` is not valid JSON: {e}\n{wrapped}")
            });
        }
    }

    /// Every code this crate constructs has a row in the table.
    ///
    /// Scanned from the source rather than from a list, because a list is the
    /// thing that goes stale: a new `ValidationError::new("…")` with no teaching
    /// would pass a test that only walked the codes somebody remembered to
    /// register. The scan reads `src/*.rs`, so adding a rejection without a fix
    /// fails here.
    ///
    /// It skips this file, which is the self-referencing-scan trap this
    /// repository has hit repeatedly: `teach.rs` contains every code by
    /// definition, so including it would make the test pass by reading its own
    /// table.
    #[test]
    fn every_code_the_validator_constructs_can_teach() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing: Vec<String> = Vec::new();
        let mut found = 0;
        for entry in std::fs::read_dir(&dir).expect("src/ is readable") {
            let path = entry.expect("a dir entry").path();
            if path.file_name().and_then(|f| f.to_str()) == Some("teach.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            // `code: "…"` at a construction site.
            for piece in src.split("code: \"").skip(1) {
                let Some(code) = piece.split('"').next() else {
                    continue;
                };
                found += 1;
                if teaching(code).is_none() {
                    missing.push(format!("{code} (in {})", path.display()));
                }
            }
        }
        assert!(
            found > 5,
            "the scan found {found} codes, which means it is not reading the construction              sites and would pass whatever the table said"
        );
        assert!(
            missing.is_empty(),
            "these codes are constructed and cannot teach: {missing:?}. A rejection with no              fix leaves the caller to guess, which is the cost AGENT-011 §5 exists to remove."
        );
    }
}
