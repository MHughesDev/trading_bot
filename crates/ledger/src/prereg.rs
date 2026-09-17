//! Pre-registration at the scale of a run (AGENT-013 §7.2, P2; Set S task
//! S-10.3).
//!
//! # What this closes
//!
//! The trial ledger makes every look *counted* (INV-16). Pre-registration makes
//! every look *pre-committed*. The platform already does the second per
//! Experiment — `Registration::prereg_hash` locks the effect size, the policy and
//! the campaign before the first execution — and that leaves the largest door in
//! the building still open: **the falsifier**.
//!
//! Without one, a run can look at a result and then decide what would have
//! counted as failure. Every statistic downstream still computes, the trial count
//! is still honest, and the conclusion is still worthless, because "it worked"
//! and "we kept looking until something worked" are the same trace.
//!
//! So a run seals this object before its first trial, and every ticket it spends
//! carries the hash.
//!
//! # `delta_practical` has no default, and that is the whole technique
//!
//! ADR-P0-08's rule, applied again: the field has no `serde(default)`, so a
//! request that omits it **fails to deserialise** rather than arriving with a
//! zero. A default here would be a decision nobody made about what improvement
//! is worth acting on, inherited silently by every registration that followed.
//!
//! The same argument applies to the falsifier, the benchmark and the window.
//! None of them has a default; a registration missing any of them is not a
//! registration.
//!
//! # Amendments are lineage, not edits
//!
//! An amendment writes a **new row** naming what it supersedes. The trial count
//! carries over, so amending cannot buy a fresh budget — which is the move the
//! rule exists to prevent. The third amendment ends the run
//! `blocked: protocol_unstable`: a protocol rewritten three times is not a
//! protocol, and the honest reading of that run is that it never had one.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreregError {
    #[error("{field} is required and has no default: {why}")]
    Missing { field: &'static str, why: String },
    #[error("delta_practical must be finite and positive; got {0}")]
    BadDelta(String),
    #[error("the window runs backwards: {start} → {end}")]
    BackwardsWindow { start: String, end: String },
    #[error(
        "this run has amended its registration {amendments} times \
         (max {max}); the run is blocked as protocol_unstable"
    )]
    TooManyAmendments { amendments: u32, max: u32 },
    #[error("an amendment must name what it supersedes")]
    AmendmentWithoutLineage,
    #[error("a first registration supersedes nothing, but this one names {0}")]
    FirstRegistrationSupersedes(Uuid),
}

/// The period a registration is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub start: String,
    pub end: String,
}

/// What a run commits to before it spends a trial (§7.2).
///
/// Every field is required. `serde` has no `default` on any of them, so a
/// registration that omits one fails at the boundary rather than arriving
/// half-formed and being completed by whatever the code happened to assume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preregistration {
    pub id: Uuid,
    pub run_id: Uuid,
    /// SPEC §12.1's shape: a vector and its constraints, never a scalar score.
    pub objective: serde_json::Value,
    pub benchmark: serde_json::Value,
    /// The smallest improvement that would change a decision. **No default.**
    pub delta_practical: f64,
    pub window: Window,
    /// Computed from the pipeline (SPEC §12.2), never typed — which is why it is
    /// [`seal`]'s output rather than the caller's input.
    pub embargo_bars: u32,
    pub cost_model_version: String,
    pub null_family: String,
    /// Trials this run may spend. Carried over by an amendment.
    pub trial_budget: u32,
    pub gate_profile: String,
    pub primary_hypothesis: String,
    /// The result that would refute it. Written before the first trial or not at
    /// all — this field is the reason the whole object exists.
    pub falsifier: String,
    /// The registration this one replaces. `None` on a first registration.
    pub supersedes: Option<Uuid>,
    /// How many times this run has amended. Zero on a first registration.
    pub amendment: u32,
}

/// What a caller supplies. The embargo and the hash are the platform's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub run_id: Uuid,
    pub objective: serde_json::Value,
    pub benchmark: serde_json::Value,
    /// No `serde(default)`: ADR-P0-08's technique. A request without it does not
    /// deserialise.
    pub delta_practical: f64,
    pub window: Window,
    pub cost_model_version: String,
    pub null_family: String,
    pub trial_budget: u32,
    pub gate_profile: String,
    pub primary_hypothesis: String,
    pub falsifier: String,
}

/// The default ceiling on amendments, and a profile value reads over it.
///
/// Three, because two is a correction and three is a pattern: a protocol
/// rewritten three times inside one run is not a protocol.
pub const MAX_PREREG_AMENDMENTS: u32 = 3;

/// Seal a first registration for a run.
///
/// `embargo` comes from the pipeline through
/// [`dataplane::split::compute_embargo`] — the same arithmetic the split spec
/// uses, so a run's embargo and its splits cannot disagree.
///
/// # Errors
/// [`PreregError`] when a required field is empty or the window runs backwards.
pub fn seal(
    draft: Draft,
    embargo: &dataplane::split::EmbargoInputs,
) -> Result<Preregistration, PreregError> {
    validate(&draft)?;
    Ok(Preregistration {
        id: Uuid::new_v4(),
        run_id: draft.run_id,
        objective: draft.objective,
        benchmark: draft.benchmark,
        delta_practical: draft.delta_practical,
        window: draft.window,
        embargo_bars: dataplane::split::compute_embargo(embargo),
        cost_model_version: draft.cost_model_version,
        null_family: draft.null_family,
        trial_budget: draft.trial_budget,
        gate_profile: draft.gate_profile,
        primary_hypothesis: draft.primary_hypothesis,
        falsifier: draft.falsifier,
        supersedes: None,
        amendment: 0,
    })
}

/// Amend a sealed registration.
///
/// Returns a **new** row naming what it supersedes. The trial budget carries
/// over from `previous` and the caller's number is ignored — amending must not
/// be a way to buy a fresh budget, which is the move the amendment cap exists to
/// bound in the first place.
///
/// # Errors
/// [`PreregError::TooManyAmendments`] at the cap, plus every error [`seal`] can
/// return.
pub fn amend(
    previous: &Preregistration,
    draft: Draft,
    embargo: &dataplane::split::EmbargoInputs,
    max_amendments: u32,
) -> Result<Preregistration, PreregError> {
    let amendment = previous.amendment + 1;
    if amendment >= max_amendments {
        return Err(PreregError::TooManyAmendments {
            amendments: amendment,
            max: max_amendments,
        });
    }
    let mut next = seal(draft, embargo)?;
    next.supersedes = Some(previous.id);
    next.amendment = amendment;
    // Carried, not taken from the draft.
    next.trial_budget = previous.trial_budget;
    Ok(next)
}

fn validate(d: &Draft) -> Result<(), PreregError> {
    let required = [
        ("cost_model_version", d.cost_model_version.trim()),
        ("null_family", d.null_family.trim()),
        ("gate_profile", d.gate_profile.trim()),
        ("primary_hypothesis", d.primary_hypothesis.trim()),
        (
            "falsifier",
            d.falsifier.trim(),
        ),
        ("window.start", d.window.start.trim()),
        ("window.end", d.window.end.trim()),
    ];
    for (field, value) in required {
        if value.is_empty() {
            return Err(PreregError::Missing {
                field,
                why: match field {
                    "falsifier" => "a hypothesis with no falsifier cannot be refuted, so a run \
                                    carrying one can only ever confirm"
                        .into(),
                    _ => "it has no default, because a default here is a decision nobody made"
                        .into(),
                },
            });
        }
    }
    if !d.delta_practical.is_finite() || d.delta_practical <= 0.0 {
        return Err(PreregError::BadDelta(d.delta_practical.to_string()));
    }
    if d.window.start > d.window.end {
        return Err(PreregError::BackwardsWindow {
            start: d.window.start.clone(),
            end: d.window.end.clone(),
        });
    }
    if d.objective.is_null() {
        return Err(PreregError::Missing {
            field: "objective",
            why: "a vector and its constraints, never a scalar score (ADR-012)".into(),
        });
    }
    if d.benchmark.is_null() {
        return Err(PreregError::Missing {
            field: "benchmark",
            why: "better than what, decided before the result".into(),
        });
    }
    Ok(())
}

impl Preregistration {
    /// The hash every ticket this run spends carries.
    ///
    /// **The falsifier is in it.** That is the extension §7.2 makes over the
    /// per-Experiment `prereg_hash`: without the falsifier in the hash, a run
    /// could seal a registration, change what would have counted as failure, and
    /// leave every ticket still matching.
    ///
    /// The amendment number is in it too, so two registrations that differ only
    /// by lineage are still two hashes — an amendment that produced the same hash
    /// as the thing it superseded would make the lineage unreadable from a
    /// ticket.
    #[must_use]
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.run_id.as_bytes());
        h.update(canonical(&self.objective).as_bytes());
        h.update(canonical(&self.benchmark).as_bytes());
        h.update(self.delta_practical.to_be_bytes());
        h.update(self.window.start.as_bytes());
        h.update(b"..");
        h.update(self.window.end.as_bytes());
        h.update(self.embargo_bars.to_be_bytes());
        h.update(self.cost_model_version.as_bytes());
        h.update(self.null_family.as_bytes());
        h.update(self.gate_profile.as_bytes());
        h.update(self.primary_hypothesis.as_bytes());
        h.update(self.falsifier.as_bytes());
        h.update(self.amendment.to_be_bytes());
        format!("sha256:{}", hex::encode(h.finalize()))
    }

    /// The chain this registration ends, oldest first, given every row for the
    /// run.
    #[must_use]
    pub fn lineage<'a>(&'a self, all: &'a [Preregistration]) -> Vec<&'a Preregistration> {
        let mut chain = vec![self];
        let mut cursor = self.supersedes;
        while let Some(id) = cursor {
            match all.iter().find(|p| p.id == id) {
                Some(p) => {
                    chain.push(p);
                    cursor = p.supersedes;
                }
                None => break,
            }
        }
        chain.reverse();
        chain
    }
}

/// Stable JSON, so the hash does not move when a map's iteration order does.
fn canonical(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{k}:{}", canonical(&m[*k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(canonical).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn embargo() -> dataplane::split::EmbargoInputs {
        dataplane::split::EmbargoInputs {
            horizon_bars: 4,
            max_lookback_bars: 20,
            max_knowledge_lag_ms: 120_000,
            settlement_lag_bars: 1,
        }
    }

    fn draft() -> Draft {
        Draft {
            run_id: Uuid::new_v4(),
            objective: json!({"maximize": ["sharpe"], "subject_to": {"max_dd": 0.2}}),
            benchmark: json!({"null_family": "block_bootstrap"}),
            delta_practical: 0.15,
            window: Window {
                start: "2024-01-01".into(),
                end: "2024-06-30".into(),
            },
            cost_model_version: "costs_v3".into(),
            null_family: "block_bootstrap".into(),
            trial_budget: 20,
            gate_profile: "strict_v1".into(),
            primary_hypothesis: "1h momentum on BTC-USD survives costs".into(),
            falsifier: "sharpe_after_costs <= 0.0 on the out-of-sample window".into(),
        }
    }

    /// The embargo is computed, never typed.
    #[test]
    fn the_embargo_comes_from_the_pipeline() {
        let p = seal(draft(), &embargo()).expect("a complete draft seals");
        assert_eq!(
            p.embargo_bars,
            dataplane::split::compute_embargo(&embargo()),
            "the same arithmetic the split spec uses, so a run's embargo and its splits \
             cannot disagree"
        );
        assert!(p.embargo_bars > 4);
    }

    /// A registration with no falsifier is not a registration.
    #[test]
    fn a_hypothesis_with_no_falsifier_is_refused_and_told_why() {
        let mut d = draft();
        d.falsifier = "   ".into();
        let err = seal(d, &embargo()).expect_err("refused");
        match err {
            PreregError::Missing { field, why } => {
                assert_eq!(field, "falsifier");
                assert!(
                    why.contains("can only ever confirm"),
                    "the reason has to be in the error, not in a comment: {why}"
                );
            }
            other => panic!("{other}"),
        }
    }

    /// `delta_practical` has no default, at the boundary.
    ///
    /// The technique, not the value: a JSON body that omits it must fail to
    /// deserialise rather than arriving as zero, because a zero here is a
    /// decision nobody made about what improvement is worth acting on.
    #[test]
    fn a_draft_without_delta_practical_does_not_deserialise() {
        let body = json!({
            "run_id": Uuid::new_v4(),
            "objective": {"maximize": ["sharpe"]},
            "benchmark": {},
            "window": {"start": "2024-01-01", "end": "2024-06-30"},
            "cost_model_version": "costs_v3",
            "null_family": "block_bootstrap",
            "trial_budget": 20,
            "gate_profile": "strict_v1",
            "primary_hypothesis": "h",
            "falsifier": "f"
        });
        let err = serde_json::from_value::<Draft>(body).expect_err("no default");
        assert!(
            err.to_string().contains("delta_practical"),
            "and it names the field: {err}"
        );
    }

    /// Zero and non-finite are refused too, so the type is not the only guard.
    #[test]
    fn a_delta_of_zero_is_refused() {
        for bad in [0.0, -0.1, f64::NAN, f64::INFINITY] {
            let mut d = draft();
            d.delta_practical = bad;
            assert!(matches!(
                seal(d, &embargo()),
                Err(PreregError::BadDelta(_))
            ));
        }
    }

    /// The falsifier is in the hash.
    ///
    /// Without it a run could seal a registration, change what would have
    /// counted as failure, and leave every ticket still matching — which is the
    /// whole door this object closes.
    #[test]
    fn changing_the_falsifier_changes_the_hash() {
        let a = seal(draft(), &embargo()).unwrap();
        let mut d = draft();
        d.run_id = a.run_id;
        d.falsifier = "sharpe_after_costs <= 0.5".into();
        let b = seal(d, &embargo()).unwrap();
        assert_ne!(a.hash(), b.hash());
    }

    /// The hash does not move when a map's iteration order does.
    #[test]
    fn the_hash_is_stable_across_key_order() {
        let mut a = draft();
        a.objective = json!({"maximize": ["sharpe"], "subject_to": {"max_dd": 0.2}});
        let mut b = draft();
        b.run_id = a.run_id;
        b.objective = json!({"subject_to": {"max_dd": 0.2}, "maximize": ["sharpe"]});
        let (x, y) = (
            seal(a, &embargo()).unwrap(),
            seal(b, &embargo()).unwrap(),
        );
        assert_eq!(x.hash(), y.hash());
    }

    /// Amending cannot buy a fresh trial budget.
    ///
    /// The carry-over is the point: without it, "amend the registration" is a
    /// button that refills the thing the amendment cap exists to bound.
    #[test]
    fn an_amendment_carries_the_trial_count_over() {
        let first = seal(draft(), &embargo()).unwrap();
        let mut d = draft();
        d.run_id = first.run_id;
        d.trial_budget = 500;
        let second = amend(&first, d, &embargo(), MAX_PREREG_AMENDMENTS).unwrap();
        assert_eq!(second.trial_budget, first.trial_budget, "carried, not asked for");
        assert_eq!(second.supersedes, Some(first.id));
        assert_eq!(second.amendment, 1);
        assert_ne!(second.hash(), first.hash());
    }

    /// The third amendment blocks the run.
    #[test]
    fn a_protocol_rewritten_three_times_is_not_a_protocol() {
        let mut current = seal(draft(), &embargo()).unwrap();
        for expected in 1..MAX_PREREG_AMENDMENTS {
            let mut d = draft();
            d.run_id = current.run_id;
            current = amend(&current, d, &embargo(), MAX_PREREG_AMENDMENTS).unwrap();
            assert_eq!(current.amendment, expected);
        }
        let mut d = draft();
        d.run_id = current.run_id;
        let err = amend(&current, d, &embargo(), MAX_PREREG_AMENDMENTS)
            .expect_err("the cap bites");
        assert!(
            matches!(err, PreregError::TooManyAmendments { .. }),
            "{err}"
        );
        assert!(
            err.to_string().contains("protocol_unstable"),
            "the error names the failure class the run ends with: {err}"
        );
    }

    /// The lineage is readable from the last row.
    #[test]
    fn the_chain_reads_back_oldest_first() {
        let first = seal(draft(), &embargo()).unwrap();
        let mut d = draft();
        d.run_id = first.run_id;
        let second = amend(&first, d, &embargo(), MAX_PREREG_AMENDMENTS).unwrap();
        let all = vec![first.clone(), second.clone()];
        let chain = second.lineage(&all);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].id, first.id);
        assert_eq!(chain[1].id, second.id);
    }

    /// A backwards window is refused rather than silently reordered.
    #[test]
    fn a_backwards_window_is_refused() {
        let mut d = draft();
        d.window = Window {
            start: "2024-06-30".into(),
            end: "2024-01-01".into(),
        };
        assert!(matches!(
            seal(d, &embargo()),
            Err(PreregError::BackwardsWindow { .. })
        ));
    }
}
