//! Deliberately corrupted inputs: the runtime half of `future_perturbation_test`
//! (S-3.10, AGENT-006 §5.4).
//!
//! # What the tool above this needs, and why it could not have it
//!
//! `future_perturbation_test` re-runs a strategy with one feature column moved
//! forward in time and asks whether the answer changes. If moving a column
//! *forward* — handing the decision bar a value computed from a bar it had not
//! seen — changes nothing, the strategy was already reading that bar. That is
//! the cheapest lookahead probe there is, and it works without reading a line of
//! strategy code.
//!
//! It needs one thing the platform did not have: a way to run the evaluation
//! over a column that is not the column the feature engine produced. Nothing in
//! the workspace mentioned `shift_column`, `mask_column` or `perturbation_pct`,
//! so the tool sat `BLOCKED:UPSTREAM`.
//!
//! # Where the perturbation is applied, and why not in `crates/features`
//!
//! The phase file prescribed *"a perturbation argument on
//! `features::runtime::backfill_column`"*. That is the wrong place, and it is
//! worth saying why rather than quietly doing something else.
//!
//! A shifted column is not a different *feature*; it is the same feature's
//! output read at a different index. `backfill_column` is already exactly the
//! batch shape a shift needs — `Vec<Option<f64>>`, one entry per bar — and it is
//! already public. Putting a `shift` parameter inside it would give every
//! feature implementation the ability to read forward, which is the one thing
//! `crates/features` is structured to make impossible: `value_at` resolves
//! `decision - k` through a windowed frame and cannot see past `decision`. The
//! guarantee that no feature can read a future bar is load-bearing, and a
//! perturbation argument would put a hole in it for the benefit of a test.
//!
//! So the perturbation lives here, over the produced column, and
//! `crates/features` is unchanged. INV-14 —
//! `features_versioned::backfill_and_incremental_serving_agree` — is what makes
//! that substitution legitimate: the batch column equals the value a growing
//! window would have served at every index, bit for bit, so reading
//! `column[i]` in place of `value_at(f, &rows[..=i], i)` is a no-op at zero
//! shift and the perturbation is the only difference between the two runs.
//!
//! # What is refused, and by name
//!
//! A perturbation probe is a tool whose *negative* result is the claim — "the
//! strategy did not change, so it was not reading the future". Every way of
//! producing a negative result for the wrong reason therefore has to be a
//! refusal rather than a quiet zero:
//!
//! * an empty set, or a zero-bar shift, is an unperturbed run wearing a
//!   perturbed run's label, and would diff to no divergence;
//! * a column the strategy does not use cannot change its answer, and "no
//!   divergence" over a column nothing reads is not evidence about anything;
//! * a shift wider than the window blanks the column entirely, which is
//!   `mask_column` by another name and reports as "it stopped firing";
//! * `fill_delay_ms` is refused outright: the preview produces no fills.

use std::collections::BTreeSet;

use crate::rng::DetRng;

/// One corrupted input.
///
/// `column` names a **feature** — the things `derive_requirements` produces for
/// a definition — not a bar field. A bar field feeds every feature at once, and
/// a probe that moved `close` could not name an offending indicator, which is
/// half of S-3.10's acceptance.
#[derive(Clone, Debug, PartialEq)]
pub enum Perturbation {
    /// Move one column along the bar axis.
    ///
    /// `shift_bars > 0` is the **lookahead seed**: the decision bar reads the
    /// value the column holds `shift_bars` bars later, which is information it
    /// could not have had. `shift_bars < 0` is the opposite probe — stale data —
    /// and is how "does this strategy survive acting a bar late" is asked.
    ShiftColumn { column: String, shift_bars: i32 },
    /// Blank one column: it has a value on no bar, so every condition naming it
    /// is false everywhere.
    MaskColumn { column: String },
    /// Multiply one column by seeded multiplicative noise of at most
    /// `perturbation_pct`. A result that survives a shift but not a 1% jitter is
    /// fitted to the decimals.
    JitterColumn {
        column: String,
        perturbation_pct: f64,
        seed: u64,
    },
}

impl Perturbation {
    /// The column this perturbation applies to.
    #[must_use]
    pub fn column(&self) -> &str {
        match self {
            Self::ShiftColumn { column, .. }
            | Self::MaskColumn { column }
            | Self::JitterColumn { column, .. } => column,
        }
    }

    /// The stable mode string a caller branches on.
    #[must_use]
    pub const fn mode(&self) -> &'static str {
        match self {
            Self::ShiftColumn { .. } => "shift_column",
            Self::MaskColumn { .. } => "mask_column",
            Self::JitterColumn { .. } => "jitter_column",
        }
    }
}

/// A perturbation as it was applied, carried on the result.
///
/// On the result rather than only in the request, because the comparison this
/// feeds is between two previews and the reader has to be able to tell which one
/// it is holding. A perturbed preview that looked like an ordinary one would let
/// a diff of two *unperturbed* runs report "no lookahead detected".
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppliedPerturbation {
    /// One of `shift_column`, `mask_column`, `jitter_column`.
    pub mode: &'static str,
    pub column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift_bars: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perturbation_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Bars on which the perturbed column had a value. Reported because a
    /// forward shift necessarily blanks the window's tail, and a caller
    /// comparing firing counts needs to know how many bars it lost.
    pub bars_with_value: usize,
}

/// Why a perturbation request cannot be run.
///
/// Every variant exists because the alternative was a preview that looked clean.
// Not `Eq`: `JitterOutOfRange` carries the rejected fraction, and NaN is one of
// the values it is there to reject.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PerturbationError {
    #[error(
        "a perturbation run was asked for with no perturbation. The result would be an \
         unperturbed preview carrying a perturbed run's label, and a diff against the original \
         would report no divergence — which reads as a clean bill of health. Call \
         `preview_signals` for an unperturbed run."
    )]
    Empty,
    #[error(
        "`shift_column` was asked for with `shift_bars = 0` on `{column}`, which is the \
         unperturbed column. A probe that changes nothing cannot detect anything, and its \
         no-divergence result would be read as evidence of no lookahead."
    )]
    ZeroShift { column: String },
    #[error(
        "`{column}` is not a feature this strategy uses. It reads {known}. A perturbation of a \
         column nothing reads cannot change the answer, so its no-divergence result is evidence \
         about nothing."
    )]
    UnknownColumn { column: String, known: String },
    #[error(
        "`shift_bars = {shift_bars}` over a {n_bars}-bar window leaves `{column}` with a value on \
         no bar. That is `mask_column` by another name and reports as a strategy that stopped \
         firing; if blanking the column is the intent, ask for it."
    )]
    ShiftExceedsWindow {
        column: String,
        shift_bars: i32,
        n_bars: usize,
    },
    #[error(
        "`perturbation_pct = {pct}` on `{column}` is not a fraction in (0, 1]. Zero is the \
         unperturbed column and anything above one inverts the sign of the feature, which is a \
         different strategy rather than a perturbed one."
    )]
    JitterOutOfRange { column: String, pct: f64 },
    #[error(
        "the same column `{column}` is perturbed twice in one request. Which perturbation \
         produced the divergence would be unrecoverable, and naming the offending indicator is \
         half of what this probe is for."
    )]
    DuplicateColumn { column: String },
    #[error(
        "`fill_delay_ms` cannot be honoured here: the preview produces no fills at all — no \
         position, no P&L, no execution — so there is nothing for a delay to apply to. A fill \
         delay needs the simulator's order path (`backtest::sim::run_simulation_detailed`), not \
         the preview's."
    )]
    FillDelayUnavailable,
}

/// A validated set of perturbations for one run.
///
/// Constructing one is the only way to reach the perturbed preview, so a probe
/// cannot be run under a request that would have produced a meaningless
/// negative.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerturbationSet {
    items: Vec<Perturbation>,
}

impl PerturbationSet {
    /// Validate a request against the feature columns `known` that the strategy
    /// actually reads and the `n_bars` the window holds.
    ///
    /// # Errors
    /// Every variant of [`PerturbationError`] except
    /// [`PerturbationError::FillDelayUnavailable`], which a caller raises when it
    /// is handed a `fill_delay_ms` field to translate.
    pub fn validate(
        items: Vec<Perturbation>,
        known: &[String],
        n_bars: usize,
    ) -> Result<Self, PerturbationError> {
        if items.is_empty() {
            return Err(PerturbationError::Empty);
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for p in &items {
            let column = p.column();
            if !known.iter().any(|k| k == column) {
                let mut names: Vec<&str> = known.iter().map(String::as_str).collect();
                names.sort_unstable();
                return Err(PerturbationError::UnknownColumn {
                    column: column.to_string(),
                    known: if names.is_empty() {
                        "no features at all".to_string()
                    } else {
                        names.join(", ")
                    },
                });
            }
            if !seen.insert(column) {
                return Err(PerturbationError::DuplicateColumn {
                    column: column.to_string(),
                });
            }
            match p {
                Perturbation::ShiftColumn { shift_bars: 0, .. } => {
                    return Err(PerturbationError::ZeroShift {
                        column: column.to_string(),
                    })
                }
                Perturbation::ShiftColumn { shift_bars, .. } => {
                    if shift_bars.unsigned_abs() as usize >= n_bars {
                        return Err(PerturbationError::ShiftExceedsWindow {
                            column: column.to_string(),
                            shift_bars: *shift_bars,
                            n_bars,
                        });
                    }
                }
                Perturbation::JitterColumn {
                    perturbation_pct, ..
                } => {
                    if !(*perturbation_pct > 0.0 && *perturbation_pct <= 1.0) {
                        return Err(PerturbationError::JitterOutOfRange {
                            column: column.to_string(),
                            pct: *perturbation_pct,
                        });
                    }
                }
                Perturbation::MaskColumn { .. } => {}
            }
        }
        Ok(Self { items })
    }

    /// The validated perturbations.
    #[must_use]
    pub fn items(&self) -> &[Perturbation] {
        &self.items
    }

    /// Whether this set touches `column`.
    #[must_use]
    pub fn touches(&self, column: &str) -> bool {
        self.items.iter().any(|p| p.column() == column)
    }

    /// The perturbation applying to `column`, if any.
    #[must_use]
    pub fn get(&self, column: &str) -> Option<&Perturbation> {
        self.items.iter().find(|p| p.column() == column)
    }

    /// Whether `column` is masked — the case whose "it never fired" diagnosis
    /// must name the mask rather than the warm-up.
    #[must_use]
    pub fn is_masked(&self, column: &str) -> bool {
        matches!(self.get(column), Some(Perturbation::MaskColumn { .. }))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Apply one perturbation to a produced column.
///
/// `column` is the feature's own output, one entry per bar, `None` where the
/// feature had no value. The result is the same length; `None` is the honest
/// answer wherever the perturbation leaves a bar without one.
#[must_use]
pub fn apply(perturbation: &Perturbation, column: &[Option<f64>]) -> Vec<Option<f64>> {
    match perturbation {
        Perturbation::MaskColumn { .. } => vec![None; column.len()],
        Perturbation::ShiftColumn { shift_bars, .. } => (0..column.len())
            .map(|i| {
                // A positive shift reads forward: bar `i` is handed the value
                // from bar `i + shift_bars`. That is the seeded lookahead, and
                // the window's last `shift_bars` bars necessarily have nothing.
                let src = i64::try_from(i).unwrap_or(i64::MAX) + i64::from(*shift_bars);
                usize::try_from(src)
                    .ok()
                    .and_then(|s| column.get(s).copied())
                    .flatten()
            })
            .collect(),
        Perturbation::JitterColumn {
            perturbation_pct,
            seed,
            ..
        } => {
            // Seeded, so two runs of the same request are the same run. A jitter
            // that differed between calls would make every repeat comparison
            // report a divergence, and the tool would be measuring its own noise.
            let mut rng = DetRng::new(*seed);
            column
                .iter()
                .map(|v| {
                    let u = rng.next_f64();
                    v.map(|x| x * (1.0 + perturbation_pct * (2.0 * u - 1.0)))
                        .filter(|x| x.is_finite())
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["ema_21".to_string(), "rsi_14".to_string()]
    }

    #[test]
    fn an_empty_request_is_refused_rather_than_previewing_the_original() {
        let e = PerturbationSet::validate(Vec::new(), &known(), 500).unwrap_err();
        assert_eq!(e, PerturbationError::Empty);
        assert!(
            e.to_string().contains("clean bill of health"),
            "the refusal says what the quiet version would have claimed"
        );
    }

    #[test]
    fn a_zero_bar_shift_is_refused_because_it_is_the_unperturbed_column() {
        let e = PerturbationSet::validate(
            vec![Perturbation::ShiftColumn {
                column: "ema_21".into(),
                shift_bars: 0,
            }],
            &known(),
            500,
        )
        .unwrap_err();
        assert_eq!(
            e,
            PerturbationError::ZeroShift {
                column: "ema_21".into()
            }
        );
    }

    #[test]
    fn a_column_the_strategy_does_not_read_is_refused_naming_the_ones_it_does() {
        let e = PerturbationSet::validate(
            vec![Perturbation::MaskColumn {
                column: "macd_12_26".into(),
            }],
            &known(),
            500,
        )
        .unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("macd_12_26"), "{msg}");
        assert!(msg.contains("ema_21, rsi_14"), "{msg}");
    }

    #[test]
    fn a_shift_wider_than_the_window_is_refused_as_a_mask_in_disguise() {
        let e = PerturbationSet::validate(
            vec![Perturbation::ShiftColumn {
                column: "ema_21".into(),
                shift_bars: 40,
            }],
            &known(),
            40,
        )
        .unwrap_err();
        assert!(matches!(e, PerturbationError::ShiftExceedsWindow { .. }));
    }

    #[test]
    fn a_jitter_outside_zero_to_one_is_refused() {
        for pct in [0.0, -0.1, 1.5, f64::NAN] {
            let e = PerturbationSet::validate(
                vec![Perturbation::JitterColumn {
                    column: "rsi_14".into(),
                    perturbation_pct: pct,
                    seed: 7,
                }],
                &known(),
                500,
            )
            .unwrap_err();
            assert!(
                matches!(e, PerturbationError::JitterOutOfRange { .. }),
                "pct {pct} was accepted"
            );
        }
    }

    #[test]
    fn perturbing_one_column_twice_is_refused_because_blame_would_be_unrecoverable() {
        let e = PerturbationSet::validate(
            vec![
                Perturbation::MaskColumn {
                    column: "ema_21".into(),
                },
                Perturbation::ShiftColumn {
                    column: "ema_21".into(),
                    shift_bars: 3,
                },
            ],
            &known(),
            500,
        )
        .unwrap_err();
        assert_eq!(
            e,
            PerturbationError::DuplicateColumn {
                column: "ema_21".into()
            }
        );
    }

    #[test]
    fn a_fill_delay_refusal_names_the_symbol_that_would_have_to_serve_it() {
        let msg = PerturbationError::FillDelayUnavailable.to_string();
        assert!(msg.contains("run_simulation_detailed"), "{msg}");
        assert!(msg.contains("no fills"), "{msg}");
    }

    #[test]
    fn a_forward_shift_hands_a_bar_the_value_of_a_later_one() {
        let col: Vec<Option<f64>> = (0..5).map(|i| Some(f64::from(i))).collect();
        let out = apply(
            &Perturbation::ShiftColumn {
                column: "ema_21".into(),
                shift_bars: 2,
            },
            &col,
        );
        assert_eq!(out[0], Some(2.0), "bar 0 reads bar 2 — the lookahead seed");
        assert_eq!(out[2], Some(4.0));
        assert_eq!(out[3], None, "the window's tail has nothing to read");
        assert_eq!(out[4], None);
    }

    #[test]
    fn a_backward_shift_hands_a_bar_a_stale_value_and_blanks_the_lead_in() {
        let col: Vec<Option<f64>> = (0..5).map(|i| Some(f64::from(i))).collect();
        let out = apply(
            &Perturbation::ShiftColumn {
                column: "ema_21".into(),
                shift_bars: -2,
            },
            &col,
        );
        assert_eq!(out[0], None);
        assert_eq!(out[1], None);
        assert_eq!(out[2], Some(0.0));
        assert_eq!(out[4], Some(2.0));
    }

    #[test]
    fn a_mask_leaves_no_bar_with_a_value() {
        let col: Vec<Option<f64>> = (0..5).map(|i| Some(f64::from(i))).collect();
        let out = apply(
            &Perturbation::MaskColumn {
                column: "ema_21".into(),
            },
            &col,
        );
        assert!(out.iter().all(Option::is_none));
        assert_eq!(out.len(), 5, "the column keeps its length");
    }

    #[test]
    fn a_jitter_is_the_same_jitter_on_every_run_of_the_same_request() {
        let col: Vec<Option<f64>> = (0..20).map(|i| Some(100.0 + f64::from(i))).collect();
        let p = Perturbation::JitterColumn {
            column: "ema_21".into(),
            perturbation_pct: 0.01,
            seed: 99,
        };
        assert_eq!(apply(&p, &col), apply(&p, &col));
        let moved = apply(&p, &col);
        assert!(
            moved.iter().zip(&col).any(|(a, b)| a != b),
            "a 1% jitter moved something"
        );
        for (a, b) in moved.iter().zip(&col) {
            let (a, b) = (a.unwrap(), b.unwrap());
            assert!((a - b).abs() <= b * 0.01 + 1e-9, "within the stated bound");
        }
    }

    #[test]
    fn a_perturbation_that_leaves_an_absent_value_absent_does_not_invent_one() {
        let col = vec![None, None, Some(1.0), Some(2.0)];
        for p in [
            Perturbation::JitterColumn {
                column: "ema_21".into(),
                perturbation_pct: 0.5,
                seed: 1,
            },
            Perturbation::ShiftColumn {
                column: "ema_21".into(),
                shift_bars: -1,
            },
        ] {
            let out = apply(&p, &col);
            assert!(
                out[0].is_none(),
                "{} invented a warm-up value",
                p.mode()
            );
        }
    }
}
