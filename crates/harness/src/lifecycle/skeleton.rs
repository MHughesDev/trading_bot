//! The pre-registration skeleton a tier is handed instead of a blank page
//! (AGENT-013 §11; Set S task S-10.3).
//!
//! # What §11 actually asks for
//!
//! Three authoring modes, one per tier band. `free` is a blank form and a
//! validator. The other two are not: `skeleton_grammar` fills the form's
//! *shape* — the fields become enums and numbers — and `skeleton_confirm` fills
//! the form's *content* from the question's own constraints and asks the model
//! to confirm it.
//!
//! The reason is a measured one rather than a courtesy. A degraded tier asked to
//! produce eleven fields from prose produces something that validates: a window
//! it invented, a cost model version it half-remembered, and a falsifier that
//! restates the hypothesis in the negative. Every one of those passes
//! `check_registration`, hashes cleanly, and is stamped on every ticket the run
//! spends — so the failure is not a refusal anybody sees, it is a protocol
//! nobody can refute, carried by every piece of evidence the run produces.
//!
//! # What this fills, and what it refuses to fill
//!
//! **Filled** — the facts the question already fixes or the platform owns: the
//! window, the instrument the objective is about, the gate profile, the trial
//! budget, the null family. A run did not choose these and restating them is
//! transcription, not commitment.
//!
//! **Never filled** — `primary_hypothesis` and `falsifier`. Those are the claim.
//! A skeleton that guessed them would be the platform pre-registering on the
//! model's behalf and the model agreeing, which is not a commitment by anybody:
//! the whole object exists so that a *stated* claim can turn out wrong. They are
//! left absent, and [`Skeleton::what_you_must_write`] names them so a
//! confirming tier is told exactly which two fields are its own.
//!
//! `cost_model_version` is a fourth hole, and an honest one rather than a
//! principled one: the platform has no cost-model *registry* to name a version
//! from. The run config carries a `cost_model_ref` string and nothing resolves
//! it to a versioned object, so there is nothing here that could be filled
//! truthfully. Writing `costs_v1` would pin every result the run produces to a
//! version nobody published — so the hole is named instead, which is at least a
//! question a reader can answer.
//!
//! `delta_practical` is not filled either, and that is the sharpest case. It has
//! no platform default anywhere by design (ADR-P0-08), because a default is a
//! decision nobody made about what improvement is worth acting on — and putting
//! one in a skeleton would be worse than a default in code, since it would
//! arrive wearing the authority of something the platform computed.

use serde_json::{json, Map, Value};

use crate::profile::{PreregAuthoring, Profile};

/// A partly-filled registration, and the list of what is still the run's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    /// The fields the platform filled, as the `preregister` call's arguments.
    pub filled: Map<String, Value>,
    /// The fields deliberately left absent, in the order a caller should be
    /// asked for them.
    pub missing: Vec<&'static str>,
}

/// The fields a skeleton never fills, whatever the tier.
///
/// Three, and each for its own reason: two are the claim, and one has no
/// platform default by design.
pub const NEVER_FILLED: &[&str] = &["primary_hypothesis", "falsifier", "delta_practical"];

impl Skeleton {
    /// One line per field the run still has to write, with why it is theirs.
    ///
    /// Handed to the model beside the skeleton. A confirming tier told "confirm
    /// this" and given a form with three holes will fill the holes with
    /// something; told *which* three and why, it fills them with its own claim,
    /// which is the only thing worth having.
    #[must_use]
    pub fn what_you_must_write(&self) -> Vec<(&'static str, &'static str)> {
        self.missing
            .iter()
            .map(|f| {
                let why = match *f {
                    "primary_hypothesis" => {
                        "your claim, in one sentence, stated so it could be wrong. The platform \
                         cannot write this: a hypothesis nobody stated cannot turn out false."
                    }
                    "falsifier" => {
                        "the result that would refute it. Written before the first trial or not \
                         at all — this is the field the whole registration exists for."
                    }
                    "delta_practical" => {
                        "the smallest improvement that would change a decision. There is no \
                         platform default and there will not be one: a default here is a \
                         decision nobody made about what is worth acting on, inherited \
                         silently by everything after it."
                    }
                    "trial_budget" => {
                        concat!(
                            "how many trials this run may spend. No profile declares a ",
                            "per-tier cap, so there is no number the platform owns — and one ",
                            "written here would set how much of the scarcest budget on this ",
                            "platform you may use, on nobody's authority.",
                        )
                    }
                    "cost_model_version" => {
                        concat!(
                            "the costs this result will be judged under. The platform ",
                            "cannot fill it: there is no cost model registry to name a ",
                            "version from, and a version nobody published pins every ",
                            "result you produce to nothing.",
                        )
                    }
                    _ => "the run's to state",
                };
                (*f, why)
            })
            .collect()
    }

    /// Whether anything was filled at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filled.is_empty()
    }
}

/// Build the skeleton for a run, or `None` when the profile authors freely.
///
/// `None` is not "an empty skeleton": a free-authoring tier is handed nothing
/// and writes the whole registration, and returning an empty map would make the
/// caller attach a `{}` that reads as a platform statement about the run.
///
/// `constraints` is the run's own constraint object — the instrument, timeframe
/// and window the caller pinned. Anything absent from it stays absent here; a
/// skeleton that invented a window would be pre-registering over a period nobody
/// chose, which is the error it exists to prevent.
#[must_use]
pub fn for_run(
    profile: &Profile,
    constraints: &Value,
    cost_model_version: Option<&str>,
    gate_profile: &str,
    trial_budget: Option<u32>,
) -> Option<Skeleton> {
    if profile.prereg_authoring == PreregAuthoring::Free {
        return None;
    }
    let text = |k: &str| {
        constraints
            .get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };

    let mut filled = Map::new();
    let mut missing: Vec<&'static str> = NEVER_FILLED.to_vec();

    if let (Some(start), Some(end)) = (text("start"), text("end")) {
        filled.insert("window".into(), json!({ "start": start, "end": end }));
    }
    // The objective's *subject* comes from the question; its content does not.
    // `maximize` is the platform's standing answer to "better at what" for this
    // asset class and is the same for every run — a run that wants a different
    // one is a run authoring freely, and this tier is not.
    if let Some(instrument) = text("instrument_id") {
        filled.insert(
            "objective".into(),
            json!({
                "maximize": ["sharpe_after_costs"],
                "subject_to": { "max_drawdown": 0.2 },
                "instrument_id": instrument,
            }),
        );
        filled.insert(
            "benchmark".into(),
            json!({ "family": "buy_and_hold", "instrument_id": instrument }),
        );
    }
    // `Option`, and absent when the caller has none. The platform has no cost
    // model *registry* to name a version from — the run config carries a
    // `cost_model_ref` string and nothing resolves it to a versioned object —
    // so there is nothing here the platform could honestly fill. Naming the
    // hole is the whole difference: a skeleton that wrote `costs_v1` would pin
    // every result the run produces to a version nobody published.
    match cost_model_version {
        Some(v) => {
            filled.insert("cost_model_version".into(), json!(v));
        }
        None => missing.push("cost_model_version"),
    }
    filled.insert("gate_profile".into(), json!(gate_profile));
    // The same treatment, and the same reason: no profile declares a per-tier
    // trial cap, so there is no number here the platform owns. A skeleton that
    // wrote one would be setting how much of the platform's scarcest budget this
    // run may spend, on nobody's authority.
    match trial_budget {
        Some(n) => {
            filled.insert("trial_budget".into(), json!(n));
        }
        None => missing.push("trial_budget"),
    }
    // Chosen before the result or the comparison is whichever null flatters it,
    // and a confirming tier has no basis to choose among them mid-run.
    filled.insert("null_family".into(), json!("block_bootstrap"));

    Some(Skeleton { filled, missing })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{frontier_fixture, PreregAuthoring};

    fn constraints() -> Value {
        json!({
            "instrument_id": "BTC-USD",
            "timeframe": "1h",
            "start": "2020-01-01",
            "end": "2024-12-31"
        })
    }

    fn confirming() -> Profile {
        let mut p = frontier_fixture();
        p.prereg_authoring = PreregAuthoring::SkeletonConfirm;
        p
    }

    /// A free-authoring tier is handed nothing, and that is distinct from being
    /// handed an empty form.
    #[test]
    fn a_free_authoring_tier_gets_no_skeleton() {
        let p = frontier_fixture();
        assert_eq!(p.prereg_authoring, PreregAuthoring::Free);
        assert!(for_run(&p, &constraints(), Some("costs_v3"), "crypto_v2", Some(32)).is_none());
    }

    /// The claim is never filled. This is the test the whole module exists for:
    /// a skeleton that guessed a falsifier would produce a registration nobody
    /// committed to, which still hashes and still stamps every ticket.
    #[test]
    fn the_claim_is_never_filled() {
        let s = for_run(&confirming(), &constraints(), Some("costs_v3"), "crypto_v2", Some(32))
            .expect("a confirming tier gets one");
        for field in NEVER_FILLED {
            assert!(
                !s.filled.contains_key(*field),
                "the skeleton filled `{field}`, which is the run's to state"
            );
            assert!(s.missing.contains(field));
        }
        // And each one is named with its reason, so "confirm this" tells the
        // model which three holes are its own.
        let owed = s.what_you_must_write();
        assert_eq!(owed.len(), NEVER_FILLED.len());
        assert!(owed.iter().all(|(_, why)| !why.is_empty()));
    }

    /// `delta_practical` in particular. It has no default in code by design, and
    /// a skeleton is a worse place for one than code: it would arrive looking
    /// like something the platform computed.
    #[test]
    fn delta_practical_has_no_default_here_either() {
        let s = for_run(&confirming(), &constraints(), Some("costs_v3"), "crypto_v2", Some(32))
            .expect("a skeleton");
        assert!(!s.filled.contains_key("delta_practical"));
        assert!(s.missing.contains(&"delta_practical"));
    }

    /// What the question fixed is transcribed; what it did not is left out.
    #[test]
    fn an_absent_constraint_is_absent_from_the_skeleton() {
        let s = for_run(&confirming(), &constraints(), Some("costs_v3"), "crypto_v2", Some(32))
            .expect("a skeleton");
        assert_eq!(s.filled["window"]["start"], "2020-01-01");
        assert_eq!(s.filled["objective"]["instrument_id"], "BTC-USD");
        assert_eq!(s.filled["gate_profile"], "crypto_v2");
        assert_eq!(s.filled["trial_budget"], 32);
        // And when the caller has no budget to give, the hole is named rather
        // than filled.
        let no_budget = for_run(&confirming(), &constraints(), None, "crypto_v2", None)
            .expect("a skeleton");
        assert!(!no_budget.filled.contains_key("trial_budget"));
        assert!(no_budget.missing.contains(&"trial_budget"));
        assert!(no_budget.missing.contains(&"cost_model_version"));
        assert!(no_budget
            .what_you_must_write()
            .iter()
            .all(|(_, why)| !why.is_empty()));

        // A run with no window pinned gets no window: inventing one would
        // pre-register over a period nobody chose.
        let bare = for_run(&confirming(), &json!({}), Some("costs_v3"), "crypto_v2", Some(32))
            .expect("a skeleton");
        assert!(!bare.filled.contains_key("window"));
        assert!(!bare.filled.contains_key("objective"));
        // What the platform owns is still filled, because it is the platform's
        // answer and not a guess about the question.
        assert_eq!(bare.filled["cost_model_version"], "costs_v3");
    }

    /// Both non-free modes get one. `skeleton_grammar` constrains the form's
    /// shape and `skeleton_confirm` its content, and a tier under a grammar
    /// still benefits from not retyping the window.
    #[test]
    fn both_constrained_modes_get_a_skeleton() {
        for mode in [
            PreregAuthoring::SkeletonGrammar,
            PreregAuthoring::SkeletonConfirm,
        ] {
            let mut p = frontier_fixture();
            p.prereg_authoring = mode;
            assert!(
                for_run(&p, &constraints(), Some("costs_v3"), "crypto_v2", Some(32)).is_some(),
                "{} got no skeleton",
                mode.as_str()
            );
        }
    }

    /// Every filled field is one `check_registration` accepts.
    ///
    /// A skeleton naming a field the tool refuses would hand a confirming tier a
    /// form that cannot be submitted — and the model would be blamed for it.
    #[test]
    fn every_filled_field_is_one_the_registration_takes() {
        const ACCEPTED: &[&str] = &[
            "objective",
            "benchmark",
            "delta_practical",
            "window",
            "cost_model_version",
            "null_family",
            "trial_budget",
            "gate_profile",
            "primary_hypothesis",
            "falsifier",
            "supersedes",
        ];
        let s = for_run(&confirming(), &constraints(), Some("costs_v3"), "crypto_v2", Some(32))
            .expect("a skeleton");
        for key in s.filled.keys() {
            assert!(
                ACCEPTED.contains(&key.as_str()),
                "the skeleton fills `{key}`, which a registration has no field for"
            );
        }
    }
}
