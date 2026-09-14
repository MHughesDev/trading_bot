//! Campaigns and the dispatcher that draws from them (SPEC §8, §10, §4.5;
//! INV-20, INV-21; checklist 2.1 and 2.9).
//!
//! A campaign is the DEFINE phase made unskippable. Its facts — the hypothesis,
//! the objective, the benchmark, the practical effect size, the budget, the gate
//! profile and the exploration floor — are stated once, before anything runs,
//! and are immutable afterwards (`mlops.campaign` refuses UPDATE and DELETE).
//! `delta_practical` has no default: a campaign that has not said what size of
//! improvement would matter cannot be defined, because deciding that *after*
//! seeing results is the post-hoc rationalization pre-registration exists to
//! prevent.
//!
//! ## The exploration floor
//!
//! §4.5 requires at least 5% of a campaign's dispatches to be uniform-random
//! draws, and INV-21 adds that the floor is "not writable by an agent and not
//! exposed in any tool schema". Three mechanisms, because a comment is not one:
//!
//! * [`ExplorationFloor`] cannot be constructed below [`MIN_EXPLORATION_FLOOR`] —
//!   `new` returns an error, and there is no other constructor.
//! * The floor is a field of the *campaign*, set at DEFINE and immutable. A
//!   `CHECK` on `mlops.campaign` refuses a row below 0.05 independently of this
//!   code.
//! * [`Dispatcher::draw`] is the only thing that turns a candidate set into a
//!   choice, it takes no floor argument, and it decides exploration itself from
//!   the achieved fraction on the ledger. There is no parameter for a caller —
//!   agent or human — to pass.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Decision, DecisionKind, DecisionLog, DecisionTier, DispatchContext, LedgerError};

/// The floor below which no campaign may be defined (§4.5, INV-21).
pub const MIN_EXPLORATION_FLOOR: f64 = 0.05;

/// A fraction of dispatches reserved for uniform-random draws.
///
/// Sealed: the only constructor refuses anything below [`MIN_EXPLORATION_FLOOR`],
/// so "an exploration floor below the floor" is unrepresentable rather than
/// merely discouraged. The same pattern as `TrialTicket` and `NEff`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ExplorationFloor(f64);

impl ExplorationFloor {
    /// The platform default. Campaigns that do not raise it get this.
    #[must_use]
    pub fn default_floor() -> Self {
        Self(MIN_EXPLORATION_FLOOR)
    }

    /// Raise the floor. Lowering it is not an operation.
    ///
    /// # Errors
    /// Anything below [`MIN_EXPLORATION_FLOOR`], above 1.0, or non-finite.
    pub fn new(fraction: f64) -> Result<Self, LedgerError> {
        if !fraction.is_finite() || !(MIN_EXPLORATION_FLOOR..=1.0).contains(&fraction) {
            return Err(LedgerError::Invalid(format!(
                "exploration floor must be in [{MIN_EXPLORATION_FLOOR}, 1.0]; {fraction} would let \
                 the platform stop looking at what it has not tried"
            )));
        }
        Ok(Self(fraction))
    }

    #[must_use]
    pub fn value(self) -> f64 {
        self.0
    }
}

impl Default for ExplorationFloor {
    fn default() -> Self {
        Self::default_floor()
    }
}

/// The budget a campaign may not exceed. Every field is required: a budget with
/// an unstated dimension is not a budget (§8).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub max_trials: i64,
    pub gpu_hours: f64,
    pub usd: f64,
    pub wall_clock_hours: f64,
}

/// What a campaign is trying to do, stated before it starts.
///
/// `delta_practical` has **no default** and must be positive: the campaign has
/// to say what size of improvement would actually matter before it is allowed to
/// look for one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CampaignDefinition {
    pub slug: String,
    pub hypothesis: String,
    /// `{"maximize": [...], "subject_to": {...}}` — a vector and its constraints,
    /// never a scalar score (ADR-012).
    pub objective: serde_json::Value,
    pub benchmark: serde_json::Value,
    /// REQUIRED, positive. The smallest improvement that would change a decision.
    pub delta_practical: f64,
    /// REQUIRED, non-negative. Spending above this in one action needs a human
    /// approval (§15, ADR-P2-20). **No default**, for the same reason
    /// `delta_practical` has none: a platform-wide default would be a decision
    /// nobody made about somebody's money, and once one exists every campaign
    /// inherits it silently. `0.0` is a legitimate and meaningful value — it
    /// means every spend is approved by a human — but it has to be written down.
    pub approval_spend_usd: f64,
    pub budget: Budget,
    #[serde(default)]
    pub exploration_floor: ExplorationFloor,
    /// The versioned gate profile this campaign is judged under (INV-23).
    pub gates_profile: String,
    #[serde(default)]
    pub preference_vector: Option<serde_json::Value>,
    #[serde(default)]
    pub search_space: serde_json::Value,
}

impl CampaignDefinition {
    /// Structural checks that hold before any backend is involved. The database
    /// repeats every one of them; this is the half that fails fast and says why.
    ///
    /// # Errors
    /// A missing or non-positive `delta_practical`, an empty hypothesis or slug,
    /// a scalarized objective, or a non-positive budget dimension.
    pub fn validate(&self) -> Result<(), LedgerError> {
        let invalid = |m: &str| Err(LedgerError::Invalid(m.to_string()));
        if self.slug.trim().is_empty() {
            return invalid("a campaign needs a slug");
        }
        if self.hypothesis.trim().is_empty() {
            return invalid(
                "a campaign needs a written hypothesis: DEFINE is where the claim is made, and an \
                 unstated claim cannot be wrong",
            );
        }
        if !self.approval_spend_usd.is_finite() || self.approval_spend_usd < 0.0 {
            return invalid(
                "approval_spend_usd is REQUIRED and must be non-negative (§15): a campaign has to                  state how much it may spend before a human is asked, and 0 — ask about                  everything — is an answer, while an unstated threshold is not",
            );
        }
        if !self.delta_practical.is_finite() || self.delta_practical <= 0.0 {
            return invalid(
                "delta_practical is REQUIRED and must be positive (§10): deciding what improvement \
                 would matter after seeing the results is the thing pre-registration prevents",
            );
        }
        let maximize = self.objective.get("maximize").and_then(|m| m.as_array());
        if maximize.is_none_or(Vec::is_empty) {
            return invalid(
                "objective must name at least one thing to maximize (ADR-012: there is no scalar \
                 score, so selection needs an explicit objective and its constraints)",
            );
        }
        if self.objective.get("subject_to").is_none() {
            return invalid("objective must state its constraints as `subject_to`");
        }
        let b = &self.budget;
        if b.max_trials <= 0
            || !b.gpu_hours.is_finite()
            || b.gpu_hours < 0.0
            || !b.usd.is_finite()
            || b.usd < 0.0
            || !b.wall_clock_hours.is_finite()
            || b.wall_clock_hours <= 0.0
        {
            return invalid("every budget dimension must be stated and non-negative (§8)");
        }
        if self.gates_profile.trim().is_empty() {
            return invalid("a campaign must name the gate profile it is judged under (INV-23)");
        }
        Ok(())
    }

    /// The hash the campaign's DEFINE facts are locked by.
    ///
    /// # Panics
    /// Never in practice: every field is JSON-representable.
    #[must_use]
    pub fn define_hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("campaign definition serializes");
        format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
    }
}

/// A defined campaign.
///
/// Sealed like [`crate::TrialTicket`]: only a ledger's `define_campaign` mints
/// one, so "dispatch against a campaign that was never defined" is not something
/// a caller can express.
#[derive(Clone, Debug, PartialEq)]
pub struct CampaignHandle {
    campaign_id: Uuid,
    tenant_id: String,
    definition: CampaignDefinition,
    define_hash: String,
}

impl CampaignHandle {
    /// Crate-internal: the ledger mints these and nothing else does.
    pub(crate) fn seal(campaign_id: Uuid, tenant_id: String, definition: CampaignDefinition) -> Self {
        let define_hash = definition.define_hash();
        Self {
            campaign_id,
            tenant_id,
            definition,
            define_hash,
        }
    }

    #[must_use]
    pub fn campaign_id(&self) -> Uuid {
        self.campaign_id
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub fn definition(&self) -> &CampaignDefinition {
        &self.definition
    }

    #[must_use]
    pub fn define_hash(&self) -> &str {
        &self.define_hash
    }

    #[must_use]
    pub fn exploration_floor(&self) -> ExplorationFloor {
        self.definition.exploration_floor
    }

    /// A dispatch context scoped to this campaign, carrying the declared effect
    /// size. The caller cannot supply a different one.
    #[must_use]
    pub fn dispatch_context(&self, actor: crate::ActorKind, actor_id: impl Into<String>) -> DispatchContext {
        let actor_id = actor_id.into();
        DispatchContext {
            tenant_id: self.tenant_id.clone(),
            campaign_id: Some(self.campaign_id),
            experiment_id: None,
            actor_kind: actor,
            actor_id: actor_id.clone(),
            on_behalf_of: None,
            policy_id: POLICY_ID.to_string(),
            policy_version: POLICY_VERSION,
            delta_practical: Some(self.definition.delta_practical),
        }
    }
}

/// The dispatcher's policy identity, logged on every decision it makes.
pub const POLICY_ID: &str = "campaign_dispatcher";
pub const POLICY_VERSION: i32 = 1;

/// One draw: what was chosen, with what probability, and whether it was forced
/// exploration.
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub decision_id: Uuid,
    pub chosen_index: usize,
    /// `p(this candidate | context, policy)`. Never `None` — a deterministic
    /// logging policy makes off-policy evaluation formally impossible (§4.2·6).
    pub propensity: f64,
    pub exploration_flag: bool,
    pub candidate_set_hash: String,
}

/// Turns a candidate set into a logged choice.
///
/// It takes **no floor argument**. The floor comes from the campaign, the
/// achieved fraction comes from the ledger, and the decision between them is
/// made here. There is nothing for an agent to pass and nothing for a tool
/// schema to expose (INV-21).
pub struct Dispatcher<'a, L: DecisionLog + ?Sized> {
    campaign: &'a CampaignHandle,
    log: &'a L,
}

impl<'a, L: DecisionLog + ?Sized> Dispatcher<'a, L> {
    #[must_use]
    pub fn new(campaign: &'a CampaignHandle, log: &'a L) -> Self {
        Self { campaign, log }
    }

    /// Choose one candidate, log the decision, and report the propensity.
    ///
    /// `achieved` is the campaign's exploration fraction so far, read from the
    /// ledger by the caller (`TrialLedger::exploration_fraction`). When it is
    /// below the floor this draw is forced to be uniform-random — which is the
    /// floor doing its job, and is exactly the behaviour §4.5 says must not be
    /// optimized away.
    ///
    /// `preference` is the policy's score per candidate, higher is better. It is
    /// advisory: on an exploration draw it is ignored, and on an exploitation
    /// draw it still yields a *stochastic* choice, because a deterministic argmax
    /// has propensity 1.0 for one candidate and 0 for the rest, and a propensity
    /// of 0 makes every off-policy estimator divide by zero.
    ///
    /// # Errors
    /// An empty candidate set, or a backend failure writing the decision.
    pub fn draw(
        &self,
        kind: DecisionKind,
        candidates: &[serde_json::Value],
        preference: &[f64],
        achieved: f64,
        rng: &mut impl FnMut() -> f64,
        rationale: Option<String>,
    ) -> Result<Draw, LedgerError> {
        if candidates.is_empty() {
            return Err(LedgerError::Invalid(
                "a decision needs a candidate set: the rejected options are the evidence".into(),
            ));
        }
        let n = candidates.len();
        let floor = self.campaign.exploration_floor().value();
        let explore = achieved < floor || rng() < floor;

        let weights: Vec<f64> = if explore {
            vec![1.0 / n as f64; n]
        } else {
            softmax(preference, n)
        };
        let chosen_index = sample(&weights, rng());
        let propensity = weights[chosen_index];

        let candidate_set_hash = hash_candidates(candidates);
        let ctx = self
            .campaign
            .dispatch_context(crate::ActorKind::Scheduler, POLICY_ID);
        let decision = Decision {
            kind,
            context_hash: self.campaign.define_hash().to_string(),
            context_uri: None,
            candidate_set: candidates.to_vec(),
            chosen: candidates[chosen_index].clone(),
            propensity: Some(propensity),
            exploration_flag: explore,
            // The dispatcher is an analytic rule, and the cold-start ladder
            // records which tier produced each decision (§13.1).
            decision_tier: DecisionTier::Rule,
            rationale,
        };
        let decision_id = self.log.log_decision(&ctx, &decision)?;

        Ok(Draw {
            decision_id,
            chosen_index,
            propensity,
            exploration_flag: explore,
            candidate_set_hash,
        })
    }

    /// Whether the campaign is below its own floor — the condition §4.5 says to
    /// alarm on. Reported, never silently corrected: a run of exploitation draws
    /// that pushes the fraction under the floor is information about the
    /// dispatcher, not something to paper over.
    #[must_use]
    pub fn below_floor(&self, achieved: f64) -> bool {
        achieved < self.campaign.exploration_floor().value()
    }
}

/// Softmax over the policy's preferences, with a floor on every weight so no
/// candidate can have propensity 0. Missing or non-finite preferences are 0.
fn softmax(preference: &[f64], n: usize) -> Vec<f64> {
    /// No candidate in the set may be unreachable: a propensity of 0 is a
    /// division by zero in every off-policy estimator that later reads the log.
    const MIN_WEIGHT: f64 = 1e-3;

    let scores: Vec<f64> = (0..n)
        .map(|i| preference.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0))
        .collect();
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exp: Vec<f64> = scores.iter().map(|s| (s - max).exp()).collect();
    let total: f64 = exp.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return vec![1.0 / n as f64; n];
    }
    let floor_mass = MIN_WEIGHT * n as f64;
    let scale = (1.0 - floor_mass).max(0.0);
    let mut w: Vec<f64> = exp.iter().map(|e| MIN_WEIGHT + scale * e / total).collect();
    // Renormalize against accumulated float error so the weights are a
    // distribution, not nearly one.
    let sum: f64 = w.iter().sum();
    for x in &mut w {
        *x /= sum;
    }
    w
}

/// Inverse-CDF sample from `weights` using `u ∈ [0, 1)`.
fn sample(weights: &[f64], u: f64) -> usize {
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        acc += w;
        if u < acc {
            return i;
        }
    }
    weights.len() - 1
}

/// Content hash of the whole candidate set, in the order it was considered.
fn hash_candidates(candidates: &[serde_json::Value]) -> String {
    let bytes = serde_json::to_vec(candidates).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryLedger;

    fn definition() -> CampaignDefinition {
        CampaignDefinition {
            slug: "mean-reversion-q3".into(),
            hypothesis: "intraday mean reversion survives costs on majors".into(),
            objective: serde_json::json!({
                "maximize": ["sharpe_net"],
                "subject_to": { "max_dd": 0.2 }
            }),
            benchmark: serde_json::json!({ "kind": "buy_and_hold" }),
            delta_practical: 0.25,
            approval_spend_usd: 50.0,
            budget: Budget {
                max_trials: 500,
                gpu_hours: 10.0,
                usd: 100.0,
                wall_clock_hours: 48.0,
            },
            exploration_floor: ExplorationFloor::default_floor(),
            gates_profile: "strict_v1".into(),
            preference_vector: None,
            search_space: serde_json::json!({}),
        }
    }

    fn handle() -> CampaignHandle {
        CampaignHandle::seal(Uuid::new_v4(), "t".into(), definition())
    }

    /// A deterministic stand-in for a uniform RNG, cycling a fixed sequence so a
    /// test's verdict does not depend on the day it ran.
    fn rng_over(values: Vec<f64>) -> impl FnMut() -> f64 {
        let mut i = 0usize;
        move || {
            let v = values[i % values.len()];
            i += 1;
            v
        }
    }

    // ------------------------------------------------------------------ //
    // 2.1 — DEFINE refuses to start without delta_practical
    // ------------------------------------------------------------------ //

    #[test]
    fn a_campaign_without_a_practical_effect_size_cannot_be_defined() {
        for bad in [0.0, -0.1, f64::NAN] {
            let mut d = definition();
            d.delta_practical = bad;
            assert!(d.validate().is_err(), "delta_practical = {bad} was accepted");
        }
        assert!(definition().validate().is_ok());
    }

    #[test]
    fn a_campaign_must_state_a_hypothesis_an_objective_and_a_budget() {
        let mut no_hypothesis = definition();
        no_hypothesis.hypothesis = "   ".into();
        assert!(no_hypothesis.validate().is_err());

        let mut scalarized = definition();
        scalarized.objective = serde_json::json!({ "score": 1.0 });
        assert!(
            scalarized.validate().is_err(),
            "a scalar score is not an objective (ADR-012)"
        );

        let mut unconstrained = definition();
        unconstrained.objective = serde_json::json!({ "maximize": ["sharpe_net"] });
        assert!(unconstrained.validate().is_err(), "constraints are required");

        let mut no_budget = definition();
        no_budget.budget.max_trials = 0;
        assert!(no_budget.validate().is_err());

        let mut no_profile = definition();
        no_profile.gates_profile = String::new();
        assert!(no_profile.validate().is_err());
    }

    #[test]
    fn the_define_hash_covers_every_declared_fact() {
        let base = definition().define_hash();
        for mutate in [
            (|d: &mut CampaignDefinition| d.delta_practical = 0.26) as fn(&mut CampaignDefinition),
            |d| d.hypothesis = "something else".into(),
            |d| d.budget.max_trials = 501,
            |d| d.gates_profile = "strict_v2".into(),
            |d| d.exploration_floor = ExplorationFloor::new(0.10).unwrap(),
        ] {
            let mut d = definition();
            mutate(&mut d);
            assert_ne!(base, d.define_hash());
        }
    }

    // ------------------------------------------------------------------ //
    // 2.9 — the exploration floor
    // ------------------------------------------------------------------ //

    /// INV-21: a floor below 5% is unrepresentable, not merely refused at the
    /// edges. There is no other constructor.
    #[test]
    fn an_exploration_floor_below_five_percent_cannot_be_constructed() {
        for bad in [0.0, 0.01, 0.049_999, -1.0, 1.5, f64::NAN] {
            assert!(ExplorationFloor::new(bad).is_err(), "{bad} was accepted");
        }
        assert!((ExplorationFloor::default_floor().value() - 0.05).abs() < f64::EPSILON);
        assert!(ExplorationFloor::new(0.05).is_ok());
        assert!(ExplorationFloor::new(0.30).is_ok());
    }

    /// Below the floor, the next draw is uniform-random whatever the policy
    /// prefers. This is the floor doing the one thing it exists to do.
    #[test]
    fn a_campaign_under_its_floor_is_forced_to_explore() {
        let led = InMemoryLedger::new();
        let c = handle();
        let d = Dispatcher::new(&c, &led);
        let candidates: Vec<serde_json::Value> = (0..4).map(|i| serde_json::json!({ "i": i })).collect();
        // The policy is certain candidate 0 is best; the achieved fraction is 0.
        let preference = vec![100.0, 0.0, 0.0, 0.0];
        let mut rng = rng_over(vec![0.99, 0.80]);
        let draw = d
            .draw(DecisionKind::Select, &candidates, &preference, 0.0, &mut rng, None)
            .expect("draw");
        assert!(draw.exploration_flag);
        assert!(
            (draw.propensity - 0.25).abs() < 1e-12,
            "a uniform draw over 4 candidates has propensity 0.25, got {}",
            draw.propensity
        );
        assert_eq!(draw.chosen_index, 3, "the policy's favourite was not forced");
        assert!(d.below_floor(0.0));
    }

    /// Above the floor the policy leads — but never deterministically, because a
    /// propensity of 0 breaks every off-policy estimator that reads the log.
    #[test]
    fn an_exploitation_draw_is_still_stochastic_and_no_candidate_is_unreachable() {
        let led = InMemoryLedger::new();
        let c = handle();
        let d = Dispatcher::new(&c, &led);
        let candidates: Vec<serde_json::Value> = (0..3).map(|i| serde_json::json!({ "i": i })).collect();
        let preference = vec![10.0, 0.0, 0.0];
        let mut rng = rng_over(vec![0.99, 0.0]);
        let draw = d
            .draw(DecisionKind::Select, &candidates, &preference, 0.50, &mut rng, None)
            .expect("draw");
        assert!(!draw.exploration_flag);
        assert!(draw.propensity > 0.0 && draw.propensity < 1.0, "{}", draw.propensity);
        assert!(!d.below_floor(0.50));

        // Every candidate keeps a reachable weight.
        let w = softmax(&preference, 3);
        assert!(w.iter().all(|x| *x > 0.0));
        assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    /// Even above the floor, the floor still fires as a random reservation — it
    /// is a floor on the *fraction*, not a trigger that switches off once met.
    #[test]
    fn the_floor_still_reserves_draws_above_the_achieved_fraction() {
        let led = InMemoryLedger::new();
        let c = handle();
        let d = Dispatcher::new(&c, &led);
        let candidates: Vec<serde_json::Value> = (0..4).map(|i| serde_json::json!({ "i": i })).collect();
        // u = 0.01 < floor 0.05 ⇒ this draw is an exploration draw.
        let mut rng = rng_over(vec![0.01, 0.5]);
        let draw = d
            .draw(DecisionKind::Select, &candidates, &[9.0, 0.0, 0.0, 0.0], 0.90, &mut rng, None)
            .expect("draw");
        assert!(draw.exploration_flag);
    }

    #[test]
    fn a_decision_with_no_candidates_is_refused() {
        let led = InMemoryLedger::new();
        let c = handle();
        let d = Dispatcher::new(&c, &led);
        let mut rng = rng_over(vec![0.5]);
        assert!(d
            .draw(DecisionKind::Select, &[], &[], 0.0, &mut rng, None)
            .is_err());
    }

    /// Every draw is logged with its candidate set, propensity and exploration
    /// flag (INV-20). The dispatcher has no path that chooses without logging.
    #[test]
    fn every_draw_is_logged_with_its_candidate_set_and_propensity() {
        let led = InMemoryLedger::new();
        let c = handle();
        let d = Dispatcher::new(&c, &led);
        let candidates: Vec<serde_json::Value> = (0..3).map(|i| serde_json::json!({ "i": i })).collect();
        let mut rng = rng_over(vec![0.9, 0.1]);
        let draw = d
            .draw(
                DecisionKind::Propose,
                &candidates,
                &[1.0, 1.0, 1.0],
                0.5,
                &mut rng,
                Some("because".into()),
            )
            .expect("draw");
        let logged = led.decisions();
        assert_eq!(logged.len(), 1);
        let rec = &logged[0];
        assert_eq!(rec.candidate_set.len(), 3);
        assert_eq!(rec.propensity, Some(draw.propensity));
        assert_eq!(rec.exploration_flag, draw.exploration_flag);
        assert_eq!(rec.chosen, candidates[draw.chosen_index]);
        assert_eq!(rec.rationale.as_deref(), Some("because"));
        assert!(draw.candidate_set_hash.starts_with("sha256:"));
    }

    /// A campaign-scoped context carries the campaign's declared effect size, so
    /// a dispatch cannot quietly use a different one.
    #[test]
    fn the_dispatch_context_carries_the_campaigns_declared_effect_size() {
        let c = handle();
        let ctx = c.dispatch_context(crate::ActorKind::Agent, "agent-1");
        assert_eq!(ctx.campaign_id, Some(c.campaign_id()));
        assert_eq!(ctx.delta_practical, Some(0.25));
        assert_eq!(ctx.policy_id, POLICY_ID);
    }
}
