//! Refinement chains, and when they stop (AGENT-013 P4, §5.9, §6.5, §7.4;
//! Set S task S-10.6).
//!
//! # The failure this bounds
//!
//! A run that may refine can refine forever. Each step looks locally reasonable —
//! the last one nearly worked, this one changes the thing that broke — and the
//! chain as a whole is a search nobody counted, over a hypothesis nobody
//! re-stated, producing a result whose selection exposure is the chain length and
//! whose ledger says one trial per link.
//!
//! So refining costs something to ask for, and the chain has four stop rules.
//!
//! # Asking costs a diagnosis and a prediction
//!
//! [`Refinement`] requires `diagnosis_id` **and** `predicted_effect`, and a
//! request missing either is refused naming both. The pair is the point:
//!
//! * the diagnosis is what makes this a *repair* rather than another draw;
//! * the prediction is what makes the next result capable of being wrong.
//!
//! A refinement with no prediction cannot disappoint, so a chain of them is a
//! sequence of draws the run will describe afterwards as progress.
//!
//! # The four stops
//!
//! 1. **The cap** — `max_refinements_per_run`, a profile value, read by
//!    conformance.
//! 2. **Diminishing returns** — SPEC §10's posterior rule at run scale:
//!    `P(next beats the incumbent by delta_practical) < 0.05` over the chain.
//!    Below three links it is [`Prospect::NotComputable`] and the *cap* governs;
//!    it is never approximated, which is ADR-P2-04's discipline and the reason
//!    is that an approximation here reads as a measurement and stops a chain
//!    that was working.
//! 3. **The same failure class twice consecutively** — the repair did not
//!    repair. A third attempt at the same class is the run doing the same thing
//!    and expecting a different number.
//! 4. **A third pre-registration amendment** — the protocol stopped being one.
//!
//! Stopping produces the negative result, and its `retry_condition` comes from
//! the **last diagnosis** rather than from prose: what would have to change is a
//! property of why it failed, and the run is the worst-placed party to say it.

use serde::{Deserialize, Serialize};

use super::diagnose::Diagnosis;
use crate::memory::negative::FailureClass;

/// Why a refinement request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefineError {
    #[error(
        "a refinement needs `diagnosis_id` and `predicted_effect`, and this has {missing}. \
         The diagnosis is what makes this a repair rather than another draw; the prediction \
         is what makes the next result capable of being wrong."
    )]
    Incomplete { missing: String },
}

/// One link a run asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refinement {
    /// The diagnosis this repairs. Names a ticket, so the repair is checkable
    /// against what was actually wrong.
    pub diagnosis_id: String,
    /// What the run expects this change to do, before it does it.
    pub predicted_effect: String,
    /// The class the diagnosis found, carried so the chain can see a repeat.
    pub class: FailureClass,
    /// What the link actually scored, once it ran. `None` while it is open.
    pub observed: Option<f64>,
}

/// Validate a request (§5.9).
///
/// # Errors
/// [`RefineError::Incomplete`] naming **both** required fields, not only the
/// missing one: a model told about one requirement supplies it and is refused
/// again, and two refusals for one request teach it that the tool is unreliable
/// rather than that the request was.
pub fn request(
    diagnosis_id: &str,
    predicted_effect: &str,
    class: FailureClass,
) -> Result<Refinement, RefineError> {
    let d = diagnosis_id.trim();
    let p = predicted_effect.trim();
    if d.is_empty() || p.is_empty() {
        let missing = match (d.is_empty(), p.is_empty()) {
            (true, true) => "neither",
            (true, false) => "no `diagnosis_id`",
            _ => "no `predicted_effect`",
        };
        return Err(RefineError::Incomplete {
            missing: missing.to_string(),
        });
    }
    Ok(Refinement {
        diagnosis_id: d.to_string(),
        predicted_effect: p.to_string(),
        class,
        observed: None,
    })
}

/// Whether another link is worth trying (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prospect {
    /// `P(next beats the incumbent by delta_practical)` is below the floor.
    Exhausted,
    /// It is not.
    Worthwhile,
    /// Fewer than [`MIN_LINKS_FOR_POSTERIOR`] links. **Not** an estimate — the
    /// cap governs instead.
    NotComputable,
}

/// Links below which the posterior says nothing.
///
/// Three, because two points have no spread and one has no trend. ADR-P2-04's
/// discipline: report `NotComputable` rather than an approximation, because an
/// approximation here reads as a measurement and would stop a chain that was
/// working.
pub const MIN_LINKS_FOR_POSTERIOR: usize = 3;

/// The probability floor below which the chain has stopped paying.
pub const PROSPECT_FLOOR: f64 = 0.05;

/// `P(the next link beats the incumbent by `delta_practical`)`, from the chain
/// so far.
///
/// A normal approximation over the observed improvements — which is the shape
/// SPEC §10 uses at trial scale, applied here at run scale. It is deliberately
/// crude and deliberately bounded: it decides only whether to *keep going*, and
/// the cap is what stops the chain when this cannot speak.
#[must_use]
pub fn prospect(chain: &[Refinement], incumbent: f64, delta_practical: f64) -> Prospect {
    let scored: Vec<f64> = chain.iter().filter_map(|r| r.observed).collect();
    if scored.len() < MIN_LINKS_FOR_POSTERIOR {
        return Prospect::NotComputable;
    }
    let improvements: Vec<f64> = scored.iter().map(|v| v - incumbent).collect();
    let n = improvements.len() as f64;
    let mean = improvements.iter().sum::<f64>() / n;
    let var = improvements.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    if sd <= f64::EPSILON {
        // Every link scored the same. Either it is above the bar or it never
        // will be, and there is no spread for a probability to come from.
        return if mean >= delta_practical {
            Prospect::Worthwhile
        } else {
            Prospect::Exhausted
        };
    }
    // P(next improvement > delta), normal with the chain's own mean and spread.
    let p = 1.0 - standard_normal_cdf((delta_practical - mean) / sd);
    if p < PROSPECT_FLOOR {
        Prospect::Exhausted
    } else {
        Prospect::Worthwhile
    }
}

/// Φ(z), by the Abramowitz-Stegun 7.1.26 error function.
///
/// Accurate to ~1.5e-7, which is several orders more than a stop rule with a
/// floor at 0.05 needs, and it avoids a dependency for one function.
fn standard_normal_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// Why a chain stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stop", rename_all = "snake_case")]
pub enum Stop {
    /// `max_refinements_per_run`.
    Cap { cap: usize },
    /// The posterior says another link will not pay.
    DiminishingReturns { links: usize },
    /// The repair did not repair.
    SameClassTwice { class: FailureClass },
    /// The protocol stopped being one.
    ProtocolUnstable { amendments: u32 },
}

impl Stop {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Stop::Cap { .. } => "cap",
            Stop::DiminishingReturns { .. } => "diminishing_returns",
            Stop::SameClassTwice { .. } => "same_class_twice",
            Stop::ProtocolUnstable { .. } => "protocol_unstable",
        }
    }

    /// One line the run is told, which has to say what would change the answer.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Stop::Cap { cap } => format!(
                "this run has spent its {cap} refinements. Report what you have; the chain \
                 is the search, and a longer one is a larger search nobody counted."
            ),
            Stop::DiminishingReturns { links } => format!(
                "over {links} links the chain is no longer likely to beat the incumbent by \
                 the pre-registered delta. Another link is a draw, not a repair."
            ),
            Stop::SameClassTwice { class } => format!(
                "two refinements in a row failed as `{}`. The repair did not repair, and a \
                 third attempt at the same class is the same thing expecting a different \
                 number.",
                class.as_str()
            ),
            Stop::ProtocolUnstable { amendments } => format!(
                "the pre-registration has been amended {amendments} times. A protocol \
                 rewritten that often is not a protocol, and nothing measured against it \
                 means what it says."
            ),
        }
    }
}

/// What the chain is allowed to do next.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// `None` means keep going.
    pub stop: Option<Stop>,
    pub prospect: Prospect,
}

/// Decide whether the chain may add another link (§6.5).
///
/// Checked in the order the reasons invalidate each other: an unstable protocol
/// makes the numbers meaningless, so it comes before any rule that reads one.
#[must_use]
pub fn may_continue(
    chain: &[Refinement],
    cap: usize,
    incumbent: f64,
    delta_practical: f64,
    prereg_amendments: u32,
    max_prereg_amendments: u32,
) -> Verdict {
    let prospect = prospect(chain, incumbent, delta_practical);

    if max_prereg_amendments > 0 && prereg_amendments >= max_prereg_amendments {
        return Verdict {
            stop: Some(Stop::ProtocolUnstable {
                amendments: prereg_amendments,
            }),
            prospect,
        };
    }
    // The same class twice *consecutively*, which is not the same as twice in
    // the chain: a run that hits costs, fixes them, regresses and hits them
    // again has learned something between the two, and stopping it would stop a
    // chain that was working.
    if let [.., a, b] = chain {
        if a.class == b.class {
            return Verdict {
                stop: Some(Stop::SameClassTwice { class: b.class }),
                prospect,
            };
        }
    }
    if chain.len() >= cap {
        return Verdict {
            stop: Some(Stop::Cap { cap }),
            prospect,
        };
    }
    // Only now, and only when it can actually speak. `NotComputable` hands the
    // decision back to the cap rather than guessing — the approximation would
    // read as a measurement.
    if prospect == Prospect::Exhausted {
        return Verdict {
            stop: Some(Stop::DiminishingReturns { links: chain.len() }),
            prospect,
        };
    }
    Verdict {
        stop: None,
        prospect,
    }
}

/// What would have to change for the idea to be worth another look (§7.4).
///
/// From the **last diagnosis**, not from the run's prose. What would change the
/// answer is a property of why it failed, and the run is the worst-placed party
/// to say it: it has just spent a chain failing to and has every reason to write
/// something that sounds re-openable.
///
/// `None` when the class has no retry condition, or when nothing diagnosed the
/// failure at all. Both are answers; neither is a gap to fill in.
#[must_use]
pub fn retry_condition(
    last: Option<&Diagnosis>,
    source: &crate::memory::negative::NegativeSource,
) -> Option<String> {
    // The `None` half is the important one and it is `FailureClass`'s, not
    // this module's: a weak effect has no retry condition, because nothing
    // could change to make a measured-small effect large, and offering one
    // anyway is the relabelling `negative.rs` exists to stop. Delegating
    // rather than re-deciding is what keeps that true here too.
    let d = last?;
    let base = d.class.retry_condition(source)?;
    Some(format!("{base} (from {}: {})", d.ticket_id, d.detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::diagnose::Diagnosis;

    fn link(class: FailureClass, observed: f64) -> Refinement {
        Refinement {
            diagnosis_id: "t_diag".into(),
            predicted_effect: "costs fall below the floor".into(),
            class,
            observed: Some(observed),
        }
    }

    /// The refusal names both fields, always.
    ///
    /// A model told about one requirement supplies it and is refused again, and
    /// two refusals for one request teach it the tool is unreliable rather than
    /// that the request was.
    #[test]
    fn a_refinement_without_a_diagnosis_or_a_prediction_is_refused_naming_both() {
        for (d, p) in [("", "it will help"), ("t_1", ""), ("", "")] {
            let err = request(d, p, FailureClass::CostsDominate).expect_err("refused");
            let text = err.to_string();
            assert!(text.contains("diagnosis_id"), "{text}");
            assert!(text.contains("predicted_effect"), "{text}");
            assert!(
                text.contains("capable of being wrong"),
                "the reason is the point: {text}"
            );
        }
        request("t_1", "costs fall", FailureClass::CostsDominate).expect("both present");
    }

    /// The cap stops the chain, and it is the caller's profile value.
    #[test]
    fn the_cap_stops_the_chain() {
        let chain = vec![
            link(FailureClass::CostsDominate, 0.1),
            link(FailureClass::WeakEffect, 0.2),
            link(FailureClass::FragileParameters, 0.3),
        ];
        let v = may_continue(&chain, 3, 0.0, 0.5, 0, 3);
        assert_eq!(v.stop, Some(Stop::Cap { cap: 3 }));
        assert!(v.stop.unwrap().detail().contains("nobody counted"));

        // Two links under a cap of three keeps going.
        let v = may_continue(&chain[..2], 3, 0.0, 0.5, 0, 3);
        assert_eq!(v.stop, None);
    }

    /// The same class twice consecutively ends it.
    #[test]
    fn a_repair_that_did_not_repair_ends_the_chain() {
        let chain = vec![
            link(FailureClass::CostsDominate, 0.1),
            link(FailureClass::CostsDominate, 0.12),
        ];
        let v = may_continue(&chain, 5, 0.0, 0.5, 0, 3);
        assert_eq!(
            v.stop,
            Some(Stop::SameClassTwice {
                class: FailureClass::CostsDominate
            })
        );
        assert!(v
            .stop
            .unwrap()
            .detail()
            .contains("expecting a different number"));
    }

    /// Twice in the chain is not twice in a row.
    ///
    /// A run that hits costs, fixes them, regresses and hits them again has
    /// learned something between the two, and stopping it would stop a chain
    /// that was working.
    #[test]
    fn the_same_class_with_something_between_is_not_a_repeat() {
        let chain = vec![
            link(FailureClass::CostsDominate, 0.1),
            link(FailureClass::WeakEffect, 0.2),
            link(FailureClass::CostsDominate, 0.3),
        ];
        // Asserted as "not this rule" rather than "no stop at all": three links
        // going 0.1, 0.2, 0.3 against a delta of 0.5 are genuinely exhausted,
        // and a test that demanded `None` would be asserting something about
        // the posterior while claiming to be about the class rule.
        assert!(
            !matches!(
                may_continue(&chain, 9, 0.0, 0.5, 0, 3).stop,
                Some(Stop::SameClassTwice { .. })
            ),
            "a run that hit costs, fixed them, regressed and hit them again has learned something between the two"
        );
    }

    /// Below three links the posterior says nothing and the cap governs.
    ///
    /// ADR-P2-04's discipline: an approximation here reads as a measurement and
    /// would stop a chain that was working.
    #[test]
    fn a_short_chain_reports_not_computable_rather_than_a_number() {
        for n in 0..MIN_LINKS_FOR_POSTERIOR {
            // Alternating classes, so the same-class rule cannot fire and the
            // thing under test is the posterior's silence rather than a second
            // stop rule that happens to agree with it.
            let chain: Vec<Refinement> = (0..n)
                .map(|i| {
                    let class = if i % 2 == 0 {
                        FailureClass::WeakEffect
                    } else {
                        FailureClass::CostsDominate
                    };
                    link(class, f64::from(u8::try_from(i).unwrap()))
                })
                .collect();
            assert_eq!(
                prospect(&chain, 0.0, 0.5),
                Prospect::NotComputable,
                "{n} links"
            );
            assert_eq!(
                may_continue(&chain, 9, 0.0, 0.5, 0, 3).stop,
                None,
                "and it does not stop the chain on an unknown"
            );
        }
    }

    /// A chain going nowhere is exhausted.
    #[test]
    fn a_chain_that_stopped_paying_is_stopped() {
        // Four links, all far below a delta of 0.5, with little spread.
        let chain: Vec<Refinement> = [0.01, 0.02, 0.015, 0.012]
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let class = if i % 2 == 0 {
                    FailureClass::WeakEffect
                } else {
                    FailureClass::CostsDominate
                };
                link(class, v)
            })
            .collect();
        assert_eq!(prospect(&chain, 0.0, 0.5), Prospect::Exhausted);
        let v = may_continue(&chain, 99, 0.0, 0.5, 0, 3);
        assert_eq!(v.stop, Some(Stop::DiminishingReturns { links: 4 }));
    }

    /// A chain that is climbing is not stopped.
    #[test]
    fn a_chain_that_is_still_paying_keeps_going() {
        let chain: Vec<Refinement> = [0.3, 0.55, 0.8, 0.9]
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let class = if i % 2 == 0 {
                    FailureClass::WeakEffect
                } else {
                    FailureClass::CostsDominate
                };
                link(class, v)
            })
            .collect();
        assert_eq!(prospect(&chain, 0.0, 0.5), Prospect::Worthwhile);
        assert_eq!(may_continue(&chain, 99, 0.0, 0.5, 0, 3).stop, None);
    }

    /// An unstable protocol outranks every rule that reads a number.
    #[test]
    fn a_rewritten_protocol_stops_the_chain_first() {
        let chain = vec![
            link(FailureClass::CostsDominate, 0.1),
            link(FailureClass::CostsDominate, 0.1),
        ];
        let v = may_continue(&chain, 9, 0.0, 0.5, 3, 3);
        assert_eq!(v.stop, Some(Stop::ProtocolUnstable { amendments: 3 }));
    }

    fn source(class: FailureClass) -> crate::memory::negative::NegativeSource {
        crate::memory::negative::NegativeSource::GateRefused {
            ticket_id: "t_gate".into(),
            gate: "Integrity".into(),
            class,
            detail: "refused".into(),
            window: crate::memory::DataWindow::new("2024-01-01", "2024-06-30"),
        }
    }

    /// The retry condition comes from the diagnosis, not from the run.
    #[test]
    fn the_retry_condition_is_the_diagnosis_not_the_runs_prose() {
        let d = Diagnosis {
            class: FailureClass::CostsDominate,
            ticket_id: "t_cost".into(),
            detail: "break-even is 1.8bp against a 20bp floor".into(),
            contradictions: vec![],
        };
        let text = retry_condition(Some(&d), &source(FailureClass::CostsDominate))
            .expect("costs have a retry condition");
        assert!(text.contains("t_cost"), "{text}");
        assert!(text.contains("1.8bp"), "{text}");
        assert!(text.contains("lower effective cost"), "{text}");
    }

    /// A class with no retry condition does not acquire one here.
    ///
    /// Nothing could change to make a measured-small effect large, and offering
    /// a condition anyway is the relabelling `negative.rs` exists to stop.
    #[test]
    fn a_weak_effect_gets_no_retry_condition_from_this_module_either() {
        let d = Diagnosis {
            class: FailureClass::WeakEffect,
            ticket_id: "t_weak".into(),
            detail: "the effect is 0.02 against a delta of 0.5".into(),
            contradictions: vec![],
        };
        assert_eq!(
            retry_condition(Some(&d), &source(FailureClass::WeakEffect)),
            None
        );
    }

    /// Nothing diagnosed means nothing is known.
    #[test]
    fn an_undiagnosed_failure_does_not_get_an_invented_retry_condition() {
        assert_eq!(
            retry_condition(None, &source(FailureClass::CostsDominate)),
            None,
            "an invented condition is one nobody can check"
        );
    }

    /// Φ is right enough for a floor at 0.05.
    #[test]
    fn the_normal_cdf_is_accurate_where_it_is_used() {
        for (z, expected) in [
            (0.0, 0.5),
            (1.0, 0.841_344_7),
            (-1.0, 0.158_655_3),
            (1.644_853_6, 0.95),
            (-1.644_853_6, 0.05),
        ] {
            let got = standard_normal_cdf(z);
            assert!(
                (got - expected).abs() < 1e-5,
                "Φ({z}) = {got}, expected {expected}"
            );
        }
    }
}
