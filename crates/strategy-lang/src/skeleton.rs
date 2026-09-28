//! A correct first version per hypothesis family (S-5.12; AGENT-011 §4).
//!
//! # Why a skeleton is a compiled artefact and not a text snippet
//!
//! `strategy_skeleton(hypothesis_family)` exists so that "the first version is
//! never a syntax lesson". A snippet that does not compile teaches the opposite
//! and teaches it with authority, because a model reads a platform-supplied
//! example as the shape it is meant to imitate: one wrong clause in here
//! propagates into every strategy a small tier writes for the rest of the
//! session. So every skeleton in this module is checked by
//! [`Skeleton::definition`] through the same parser and the same compiler a
//! model's own text goes through: `every_worked_skeleton_compiles` fails the
//! build if one of them stops being a strategy, and
//! `every_skeleton_is_reachable_through_the_clause_grammar` in
//! `tests/round_trip.rs` fails it if one stops being writable by the tier that
//! needs the example most.
//!
//! # Why two of the six families have no skeleton
//!
//! AGENT-011 §4 names six: trend, mean reversion, breakout, carry, vol target
//! and model signal. Four of them can be said with the families
//! `features::registry` actually serves. Two cannot:
//!
//! - **carry** needs a funding rate or a basis, and no family computes either —
//!   they are DATA-006's F3 liquidity/cost set, unbuilt.
//! - **model signal** needs `prediction(series_ref)`, which is F4 and reaches
//!   the MLOps registry rather than the bar.
//!
//! Both are **refused by name**, listing the families that do not exist, rather
//! than approximated with something that does. A "carry" skeleton built out of
//! `returns_n` would compile, run, and be a different strategy than the one its
//! name promises — and the model would have no way to find that out.
//! `a_refused_family_names_features_the_engine_really_does_not_have` checks the
//! refusal against the live registry, so the day the engine gains them the test
//! fails and asks for the skeleton rather than leaving a stale excuse in place.

use std::collections::BTreeMap;

use domain::strategy_def::{params::ParamSpec, StrategyDefinition};

use crate::catalogue::Catalogue;
use crate::diagnostic::{Code, Diagnostic, Span};

/// A skeleton has no source text of its own to point at.
const NO_SOURCE: Span = Span::new(0, 0, 0, 0);

/// The hypothesis families AGENT-011 §4 names.
///
/// A closed enum, and the `body` table matches it exhaustively, so a seventh family
/// cannot be added without deciding what it writes — which is the only way this
/// list and the list of skeletons stay the same list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HypothesisFamily {
    Trend,
    MeanReversion,
    Breakout,
    Carry,
    VolTarget,
    ModelSignal,
}

impl HypothesisFamily {
    /// Every family, in the order AGENT-011 §4 lists them.
    pub const ALL: [Self; 6] = [
        Self::Trend,
        Self::MeanReversion,
        Self::Breakout,
        Self::Carry,
        Self::VolTarget,
        Self::ModelSignal,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trend => "trend",
            Self::MeanReversion => "mean_reversion",
            Self::Breakout => "breakout",
            Self::Carry => "carry",
            Self::VolTarget => "vol_target",
            Self::ModelSignal => "model_signal",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }
}

impl std::fmt::Display for HypothesisFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A worked starting point: the text, and what a reader needs to know about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skeleton {
    pub family: HypothesisFamily,
    /// SLv2, compiling, with the comments that say why each threshold is what
    /// it is.
    pub text: &'static str,
    /// What this skeleton is doing, and — where it differs from the textbook
    /// form of its hypothesis — what the platform cannot do and why.
    pub notes: &'static str,
}

impl Skeleton {
    /// The canonical definition this skeleton compiles to.
    ///
    /// # Errors
    ///
    /// Every diagnostic the text produced. A skeleton that returns `Err` is a
    /// defect in this module, not in the caller's input, and the test that
    /// walks every family exists to stop one being committed.
    pub fn definition(&self) -> Result<StrategyDefinition, Vec<Diagnostic>> {
        crate::text_to_definition(self.text)
    }

    /// The parameters a sweep would vary, read off the compiled definition
    /// rather than listed a second time here. AGENT-011 §4's `params[]`.
    ///
    /// # Errors
    ///
    /// As [`Skeleton::definition`].
    pub fn params(&self) -> Result<BTreeMap<String, ParamSpec>, Vec<Diagnostic>> {
        Ok(self.definition()?.parameters)
    }
}

/// The starting point for a hypothesis family.
///
/// # Errors
///
/// [`Code::UnknownFeature`], naming the feature families the platform would
/// need before this family can be written at all.
pub fn skeleton(family: HypothesisFamily) -> Result<Skeleton, Diagnostic> {
    skeleton_with(family, &Catalogue::engine())
}

/// As [`skeleton`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`skeleton`].
pub fn skeleton_with(family: HypothesisFamily, cat: &Catalogue) -> Result<Skeleton, Diagnostic> {
    match body(family) {
        Body::Worked { text, notes } => Ok(Skeleton {
            family,
            text,
            notes,
        }),
        Body::Missing { needs, why } => {
            let absent: Vec<&str> = needs
                .iter()
                .copied()
                .filter(|n| cat.family(n).is_none())
                .collect();
            // When the engine has gained every family this hypothesis needs,
            // the refusal has become untrue and the skeleton is writable. Say
            // that, rather than quoting a list of things that now exist.
            let message = if absent.is_empty() {
                format!(
                    "there is no `{family}` skeleton yet, and the features it needs \
                     ({}) now exist — it can be written",
                    needs.join(", ")
                )
            } else {
                format!(
                    "a `{family}` strategy needs {why}, and no feature family computes it: \
                     {} {} in the engine registry",
                    absent.join(", "),
                    if absent.len() == 1 {
                        "is not"
                    } else {
                        "are not"
                    }
                )
            };
            Err(
                Diagnostic::semantic(Code::UnknownFeature, NO_SOURCE, message)
                    .with_fix(
                        "pick a family the engine can serve — `trend`, `mean_reversion`, \
                     `breakout` or `vol_target` — or ask `list_features()` for what exists",
                    )
                    .with_see("guide#skeletons"),
            )
        }
    }
}

/// Every skeleton the platform can currently write, in family order.
#[must_use]
pub fn worked() -> Vec<Skeleton> {
    HypothesisFamily::ALL
        .into_iter()
        .filter_map(|f| skeleton(f).ok())
        .collect()
}

enum Body {
    Worked {
        text: &'static str,
        notes: &'static str,
    },
    Missing {
        /// Feature families the platform would need. Checked against the
        /// catalogue before the refusal is written, so the reason stays true.
        needs: &'static [&'static str],
        why: &'static str,
    },
}

/// The one table. Exhaustive over [`HypothesisFamily`] by construction.
const fn body(family: HypothesisFamily) -> Body {
    match family {
        HypothesisFamily::Trend => Body::Worked {
            text: TREND,
            notes: TREND_NOTES,
        },
        HypothesisFamily::MeanReversion => Body::Worked {
            text: MEAN_REVERSION,
            notes: MEAN_REVERSION_NOTES,
        },
        HypothesisFamily::Breakout => Body::Worked {
            text: BREAKOUT,
            notes: BREAKOUT_NOTES,
        },
        HypothesisFamily::VolTarget => Body::Worked {
            text: VOL_TARGET,
            notes: VOL_TARGET_NOTES,
        },
        HypothesisFamily::Carry => Body::Missing {
            needs: &["funding_rate", "basis", "carry"],
            why: "the funding rate or the futures basis it is paid out of",
        },
        HypothesisFamily::ModelSignal => Body::Missing {
            needs: &["prediction", "regime", "text"],
            why: "a model's prediction read through the MLOps registry \
                  (DATA-006 F4, unbuilt)",
        },
    }
}

// ── the four worked skeletons ────────────────────────────────────────────────

/// AGENT-011 §3's own example, made swept and given exits.
const TREND: &str = "\
# Trend: hold while the fast baseline leads the slow one, and do not buy a move
# that has already run.
strategy trend_skeleton on $bound_at_init timeframe 1h
# The two baselines are what the hypothesis is about, so they are the two things
# a sweep varies. `constraint` keeps the sweep from proposing a fast line slower
# than the slow one, which would invert the rule rather than weaken it.
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
constraint param('fast') < param('slow')
input fast_ma = ema(close, fast)
input slow_ma = ema(close, slow)
# 60 rather than 70: this is a trend rule, so the filter is there to refuse an
# entry into an extended move, not to call a top.
entry long when fast_ma > slow_ma and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
# A trend that has not worked in two days was not the trend this rule is about.
exit time 48 bars
risk max_position 0.5
";

const TREND_NOTES: &str = "\
`ema_N` warms up over 5N bars, so at `slow = 21` the first tradeable bar is 105 \
bars into the window — check `warmup_ok` before reading a preview that shows no \
entries.

The textbook form of this rule is a crossover — enter on the bar where the fast line crosses the slow one. `crossed_above` is refused by name (`needs_history`). The refusal is **not** that the runtime cannot do it: `crates/strategy-runtime` has the cross operations, a previous-value register and a stateful evaluator, and drives a cross end to end in its own tests. What is missing is that the canonical expression grammar has no such function, so a strategy using one cannot be stored, and replay evaluates conditions without history, so it could not be backtested either. The level comparison here fires on every bar the trend holds rather than on the one bar it starts, which the exits and the risk gate then bound. `macd(close, fast, slow) > 0` says the same thing in one feature.";

const MEAN_REVERSION: &str = "\
# Mean reversion: buy a stretch away from the mean, but only while the series is
# the kind that comes back.
strategy mean_reversion_skeleton on $bound_at_init timeframe 1h
param window: int[10..100 step 5] = 20
# Negative, because it is a distance below the mean. Sweep it: this threshold
# decides whether the rule ever fires, and it is the first thing a blocking
# profile will point at.
param stretch: float[-4..-0.5] = -2
input z = zscore(close, window)
# Lo-MacKinlay variance ratio below 1 is the mean-reverting regime. Above 1 the
# series is trending and this rule is standing in front of it.
input persistence = variance_ratio(close, 64, 4)
entry long when z < stretch and persistence < 1
size equity_fraction 0.1
# The target is nearer than the stop on purpose: a mean-reversion rule wins
# often and small, and the stop is what stops it losing rarely and large.
exit target 1.5 % from entry
exit stop 3 % from entry
exit time 24 bars
risk max_orders_per_min 6
";

const MEAN_REVERSION_NOTES: &str = "\
`zscore_N` is scale-free, so the `stretch` threshold means the same thing on any \
instrument — which is why the rule compares against a z-score rather than \
against a price distance.

The regime gate is the half of this hypothesis that is usually missing. A \
mean-reversion rule without one is a rule that pays for every trend, and the \
diagnostics will report it as \"loses in trending months\" rather than as a \
missing clause.";

const BREAKOUT: &str = "\
# Breakout: take the move out of a channel, in whichever direction it leaves.
strategy breakout_skeleton on $bound_at_init timeframe 1h
param channel: int[10..200 step 10] = 20
input upper = donchian_upper(close, channel)
input lower = donchian_lower(close, channel)
# Volume is the confirmation: a channel break on an ordinary bar is usually
# noise, and `rel_volume` is 1.0 on an ordinary bar by construction.
input volume_surge = rel_volume(close, 20)
entry long when close > upper and volume_surge > 1.5
entry short when close < lower and volume_surge > 1.5
size fixed 0.01
# Trailing rather than a fixed target: the whole hypothesis is that the move
# continues, so the exit should not cap it.
exit trailing 3 % from high_water
exit time 96 bars
";

const BREAKOUT_NOTES: &str = "\
`donchian_upper_N` is the highest high of the last N bars **including this \
one**, so `close > donchian_upper(close, N)` is true only on a bar that closes \
at a new high rather than on every bar above the old channel. That is the \
rising edge this rule wants, and it is the one place the language gets one \
without `crossed_above`.

Both sides share one `size` clause and one exit policy, because the canonical \
order specification carries sizing per action and the exit policy is per \
strategy. A short book with a different stop needs a second strategy.";

const VOL_TARGET: &str = "\
# Volatility target: keep what is at risk per trade steady, and stand aside when
# the market is not the one the rule was measured on.
strategy vol_target_skeleton on $bound_at_init timeframe 1h
param vol_window: int[10..200 step 10] = 20
# Band width is a fraction of the mean, so this ceiling is scale-free and means
# the same thing on any instrument.
param vol_ceiling: float[0.01..0.2 log] = 0.08
input band_width = bb_width(close, vol_window, 2)
# Efficiency ratio is net move over path length: 0.4 asks for a market that is
# going somewhere rather than one that is merely moving.
input trend_quality = efficiency_ratio(close, 50)
entry long when band_width < vol_ceiling and trend_quality > 0.4
size risk_per_trade 0.01
exit stop 2 % from entry
exit time 72 bars
risk max_position 0.25
";

const VOL_TARGET_NOTES: &str = "\
**This skeleton targets volatility with the mechanisms the runtime has, and \
they are not the textbook ones. Read this before copying it.**

`size vol_target 0.15` does not compile: the canonical order specification has \
three size modes — `fixed`, `equity_fraction` and `risk_per_trade` — and none \
of them scales a quantity by a measured volatility. It is refused by name \
(`size_mode_not_representable`) rather than approximated. `risk_per_trade` is \
the nearest mechanism that exists: it sizes from a risk unit, so a constant \
fraction of equity is at risk on each entry given the stop. Combined with the \
fixed 2 % stop here, position size moves inversely with the stop distance in \
price terms but not with realised volatility.

The volatility gate is a *level* comparison and not a percentile, for the same \
kind of reason. Ranking volatility against its own history is \
`pct_rank(rv(close, 20), 200)` — a feature of a feature — and the canonical \
expression grammar's only named value is `feature('name')`, a name rather than \
an expression, so a family cannot take another family as its input. \
`bb_width` is used instead because it is already normalised by the mean, which \
makes a fixed ceiling defensible where a fixed `rv` threshold would be \
instrument-specific and would silently stop firing on a different asset.";

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_def::exits::ExitPolicy;

    /// The whole point of the task: a skeleton that does not compile is a
    /// worked example teaching something false, with the platform's authority
    /// behind it.
    #[test]
    fn every_worked_skeleton_compiles() {
        let worked = worked();
        assert_eq!(
            worked.len(),
            4,
            "four families can be written with the engine's own catalogue; \
             the scan found {}",
            worked.len()
        );
        for s in &worked {
            let def = s
                .definition()
                .unwrap_or_else(|e| panic!("the {} skeleton does not compile: {e:#?}", s.family));
            assert_eq!(
                def.strategy_id,
                format!("{}_skeleton", s.family),
                "a skeleton's name says which family it came from"
            );
        }
    }

    /// AGENT-011 §8.2 asks the three named skeletons to preview "with no
    /// warm-up violation", which this crate cannot run — it has no bars. What
    /// it can hold is the half of that criterion which is a property of the
    /// text: every skeleton declares exits, so a position it opens closes.
    #[test]
    fn every_worked_skeleton_says_what_closes_a_position() {
        for s in worked() {
            let def = s.definition().expect("compiles");
            match def.exits {
                Some(ExitPolicy::Rules { ref rules }) => {
                    assert!(!rules.is_empty(), "{}: an empty rule list", s.family);
                }
                other => panic!(
                    "{}: a skeleton delegating its exits teaches delegation: {other:?}",
                    s.family
                ),
            }
        }
    }

    /// Each skeleton is something a sweep can vary, and the parameters are read
    /// off the compiled definition rather than listed a second time in prose.
    #[test]
    fn every_worked_skeleton_declares_parameters_the_sweeper_can_read() {
        for s in worked() {
            let params = s.params().expect("compiles");
            assert!(
                !params.is_empty(),
                "{}: a skeleton with nothing to sweep is a fixed answer",
                s.family
            );
            for (name, spec) in &params {
                spec.validate_spec(name)
                    .unwrap_or_else(|e| panic!("{}: `{name}` is not sweepable: {e}", s.family));
            }
        }
    }

    /// The refusal has to stay true. If the engine gains a funding rate this
    /// fails, and what it asks for is the skeleton — not a wider allowlist.
    #[test]
    fn a_refused_family_names_features_the_engine_really_does_not_have() {
        let cat = Catalogue::engine();
        let mut refused = 0;
        for family in HypothesisFamily::ALL {
            let Body::Missing { needs, .. } = body(family) else {
                continue;
            };
            refused += 1;
            assert!(!needs.is_empty(), "{family}: a refusal naming nothing");
            for n in needs {
                assert!(
                    cat.family(n).is_none(),
                    "the {family} skeleton is refused because `{n}` does not exist, and it does \
                     — write the skeleton"
                );
            }
            let d = skeleton_with(family, &cat).expect_err("refused");
            assert_eq!(d.code, Code::UnknownFeature);
            assert!(
                needs.iter().any(|n| d.message.contains(n)),
                "{family}: the refusal does not name what is missing: {}",
                d.message
            );
            assert!(d.fix.is_some() && d.see.is_some(), "{family}");
        }
        assert_eq!(refused, 2, "carry and model_signal are the two that refuse");
    }

    /// The notes on `vol_target` are the load-bearing half of that skeleton: it
    /// is not the textbook rule and a reader who assumes it is will size wrong.
    #[test]
    fn the_vol_target_notes_name_the_mechanism_that_is_missing() {
        let s = skeleton(HypothesisFamily::VolTarget).expect("worked");
        assert!(
            s.notes.contains("size_mode_not_representable"),
            "the notes must name the refusal a reader will hit"
        );
        assert!(s.notes.contains("risk_per_trade"));
        // And the text must not claim the sizing it cannot do.
        assert!(!s.text.contains("size vol_target"));
    }

    #[test]
    fn a_family_name_round_trips_through_its_string() {
        for f in HypothesisFamily::ALL {
            assert_eq!(HypothesisFamily::parse(f.as_str()), Some(f));
        }
        assert_eq!(HypothesisFamily::parse("momentum"), None);
    }
}
