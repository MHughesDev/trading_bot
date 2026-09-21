//! Why it failed, derived from tickets (AGENT-013 §5.8, §8.2; Set S task
//! S-10.7).
//!
//! # The class is never chosen
//!
//! AGENT-007 §3.2's rule, and the reason for it: a model that may pick its own
//! failure class will pick the one whose `retry_condition` is "when the platform
//! is bigger", and an idea that can be re-opened by renaming its failure was
//! never closed.
//!
//! So every class here comes from a **ticket the platform minted**, and each of
//! the four §8.2 added has exactly one producer:
//!
//! | Class | The ticket that produces it |
//! |---|---|
//! | `leakage` | `leakage_audit` says the measurement is contaminated |
//! | `protocol_unstable` | the pre-registration amendment cap was reached |
//! | `fragile_parameters` | `parameter_fragility_map` says knife-edge |
//! | `regime_specific` | `regime_failure_search` says the current regime is the worst one it holds in |
//!
//! There is no argument by which a model reaches any of them, and
//! `the_agent_cannot_name_a_class` is what keeps it that way.
//!
//! # Order is not arbitrary
//!
//! [`diagnose`] checks in a fixed order and the order encodes what invalidates
//! what. **Leakage first**, because a contaminated measurement makes every number
//! below it meaningless — a run told "costs dominate" about a leaking backtest
//! has been told something false, and it will go and work on costs.
//! **Protocol instability second**, because numbers measured against three
//! different commitments are not one measurement either.
//!
//! Only after those two is it worth asking what the result actually says.

use serde::{Deserialize, Serialize};

use crate::memory::negative::FailureClass;

/// One diagnostic the platform ran, and what it found.
///
/// The ticket id is not decoration: it is what makes the diagnosis checkable
/// afterwards. A class with no resolvable ticket is a claim, and §3.2's store
/// does not hold claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub ticket_id: String,
    /// Whether this diagnostic *fired* — found the thing it looks for.
    pub fired: bool,
    /// One line of what it saw, in the diagnostic's own words.
    pub detail: String,
}

impl Finding {
    #[must_use]
    pub fn fired(ticket_id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            ticket_id: ticket_id.into(),
            fired: true,
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn clear(ticket_id: impl Into<String>) -> Self {
        Self {
            ticket_id: ticket_id.into(),
            fired: false,
            detail: "clear".into(),
        }
    }
}

/// What the platform ran, when the run asked why (§5.8).
///
/// Every field is `Option`: a diagnostic that was not run is not the same as one
/// that came back clear, and collapsing the two would let "we did not look"
/// masquerade as "we looked and it was fine". [`unrun`] is what reports the
/// difference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    /// `leakage_audit` — is the measurement contaminated.
    pub leakage_audit: Option<Finding>,
    /// `parameter_fragility_map` — does the result live at a knife edge.
    pub fragility_map: Option<Finding>,
    /// `regime_failure_search` — does it hold in one regime and nowhere else.
    pub regime_search: Option<Finding>,
    /// `cost_floor_check` — is the gross edge under the cost floor.
    pub cost_floor: Option<Finding>,
    /// The null comparison — was the null beaten.
    pub null_comparison: Option<Finding>,
    /// Out-of-sample — did it survive.
    pub replication: Option<Finding>,
    /// `capacity_curve` — does it collapse above a reachable AUM.
    pub capacity: Option<Finding>,
    /// Pre-registration amendments this run made. The cap is the ticket.
    pub prereg_amendments: u32,
    pub max_prereg_amendments: u32,
    /// Observations behind the whole bundle.
    pub n: u32,
    /// Memory entries that contradict what this run measured (§2.2.1).
    ///
    /// Carried on the bundle rather than folded into the class: a contradiction
    /// is a *second* thing to say, not a different way of saying the first, and
    /// resolving it into a class would be the platform picking a winner between
    /// two measurements — which P5 forbids in every other place it comes up.
    pub contradictions: Vec<String>,
}

/// A class, with the ticket that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub class: FailureClass,
    /// The ticket, or `amendment_cap` when the producer is the cap rather than a
    /// diagnostic run.
    pub ticket_id: String,
    pub detail: String,
    /// Memory entries that disagree, carried through so the caller surfaces them
    /// beside the diagnosis rather than instead of it.
    pub contradictions: Vec<String>,
}

/// Observations below which the only honest class is `insufficient_data`.
pub const MIN_OBSERVATIONS: u32 = 3;

/// Derive the class, or `None` when nothing the platform ran explains it.
///
/// `None` is a real answer and it is the one worth reading: it means every
/// diagnostic came back clear, which is a result about the *diagnostics* rather
/// than about the idea. A caller that turned it into a class would be inventing
/// the thing this module exists to stop being invented.
#[must_use]
pub fn diagnose(b: &DiagnosticBundle) -> Option<Diagnosis> {
    let with = |class: FailureClass, f: &Finding| {
        Some(Diagnosis {
            class,
            ticket_id: f.ticket_id.clone(),
            detail: f.detail.clone(),
            contradictions: b.contradictions.clone(),
        })
    };

    // 1. Leakage. A contaminated measurement makes every number below it
    //    meaningless, so it is checked first — and a run told "costs dominate"
    //    about a leaking backtest has been told something false and will go and
    //    work on costs.
    if let Some(f) = b.leakage_audit.as_ref().filter(|f| f.fired) {
        return with(FailureClass::Leakage, f);
    }

    // 2. Protocol instability. Numbers measured against three different
    //    commitments are not one measurement either. The producer is the cap,
    //    not a diagnostic run, which is why the ticket id is a name rather than
    //    an id — and saying so beats a fabricated one.
    if b.max_prereg_amendments > 0 && b.prereg_amendments >= b.max_prereg_amendments {
        return Some(Diagnosis {
            class: FailureClass::ProtocolUnstable,
            ticket_id: "amendment_cap".into(),
            detail: format!(
                "the pre-registration was amended {} times (cap {})",
                b.prereg_amendments, b.max_prereg_amendments
            ),
            contradictions: b.contradictions.clone(),
        });
    }

    // 3. Not enough data to say anything. Above the two that invalidate a
    //    measurement and below everything that reads one, because a thin sample
    //    is a fact about the evidence rather than about the idea.
    if b.n > 0 && b.n < MIN_OBSERVATIONS {
        return Some(Diagnosis {
            class: FailureClass::InsufficientData,
            ticket_id: "sample_size".into(),
            detail: format!("{} observations, below {MIN_OBSERVATIONS}", b.n),
            contradictions: b.contradictions.clone(),
        });
    }

    // 4. What the result actually says, cheapest-to-act-on first. Costs and the
    //    null are properties of the idea as measured; fragility and regime are
    //    properties of where it holds; capacity is a property of how much it can
    //    carry. A run that hears "costs dominate" knows what to change.
    if let Some(f) = b.cost_floor.as_ref().filter(|f| f.fired) {
        return with(FailureClass::CostsDominate, f);
    }
    if let Some(f) = b.null_comparison.as_ref().filter(|f| f.fired) {
        return with(FailureClass::NoBetterThanNull, f);
    }
    if let Some(f) = b.fragility_map.as_ref().filter(|f| f.fired) {
        return with(FailureClass::FragileParameters, f);
    }
    if let Some(f) = b.regime_search.as_ref().filter(|f| f.fired) {
        return with(FailureClass::RegimeSpecific, f);
    }
    if let Some(f) = b.replication.as_ref().filter(|f| f.fired) {
        return with(FailureClass::ReplicationFailed, f);
    }
    if let Some(f) = b.capacity.as_ref().filter(|f| f.fired) {
        return with(FailureClass::CapacityInsufficient, f);
    }
    None
}

/// Diagnostics nobody ran.
///
/// Reported beside a diagnosis because "we did not look" and "we looked and it
/// was clear" are different states, and a `None` from [`diagnose`] means very
/// different things depending on which of them produced it.
#[must_use]
pub fn unrun(b: &DiagnosticBundle) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (name, run) in [
        ("leakage_audit", b.leakage_audit.is_some()),
        ("cost_floor_check", b.cost_floor.is_some()),
        ("null_comparison", b.null_comparison.is_some()),
        ("parameter_fragility_map", b.fragility_map.is_some()),
        ("regime_failure_search", b.regime_search.is_some()),
        ("out_of_sample", b.replication.is_some()),
        ("capacity_curve", b.capacity.is_some()),
    ] {
        if !run {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> DiagnosticBundle {
        DiagnosticBundle {
            leakage_audit: Some(Finding::clear("t_leak")),
            fragility_map: Some(Finding::clear("t_frag")),
            regime_search: Some(Finding::clear("t_regime")),
            cost_floor: Some(Finding::clear("t_cost")),
            null_comparison: Some(Finding::clear("t_null")),
            replication: Some(Finding::clear("t_oos")),
            capacity: Some(Finding::clear("t_cap")),
            prereg_amendments: 0,
            max_prereg_amendments: 3,
            n: 512,
            contradictions: vec![],
        }
    }

    /// What one case does to a bundle to make its class the answer.
    type Plant = Box<dyn Fn(&mut DiagnosticBundle)>;

    /// Each of §8.2's four classes has exactly one producer.
    #[test]
    fn each_new_class_comes_from_its_own_ticket() {
        let cases: Vec<(FailureClass, Plant, &str)> = vec![
            (
                FailureClass::Leakage,
                Box::new(|b: &mut DiagnosticBundle| {
                    b.leakage_audit = Some(Finding::fired("t_leak", "higher-tf open is visible"));
                }),
                "t_leak",
            ),
            (
                FailureClass::FragileParameters,
                Box::new(|b: &mut DiagnosticBundle| {
                    b.fragility_map =
                        Some(Finding::fired("t_frag", "the edge survives one cell of 49"));
                }),
                "t_frag",
            ),
            (
                FailureClass::RegimeSpecific,
                Box::new(|b: &mut DiagnosticBundle| {
                    b.regime_search =
                        Some(Finding::fired("t_regime", "holds in 2021 and nowhere else"));
                }),
                "t_regime",
            ),
        ];
        for (class, plant, ticket) in cases {
            let mut b = bundle();
            plant(&mut b);
            let d = diagnose(&b).expect("a fired diagnostic produces a class");
            assert_eq!(d.class, class);
            assert_eq!(d.ticket_id, ticket, "the class names the ticket that made it");
            assert!(!d.detail.is_empty(), "in the diagnostic's own words");
        }
    }

    /// The fourth producer is the amendment cap, not a diagnostic run.
    #[test]
    fn the_amendment_cap_is_the_producer_of_protocol_unstable() {
        let mut b = bundle();
        b.prereg_amendments = 3;
        let d = diagnose(&b).expect("the cap fires");
        assert_eq!(d.class, FailureClass::ProtocolUnstable);
        assert_eq!(
            d.ticket_id, "amendment_cap",
            "a name rather than a fabricated ticket id: the producer is the cap"
        );
        assert!(d.detail.contains("amended 3 times"), "{}", d.detail);
    }

    /// Leakage outranks everything, and the order is what makes it useful.
    ///
    /// A run told "costs dominate" about a leaking backtest has been told
    /// something false, and it will go and work on costs.
    #[test]
    fn a_contaminated_measurement_is_diagnosed_before_what_it_measured() {
        let mut b = bundle();
        b.leakage_audit = Some(Finding::fired("t_leak", "leaking"));
        b.cost_floor = Some(Finding::fired("t_cost", "under the floor"));
        b.null_comparison = Some(Finding::fired("t_null", "p = 0.4"));
        b.fragility_map = Some(Finding::fired("t_frag", "knife edge"));
        assert_eq!(
            diagnose(&b).unwrap().class,
            FailureClass::Leakage,
            "every number below a leak is meaningless"
        );
    }

    /// An unstable protocol outranks the result it was measuring.
    #[test]
    fn three_commitments_are_not_one_measurement() {
        let mut b = bundle();
        b.prereg_amendments = 3;
        b.cost_floor = Some(Finding::fired("t_cost", "under the floor"));
        assert_eq!(
            diagnose(&b).unwrap().class,
            FailureClass::ProtocolUnstable
        );
    }

    /// Every diagnostic clear is `None`, and that is a real answer.
    #[test]
    fn nothing_explaining_it_is_reported_as_nothing() {
        assert_eq!(
            diagnose(&bundle()),
            None,
            "a class invented here would be the exact thing this module exists to prevent"
        );
        assert!(
            unrun(&bundle()).is_empty(),
            "and everything was actually run, which is what makes the None readable"
        );
    }

    /// "We did not look" and "we looked and it was clear" are different states.
    #[test]
    fn a_diagnostic_nobody_ran_is_named() {
        let mut b = bundle();
        b.leakage_audit = None;
        b.regime_search = None;
        assert_eq!(diagnose(&b), None);
        assert_eq!(
            unrun(&b),
            vec!["leakage_audit", "regime_failure_search"],
            "collapsing these into `clear` would let `we did not look` masquerade as \
             `we looked and it was fine`"
        );
    }

    /// A thin sample is a fact about the evidence, not about the idea.
    #[test]
    fn too_few_observations_is_its_own_class() {
        let mut b = bundle();
        b.n = 2;
        b.cost_floor = Some(Finding::fired("t_cost", "under the floor"));
        assert_eq!(
            diagnose(&b).unwrap().class,
            FailureClass::InsufficientData,
            "two observations cannot establish that costs dominate either"
        );
    }

    /// A contradiction travels beside the class, never instead of it.
    ///
    /// P5, in the fourth place it comes up: resolving a contradiction into a
    /// class would be the platform picking a winner between two measurements.
    #[test]
    fn a_contradiction_is_carried_not_resolved() {
        let mut b = bundle();
        b.cost_floor = Some(Finding::fired("t_cost", "under the floor"));
        b.contradictions = vec!["mem_41 says costs cleared on this pair in April".into()];
        let d = diagnose(&b).unwrap();
        assert_eq!(d.class, FailureClass::CostsDominate, "the class is unchanged");
        assert_eq!(d.contradictions.len(), 1, "and the disagreement travels with it");
    }

    /// The agent has no way to name a class.
    ///
    /// Stated as a test over the module's own surface: every constructor of a
    /// `Diagnosis` here takes a bundle, and a bundle is made of tickets. There is
    /// no argument anywhere that is a class.
    #[test]
    fn the_agent_cannot_name_a_class() {
        let source = include_str!("diagnose.rs");
        let body = source
            .split("#[cfg(test)]")
            .next()
            .expect("the module has a non-test half");
        assert!(
            !body.contains("fn classify(class:") && !body.contains("class: FailureClass) ->"),
            "no function here may take a class as an argument: the class is derived"
        );
        assert!(
            body.contains("pub fn diagnose(b: &DiagnosticBundle)"),
            "and the one entry point takes tickets"
        );
    }
}
