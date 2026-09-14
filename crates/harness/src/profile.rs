//! Capability profiles (harness guide §1.2).
//!
//! One harness, parameterised. The profile is the only place that knows a model is
//! a frontier model or a 7B local one, and every constraint in it is enforced by
//! harness code somewhere — `every_profile_field_is_enforced_somewhere` in
//! `tests/conformance.rs` is what keeps that true, because **a profile value that
//! nothing enforces is a bug**: it reads as a guarantee and behaves as a comment.
//!
//! The frontier profile is the permissive case of this schema, not a different code
//! path. That is the whole point — a system with a "local mode" bolted onto a "cloud
//! mode" has two behaviours to test and one of them is always stale.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How much the harness may trust a model to hold together.
///
/// Ordered: `Frontier` is the most permissive. An unknown model resolves to
/// `LocalSmall` (§1.2) — the conservative default is the one that fails visibly
/// rather than the one that fails subtly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Hardware that cannot run the reference model at all (D-18).
    ///
    /// Not a capability band like the others — a statement that this machine is
    /// below the deployment target. It exists so the shortfall is explicit and
    /// isolated: a degraded host runs what it can and **refuses** what it cannot,
    /// rather than quietly reshaping the design for every other host.
    Degraded,
    LocalSmall,
    LocalMid,
    LocalHigh,
    Frontier,
}

impl Tier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Degraded => "degraded",
            Tier::LocalSmall => "local_small",
            Tier::LocalMid => "local_mid",
            Tier::LocalHigh => "local_high",
            Tier::Frontier => "frontier",
        }
    }

    /// Whether this tier runs a locally hosted model.
    ///
    /// Drives the requirements the guide makes unconditional for local inference:
    /// constrained decoding, the startup canary, the native chat template.
    #[must_use]
    pub fn is_local(self) -> bool {
        !matches!(self, Tier::Frontier)
    }
}

/// What kind of agent this is (guide §9). Drives environment, security and
/// lifecycle decisions, so it is declared rather than inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Archetype {
    /// A1 — conversational, user present.
    ChatAssistant,
    /// A2 — fixed business process with model-powered steps.
    WorkflowAgent,
    /// A3 — operates on a workspace of files over many steps.
    KnowledgeWorker,
    /// A4 — acts on GUIs and external apps.
    ComputerUse,
    /// A5 — long-running, async, user absent.
    Background,
    /// A6 — orchestrator plus sub-agents.
    MultiAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudget {
    /// What the provider advertises. Recorded for the gap, never used as a limit.
    pub advertised_tokens: u32,
    /// The HARD cap the Context Manager enforces (§4.1).
    ///
    /// Deliberately far below the advertised window at every tier. Reasoning quality
    /// degrades well before a window is full, so the advertised number is a
    /// capacity, not a working budget.
    pub effective_budget_tokens: u32,
    pub reserve_for_output: u32,
    /// Per-tool-result cap before it enters context (§2.3).
    ///
    /// A tool that returns a megabyte has not given the model information, it has
    /// spent the step's budget on one answer.
    pub max_tool_result_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaStyle {
    /// Full JSON Schema: nesting, formats, long descriptions.
    Rich,
    /// Flattened: top-level primitives, enums, one-line descriptions (§2.4).
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    /// Expose the whole catalogue. Only legal when it fits the budget.
    None,
    /// Route to a namespace before each call (§2.2).
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    /// Simultaneous exposure cap (§2.1). The catalogue may be any size.
    pub max_exposed_per_step: usize,
    pub schema_style: SchemaStyle,
    pub routing: Routing,
    pub parallel_calls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputPolicy {
    /// Grammar/schema-constrained decoding. Required for every local tier (§3.1).
    pub constrained_decoding: bool,
    /// Use the model's own trained chat and tool-call template (§3.3).
    ///
    /// There is no legitimate `false`: a custom "respond in this XML" format
    /// destroys reliability faster than parameter count does. It is a field rather
    /// than an assumption so that a profile asserting otherwise fails validation
    /// loudly instead of being expressed as a silently wrong adapter.
    pub native_tool_template: bool,
    pub temperature_tool_calls: f32,
    pub max_retries_per_call: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The model manages its own plan (frontier).
    Freeform,
    /// Harness-owned plan, minimal per-step context (§5.2).
    PlannerExecutor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Orchestration {
    pub mode: Mode,
    pub max_steps: u32,
    pub few_shot_examples: u32,
    /// Self-critique steps. Off for local tiers: they burn steps and local models
    /// rarely self-correct productively — deterministic checks are the better
    /// feedback sensor (§5.4).
    pub reflection: bool,
    /// Sub-agent recursion depth (§13.3). 1 unless an eval proves the need.
    pub max_subagent_depth: u32,
    /// Free-form code execution, tiered per §11.
    pub code_execution: CodeExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeExecution {
    /// Narrow tools only. Writing correct multi-step programs is exactly where small
    /// local models are weakest.
    Denied,
    /// Single-purpose scripts with templates in the charter.
    Templated,
    /// Full programmatic tool calling.
    Full,
}

/// The three legs of the guide's trifecta rule (§14.1).
///
/// Recorded per agent, in config, because an agent holding all three is exploitable
/// and the audit has to be a reviewable artifact rather than a belief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trifecta {
    /// (a) Access to private data.
    pub private_data: bool,
    /// (b) Exposure to untrusted content: web pages, forum text, third-party tool
    /// descriptions, user-uploaded documents.
    pub untrusted_content: bool,
    /// (c) An outbound channel: network, messaging, publishing.
    pub outbound_channel: bool,
    /// Which leg was cut or gated, and how. Required when all three would be true.
    #[serde(default)]
    pub mitigation: String,
}

fn default_tool_calling() -> crate::hardware::ToolCalling {
    crate::hardware::ToolCalling::MultiStep
}

impl Trifecta {
    /// Whether all three legs are live.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.private_data && self.untrusted_content && self.outbound_channel
    }
}

/// Measured evidence that a configuration sustains multi-step tool calling.
///
/// D-18 refuses `multi_step` on a degraded tier, and that default is right: a tier
/// that *asserts* a capability it has never demonstrated is exactly the wishful
/// configuration the rule exists to stop.
///
/// What the rule got wrong was making the claim **unreachable** rather than
/// **unearned**. Tool-calling capability is a property of the model and the harness
/// around it, not of the card. What carries a chain here is the two-step decode
/// (LOCAL_TIER_FINDINGS §4), the exposure budget being the decoding grammar, and a
/// harness-owned plan — and not one of those depends on tensor cores or on fitting
/// the reference model. Tying the claim to the *hardware tier* was a proxy for
/// measuring it, and the proxy was wrong in the direction that costs capability.
///
/// So the claim becomes admissible on evidence, under three conditions that keep it
/// from decaying into a checkbox:
///
/// 1. The evidence names the **model it was taken on**, and that must be this
///    profile's model. Evidence for a sibling licenses nothing.
/// 2. The evidence names the **device it was taken on**, and
///    [`Profile::effective_tool_calling`] re-checks that against the machine the
///    agent is actually starting on. An attestation from a 3090 does not license a
///    1080 Ti, and one carried to a different box silently loses its force rather
///    than quietly keeping it.
/// 3. Every trial must be clean. A chain that fails one run in ten fails *slowly*,
///    which is the exact outcome D-18 exists to prevent — and re-running the eval is
///    cheap, so the strict bar costs little and refuses plainly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiStepEvidence {
    /// Which eval produced this. A name someone can go and re-run.
    pub eval: String,
    /// The model the eval drove. Must equal the profile's `model_id`.
    pub model: String,
    /// The accelerator it was measured on, as `nvidia-smi` names it. Re-checked
    /// against the detected hardware at startup.
    pub device: String,
    pub trials: u32,
    /// Trials where every check passed. Must equal `trials`.
    pub clean: u32,
    /// When it was taken, `YYYY-MM-DD`.
    pub recorded: String,
    /// What the eval actually exercised, so a reader can judge whether it was hard
    /// enough without going to find the code.
    #[serde(default)]
    pub detail: String,
}

/// The floor for an attestation to mean anything. Three trials is an anecdote.
const MIN_EVIDENCE_TRIALS: u32 = 10;

/// Per-task ceilings, enforced like step budgets (§15.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budgets {
    pub max_usd_per_task: f64,
    pub max_wall_clock_s: u64,
    pub max_tokens_per_task: u64,
}

/// One model's harness configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub model_id: String,
    pub provider: String,
    pub tier: Tier,
    pub archetype: Archetype,
    pub context: ContextBudget,
    pub tools: ToolPolicy,
    pub output: OutputPolicy,
    pub orchestration: Orchestration,
    pub trifecta: Trifecta,
    /// How much tool use this configuration can actually sustain (D-18).
    ///
    /// The reference model on the target hardware is `multi_step`; that is the
    /// baseline assumption, not a capability to be defensive about. A degraded host
    /// declares `single_shot` and the admission check refuses or escalates
    /// multi-step work rather than letting it fail slowly.
    #[serde(default = "default_tool_calling")]
    pub tool_calling: crate::hardware::ToolCalling,
    /// Measured proof for a `multi_step` claim a tier could not otherwise make.
    ///
    /// Required when a `degraded` tier declares `multi_step`; ignored otherwise, so
    /// a profile on target hardware carries none and nothing changes for it.
    #[serde(default)]
    pub multi_step_evidence: Option<MultiStepEvidence>,
    /// What this model needs to run: resident bytes at the deployed quantisation,
    /// and whether it assumes Ampere-or-newer.
    #[serde(default)]
    pub requires: Option<crate::hardware::ModelRequirements>,
    /// Where work goes when this tier cannot take it. A profile id.
    #[serde(default)]
    pub escalate_to: Option<String>,
    pub budgets: Budgets,
    /// Free-text note recorded on the profile, e.g. why a tier was chosen.
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("profile {model_id:?} is invalid: {detail}")]
    Invalid { model_id: String, detail: String },
    #[error("no profile for model {0:?}, and no conservative default is configured")]
    Unknown(String),
}

impl Profile {
    /// Checks the invariants the guide states unconditionally.
    ///
    /// Called on load, so a bad profile fails at startup rather than at the first
    /// step of a long research session.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let bad = |detail: &str| ProfileError::Invalid {
            model_id: self.model_id.clone(),
            detail: detail.to_string(),
        };

        if self.context.effective_budget_tokens == 0 {
            return Err(bad("effective_budget_tokens must be non-zero: the context budget is a hard cap, and zero means every prompt is over budget"));
        }
        if self.context.effective_budget_tokens > self.context.advertised_tokens {
            return Err(bad(
                "effective_budget_tokens exceeds the advertised window; the effective budget is meant to sit well below it, not above",
            ));
        }
        if self.context.reserve_for_output >= self.context.effective_budget_tokens {
            return Err(bad("reserve_for_output leaves no room for input"));
        }
        if self.tools.max_exposed_per_step == 0 {
            return Err(bad("max_exposed_per_step must be at least 1"));
        }
        if self.context.max_tool_result_bytes == 0 {
            return Err(bad("max_tool_result_bytes must be non-zero"));
        }

        // §3.1: constrained decoding is required for every local tier. A local model
        // without it emits malformed calls that no amount of prompting removes.
        if self.tier.is_local() && !self.output.constrained_decoding {
            return Err(bad(
                "constrained_decoding is required for local tiers (guide §3.1): format unreliability must be eliminated structurally, never prompted away",
            ));
        }
        // §3.3: there is no legitimate false here at any tier.
        if !self.output.native_tool_template {
            return Err(bad(
                "native_tool_template must be true (guide §3.3): template mismatch destroys reliability faster than parameter count does",
            ));
        }
        // §3.1: tool-call steps run at temperature 0 under constrained decoding.
        if self.output.constrained_decoding && self.output.temperature_tool_calls != 0.0 {
            return Err(bad(
                "temperature_tool_calls must be 0.0 under constrained decoding (guide §3.1)",
            ));
        }
        if self.output.max_retries_per_call == 0 {
            return Err(bad(
                "max_retries_per_call must be at least 1, or a single malformed call fails the step",
            ));
        }

        // §2.2: `routing: none` is only honest when the catalogue actually fits.
        // The registry re-checks this against the real catalogue size; here we can
        // only catch the self-contradictory case.
        if matches!(self.tools.routing, Routing::None) && self.tools.max_exposed_per_step < 2 {
            return Err(bad(
                "routing 'none' with a budget under 2 cannot expose a working tool set; use dynamic routing",
            ));
        }

        // §7 ladder: parallel calls are the first thing cut below frontier.
        if self.tier != Tier::Frontier && self.tools.parallel_calls {
            return Err(bad(
                "parallel tool calls are the first cut on the degradation ladder (guide §7.1) and are not available below the frontier tier",
            ));
        }
        // §5.4: reflection is off for local tiers.
        if self.tier.is_local() && self.orchestration.reflection {
            return Err(bad(
                "reflection is off for local tiers (guide §5.4): self-critique burns steps and local models rarely self-correct productively",
            ));
        }
        // §11: code execution is profile-gated.
        if self.tier == Tier::LocalSmall
            && self.orchestration.code_execution != CodeExecution::Denied
        {
            return Err(bad(
                "local_small may not run free-form code (guide §11): narrow tools only",
            ));
        }
        // §13.3: depth 1 by default.
        if self.orchestration.max_subagent_depth > 1 && self.tier != Tier::Frontier {
            return Err(bad(
                "sub-agent depth above 1 needs eval evidence (guide §13.3) and is not available below the frontier tier",
            ));
        }
        if self.orchestration.max_steps == 0 {
            return Err(bad("max_steps must be non-zero"));
        }

        // §14.1: an agent with all three legs is exploitable. It may still be
        // configured that way, but only with the mitigation written down.
        if self.trifecta.is_complete() && self.trifecta.mitigation.trim().is_empty() {
            return Err(bad(
                "all three trifecta legs are live (private data + untrusted content + outbound channel) with no recorded mitigation; cut or gate one leg and say so here (guide §14.1)",
            ));
        }

        // D-18. A degraded host is one that cannot run the reference model, and an
        // *asserted* multi_step claim there is the wishful configuration this rule
        // exists to stop: the work gets admitted and then fails slowly.
        //
        // But the claim is unearned, not impossible — see [`MultiStepEvidence`].
        // Measured evidence, bound to this model and re-checked against the running
        // hardware, is admissible. Nothing else is.
        if self.tier == Tier::Degraded
            && self.tool_calling == crate::hardware::ToolCalling::MultiStep
        {
            match &self.multi_step_evidence {
                None => {
                    return Err(bad(
                        "a degraded tier may not simply assert multi_step tool calling; either declare single_shot (or none) with escalate_to, so multi-step work is refused or handed on rather than failing slowly, or record `multi_step_evidence` from an eval run on this model and this hardware",
                    ));
                }
                Some(ev) => {
                    if ev.model != self.model_id {
                        return Err(bad(&format!(
                            "multi_step_evidence was taken on model {:?} but this profile runs {:?}; evidence for a different model licenses nothing",
                            ev.model, self.model_id
                        )));
                    }
                    if ev.device.trim().is_empty() {
                        return Err(bad(
                            "multi_step_evidence must name the device it was measured on, or it cannot be re-checked against the machine the agent starts on",
                        ));
                    }
                    if ev.trials < MIN_EVIDENCE_TRIALS {
                        return Err(bad(&format!(
                            "multi_step_evidence records {} trials; at least {MIN_EVIDENCE_TRIALS} are needed before a chain-capability claim means anything",
                            ev.trials
                        )));
                    }
                    if ev.clean != ev.trials {
                        return Err(bad(&format!(
                            "multi_step_evidence records {} clean of {} trials; a chain that fails one run in ten fails slowly, which is what this tier exists to prevent — every trial must be clean",
                            ev.clean, ev.trials
                        )));
                    }
                }
            }
        }
        // Evidence that licenses nothing is worse than no evidence: it reads as a
        // justification and is never consulted.
        if self.multi_step_evidence.is_some()
            && self.tool_calling != crate::hardware::ToolCalling::MultiStep
        {
            return Err(bad(
                "multi_step_evidence is recorded but tool_calling is not multi_step; drop one or the other rather than leaving a justification for a claim the profile does not make",
            ));
        }
        // A local tier that does not say what it needs cannot be fit-checked against
        // the hardware, so the resolver would have to guess — and guessing wrong
        // means an OOM partway through a long session.
        if self.tier.is_local() && self.requires.is_none() {
            return Err(bad(
                "a local tier must declare `requires` (resident bytes at the deployed quantisation, and whether it needs Ampere-or-newer) so the capability resolver can check it against the hardware",
            ));
        }
        // Escalation must point somewhere real-looking. A tier that cannot take the
        // work and names no successor can only refuse, which is legitimate but
        // should be a deliberate choice rather than an empty string.
        if let Some(target) = &self.escalate_to {
            if target.trim().is_empty() {
                return Err(bad(
                    "escalate_to is empty; name a profile id or omit the field",
                ));
            }
            if target == &self.model_id {
                return Err(bad(
                    "escalate_to points at this same profile, which would loop",
                ));
            }
        }

        if self.budgets.max_usd_per_task <= 0.0 || self.budgets.max_wall_clock_s == 0 {
            return Err(bad("per-task budgets must be positive (guide §15.3)"));
        }
        Ok(())
    }

    /// Input token budget: the effective budget minus what is held for output.
    #[must_use]
    pub fn input_budget_tokens(&self) -> u32 {
        self.context
            .effective_budget_tokens
            .saturating_sub(self.context.reserve_for_output)
    }

    /// What this profile may actually claim **on this machine**.
    ///
    /// `validate` checks an attestation is well-formed at load; this checks it still
    /// applies at startup. The two are deliberately separate, because a config file
    /// travels and a GPU does not: a profile measured on the dev box and then
    /// deployed elsewhere must lose its promotion rather than carry it silently.
    ///
    /// Returns the declared capability, or a reduced one with the reason. It reduces
    /// rather than refusing so the ordinary [`crate::hardware::admit`] path handles
    /// it — an unlicensed claim then escalates or refuses through the same machinery
    /// as any other over-demand, instead of through a second failure mode that no
    /// test would exercise.
    #[must_use]
    pub fn effective_tool_calling(
        &self,
        hw: &crate::hardware::Hardware,
    ) -> (crate::hardware::ToolCalling, Option<String>) {
        let declared = self.tool_calling;
        let Some(ev) = &self.multi_step_evidence else {
            return (declared, None);
        };
        let names: Vec<&str> = hw.devices().iter().map(|d| d.name.as_str()).collect();
        if names.iter().any(|n| n.trim() == ev.device.trim()) {
            return (declared, None);
        }
        let found = if names.is_empty() {
            "no accelerator was detected".to_string()
        } else {
            format!("this machine has {}", names.join(" + "))
        };
        (
            crate::hardware::ToolCalling::SingleShot,
            Some(format!(
                "the multi_step claim is licensed by an eval measured on {:?}, but {found}; \
                 the promotion does not transfer, so this run is treated as single_shot — \
                 re-run the eval here to restore it",
                ev.device
            )),
        )
    }
}

/// The outcome of resolving a model id for execution.
///
/// Three cases rather than a `Result`, because "we ran something else" is not an
/// error and is not a success — it is a decision the caller has to make.
#[derive(Debug)]
pub enum Resolution<'a> {
    /// The requested model has a profile.
    Exact(&'a Profile),
    /// It does not, and this is the configured conservative default. The caller
    /// decides whether to accept the substitution or refuse the work.
    Substituted {
        requested: String,
        profile: &'a Profile,
        reason: String,
    },
    /// It does not, and no default is configured. Nothing to run.
    Unknown { requested: String },
}

impl<'a> Resolution<'a> {
    /// The profile, if the caller is willing to accept a substitution.
    ///
    /// Named to make the acceptance explicit at the call site — `.profile()` would
    /// read as a getter and would reintroduce exactly the silent downgrade this type
    /// exists to prevent.
    #[must_use]
    pub fn accepting_substitution(&self) -> Option<&'a Profile> {
        match self {
            Resolution::Exact(p) => Some(p),
            Resolution::Substituted { profile, .. } => Some(profile),
            Resolution::Unknown { .. } => None,
        }
    }

    /// The profile only when it is exactly what was asked for.
    #[must_use]
    pub fn exact(&self) -> Option<&'a Profile> {
        match self {
            Resolution::Exact(p) => Some(p),
            _ => None,
        }
    }
}

/// Every profile the platform knows, keyed by `model_id`.
#[derive(Debug, Clone, Default)]
pub struct ProfileSet {
    profiles: BTreeMap<String, Profile>,
    /// The profile an unknown model falls back to. Guide §1.2: the most
    /// conservative tier until evals justify promotion.
    default_model_id: Option<String>,
}

impl ProfileSet {
    /// Loads every `*.yaml` in a directory.
    ///
    /// An unparseable or invalid profile is an error, not a skip. A profile set that
    /// silently dropped a bad file would fall back to the conservative default and
    /// look like a working system running the wrong model.
    pub fn load_dir(dir: &Path) -> Result<Self, ProfileError> {
        let mut profiles = BTreeMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| ProfileError::Io {
            path: dir.display().to_string(),
            source: e,
        })?;
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
            .collect();
        paths.sort();

        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|e| ProfileError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
            let profile: Profile =
                serde_yaml::from_str(&text).map_err(|e| ProfileError::Parse {
                    path: path.display().to_string(),
                    source: e,
                })?;
            profile.validate()?;
            profiles.insert(profile.model_id.clone(), profile);
        }

        // NOT derived from the lowest tier. Deriving it means adding a dev-box
        // profile silently repoints the platform-wide fallback at the dev box —
        // a config change with a consequence nobody wrote down. The default is set
        // explicitly by the caller via `with_default`, and is `None` until then.
        Ok(Self {
            profiles,
            default_model_id: None,
        })
    }

    #[must_use]
    pub fn from_profiles(profiles: Vec<Profile>) -> Self {
        let map: BTreeMap<String, Profile> = profiles
            .into_iter()
            .map(|p| (p.model_id.clone(), p))
            .collect();
        Self {
            profiles: map,
            default_model_id: None,
        }
    }

    /// Resolves a model id to its profile.
    ///
    /// An unknown model gets the most conservative profile present and a loud log
    /// line — guide §1.2. Silently running an unknown model at frontier settings is
    /// the failure this prevents.
    pub fn resolve(&self, model_id: &str) -> Result<&Profile, ProfileError> {
        self.profiles
            .get(model_id)
            .ok_or_else(|| ProfileError::Unknown(model_id.to_string()))
    }

    /// Resolves a model for EXECUTION, refusing rather than quietly downgrading.
    ///
    /// The difference from [`resolve`] is the whole point. `resolve` used to hand
    /// back the most conservative profile for any unknown id with nothing but a
    /// `tracing::warn!` — so a typo in a pinned model id produced a session that ran
    /// happily at a tier nobody chose, and the only evidence was a log line that no
    /// caller could see.
    ///
    /// Guide §1.2 does say an unknown model should default conservatively, and it is
    /// right — but "default conservatively" has to reach the caller as a decision,
    /// not as a `Ok(&Profile)` indistinguishable from a hit. So a substitution is
    /// reported as [`Resolution::Substituted`] and the caller decides whether to
    /// accept it.
    pub fn resolve_for_execution(&self, model_id: &str) -> Resolution<'_> {
        if let Some(p) = self.profiles.get(model_id) {
            return Resolution::Exact(p);
        }
        match self
            .default_model_id
            .as_ref()
            .and_then(|id| self.profiles.get(id))
        {
            Some(fallback) => Resolution::Substituted {
                requested: model_id.to_string(),
                profile: fallback,
                reason: format!(
                    "no capability profile for {model_id:?}; the configured default                      {:?} is the most conservative tier available and no eval has                      justified anything wider",
                    fallback.model_id
                ),
            },
            None => Resolution::Unknown {
                requested: model_id.to_string(),
            },
        }
    }

    #[must_use]
    pub fn get(&self, model_id: &str) -> Option<&Profile> {
        self.profiles.get(model_id)
    }

    /// Names the profile unknown models fall back to.
    ///
    /// Explicit rather than derived: see `load_dir`.
    #[must_use]
    pub fn with_default(mut self, model_id: impl Into<String>) -> Self {
        self.default_model_id = Some(model_id.into());
        self
    }

    #[must_use]
    pub fn default_model_id(&self) -> Option<&str> {
        self.default_model_id.as_deref()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

#[cfg(test)]
pub(crate) fn frontier_fixture() -> Profile {
    Profile {
        model_id: "test-frontier".into(),
        provider: "anthropic".into(),
        tier: Tier::Frontier,
        archetype: Archetype::Background,
        context: ContextBudget {
            advertised_tokens: 200_000,
            effective_budget_tokens: 120_000,
            reserve_for_output: 16_000,
            max_tool_result_bytes: 32 * 1024,
        },
        tools: ToolPolicy {
            max_exposed_per_step: 24,
            schema_style: SchemaStyle::Rich,
            routing: Routing::Dynamic,
            parallel_calls: true,
        },
        output: OutputPolicy {
            constrained_decoding: false,
            native_tool_template: true,
            temperature_tool_calls: 0.0,
            max_retries_per_call: 3,
        },
        orchestration: Orchestration {
            mode: Mode::Freeform,
            max_steps: 200,
            few_shot_examples: 0,
            reflection: true,
            max_subagent_depth: 1,
            code_execution: CodeExecution::Full,
        },
        trifecta: Trifecta {
            private_data: true,
            untrusted_content: true,
            outbound_channel: false,
            mitigation: "outbound cut: internal-only container network".into(),
        },
        tool_calling: crate::hardware::ToolCalling::MultiStep,
        multi_step_evidence: None,
        requires: None,
        escalate_to: None,
        budgets: Budgets {
            max_usd_per_task: 25.0,
            max_wall_clock_s: 14_400,
            max_tokens_per_task: 20_000_000,
        },
        notes: String::new(),
    }
}

/// A degraded-tier profile: the dev box shape, valid apart from whatever a test
/// deliberately breaks.
#[cfg(test)]
pub(crate) fn degraded_fixture() -> Profile {
    Profile {
        model_id: "test-local".into(),
        provider: "ollama".into(),
        tier: Tier::Degraded,
        archetype: Archetype::ChatAssistant,
        context: ContextBudget {
            advertised_tokens: 32_768,
            effective_budget_tokens: 12_000,
            reserve_for_output: 1_500,
            max_tool_result_bytes: 4096,
        },
        tools: ToolPolicy {
            max_exposed_per_step: 3,
            schema_style: SchemaStyle::Flat,
            routing: Routing::Dynamic,
            parallel_calls: false,
        },
        output: OutputPolicy {
            constrained_decoding: true,
            native_tool_template: true,
            temperature_tool_calls: 0.0,
            max_retries_per_call: 3,
        },
        orchestration: Orchestration {
            mode: Mode::PlannerExecutor,
            max_steps: 12,
            few_shot_examples: 2,
            reflection: false,
            max_subagent_depth: 1,
            code_execution: CodeExecution::Denied,
        },
        trifecta: Trifecta {
            private_data: true,
            untrusted_content: true,
            outbound_channel: false,
            mitigation: "outbound cut".into(),
        },
        tool_calling: crate::hardware::ToolCalling::SingleShot,
        multi_step_evidence: None,
        requires: Some(crate::hardware::ModelRequirements {
            resident_bytes: 5 * 1024 * 1024 * 1024,
            requires_target_architecture: false,
        }),
        // A fixture names no production model: `model_names_and_context_sizes_are
        // _config_not_logic` reads this file, and a real model id here would be a
        // model name in logic.
        escalate_to: Some("test-frontier".into()),
        budgets: Budgets {
            max_usd_per_task: 0.5,
            max_wall_clock_s: 3600,
            max_tokens_per_task: 2_000_000,
        },
        notes: String::new(),
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;
    use crate::hardware::{self, ToolCalling};

    fn good_evidence() -> MultiStepEvidence {
        MultiStepEvidence {
            eval: "local_multistep_eval".into(),
            model: "test-local".into(),
            device: "NVIDIA GeForce GTX 1080 Ti".into(),
            trials: 10,
            clean: 10,
            recorded: "2026-09-12".into(),
            detail: String::new(),
        }
    }

    fn claiming(ev: Option<MultiStepEvidence>) -> Profile {
        let mut p = degraded_fixture();
        p.tool_calling = ToolCalling::MultiStep;
        p.multi_step_evidence = ev;
        p
    }

    /// The original rule, intact. An assertion with nothing behind it is still the
    /// thing D-18 exists to refuse.
    #[test]
    fn a_degraded_tier_still_cannot_simply_assert_multi_step() {
        let err = claiming(None).validate().unwrap_err();
        assert!(format!("{err}").contains("multi_step_evidence"));
    }

    #[test]
    fn a_degraded_tier_with_measured_evidence_may_claim_multi_step() {
        claiming(Some(good_evidence())).validate().unwrap();
    }

    /// Evidence is per model. A sibling that happens to share a family proves
    /// nothing about the one actually being run.
    #[test]
    fn evidence_for_a_different_model_licenses_nothing() {
        let mut ev = good_evidence();
        ev.model = "some-other-model".into();
        let err = claiming(Some(ev)).validate().unwrap_err();
        assert!(format!("{err}").contains("licenses nothing"));
    }

    #[test]
    fn three_trials_is_an_anecdote_not_evidence() {
        let mut ev = good_evidence();
        ev.trials = 3;
        ev.clean = 3;
        assert!(claiming(Some(ev)).validate().is_err());
    }

    /// The strict bar, and the reason for it: a chain that fails one run in ten
    /// fails slowly, which is the exact outcome the tier exists to prevent.
    #[test]
    fn one_failed_trial_in_ten_is_refused() {
        let mut ev = good_evidence();
        ev.clean = 9;
        let err = claiming(Some(ev)).validate().unwrap_err();
        assert!(format!("{err}").contains("fails slowly"));
    }

    #[test]
    fn an_attestation_must_name_the_device_it_was_taken_on() {
        let mut ev = good_evidence();
        ev.device = "  ".into();
        assert!(claiming(Some(ev)).validate().is_err());
    }

    /// Evidence attached to a profile that does not make the claim reads as a
    /// justification and is never consulted — worse than no evidence at all.
    #[test]
    fn evidence_without_the_claim_is_refused() {
        let mut p = degraded_fixture();
        p.multi_step_evidence = Some(good_evidence());
        assert!(p.validate().is_err());
    }

    /// The runtime half. A config file travels; a GPU does not.
    #[test]
    fn the_promotion_holds_on_the_machine_it_was_measured_on() {
        let p = claiming(Some(good_evidence()));
        let hw = hardware::discrete(vec![hardware::gtx_1080ti()], false, 32);
        let (cap, why) = p.effective_tool_calling(&hw);
        assert_eq!(cap, ToolCalling::MultiStep);
        assert!(why.is_none());
    }

    #[test]
    fn the_promotion_does_not_travel_to_another_machine() {
        let p = claiming(Some(good_evidence()));
        let hw = hardware::discrete(vec![hardware::rtx_3090()], false, 64);
        let (cap, why) = p.effective_tool_calling(&hw);
        assert_eq!(
            cap,
            ToolCalling::SingleShot,
            "an eval taken on a 1080 Ti does not license a claim on a different card"
        );
        assert!(why.unwrap().contains("does not transfer"));
    }

    #[test]
    fn a_machine_with_no_accelerator_licenses_nothing() {
        let p = claiming(Some(good_evidence()));
        let (cap, why) = p.effective_tool_calling(&hardware::discrete(vec![], false, 64));
        assert_eq!(cap, ToolCalling::SingleShot);
        assert!(why.unwrap().contains("no accelerator"));
    }

    /// A profile carrying no attestation is untouched by any of this.
    #[test]
    fn a_profile_without_evidence_is_unaffected() {
        let p = frontier_fixture();
        let hw = hardware::discrete(vec![hardware::rtx_3090()], false, 64);
        assert_eq!(p.effective_tool_calling(&hw).0, ToolCalling::MultiStep);
    }

    /// The end-to-end point of the change: with the promotion in force, a
    /// multi-step task is admitted on the dev box instead of escalating.
    #[test]
    fn a_promoted_degraded_tier_admits_multi_step_work() {
        let p = claiming(Some(good_evidence()));
        let hw = hardware::discrete(vec![hardware::gtx_1080ti()], false, 32);
        let (cap, _) = p.effective_tool_calling(&hw);
        assert_eq!(
            hardware::admit(cap, hardware::TaskDemand::MultiStep, p.escalate_to.as_deref()),
            hardware::Admission::Admit
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_is_valid() {
        frontier_fixture().validate().unwrap();
    }

    #[test]
    fn a_local_tier_without_constrained_decoding_is_refused() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.tools.parallel_calls = false;
        p.orchestration.reflection = false;
        p.output.constrained_decoding = false;
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("constrained_decoding"), "{err}");
    }

    #[test]
    fn a_custom_chat_template_is_refused_at_every_tier() {
        let mut p = frontier_fixture();
        p.output.native_tool_template = false;
        assert!(p
            .validate()
            .unwrap_err()
            .to_string()
            .contains("native_tool_template"));
    }

    #[test]
    fn parallel_calls_are_the_first_cut_below_frontier() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalHigh;
        p.output.constrained_decoding = true;
        p.orchestration.reflection = false;
        // parallel_calls left true
        assert!(p
            .validate()
            .unwrap_err()
            .to_string()
            .contains("parallel tool calls"));
    }

    #[test]
    fn local_small_may_not_run_free_form_code() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalSmall;
        p.tools.parallel_calls = false;
        p.orchestration.reflection = false;
        p.output.constrained_decoding = true;
        assert!(p
            .validate()
            .unwrap_err()
            .to_string()
            .contains("free-form code"));
    }

    /// The audit that the guide requires to be written down rather than believed.
    #[test]
    fn all_three_trifecta_legs_need_a_recorded_mitigation() {
        let mut p = frontier_fixture();
        p.trifecta.outbound_channel = true;
        p.trifecta.mitigation = String::new();
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("trifecta"), "{err}");

        p.trifecta.mitigation = "outbound gated behind human-approved deliver".into();
        assert!(
            p.validate().is_ok(),
            "a recorded mitigation is the documented way to hold all three"
        );
    }

    #[test]
    fn an_effective_budget_above_the_advertised_window_is_refused() {
        let mut p = frontier_fixture();
        p.context.effective_budget_tokens = p.context.advertised_tokens + 1;
        assert!(p.validate().is_err());
    }

    /// An unknown model is a decision the caller has to make, not a silent
    /// substitution.
    ///
    /// This test used to assert that `resolve` handed back the lowest tier with a
    /// log line. That was the bug: a typo in a pinned model id produced a session
    /// running at a tier nobody chose, and `Ok(&Profile)` made it indistinguishable
    /// from a hit.
    #[test]
    fn an_unknown_model_does_not_silently_become_a_downgrade() {
        let mut small = frontier_fixture();
        small.model_id = "tiny".into();
        small.tier = Tier::LocalSmall;
        small.tools.parallel_calls = false;
        small.orchestration.reflection = false;
        small.output.constrained_decoding = true;
        small.orchestration.code_execution = CodeExecution::Denied;
        small.requires = Some(crate::hardware::ModelRequirements {
            resident_bytes: 5 * 1024 * 1024 * 1024,
            requires_target_architecture: false,
        });

        let set = ProfileSet::from_profiles(vec![frontier_fixture(), small]);

        // With no default configured, an unknown id resolves to nothing at all.
        assert!(matches!(
            set.resolve_for_execution("nobody-configured-this"),
            Resolution::Unknown { .. }
        ));
        assert!(set.resolve("nobody-configured-this").is_err());

        // With a default, the substitution is REPORTED rather than returned as a hit.
        let set = set.with_default("tiny");
        match set.resolve_for_execution("nobody-configured-this") {
            Resolution::Substituted {
                profile, reason, ..
            } => {
                assert_eq!(profile.tier, Tier::LocalSmall);
                assert!(reason.contains("no capability profile"));
            }
            other => panic!("expected a reported substitution, got {other:?}"),
        }

        // And an exact hit stays an exact hit.
        assert!(set.resolve_for_execution("test-frontier").exact().is_some());
        assert!(set
            .resolve_for_execution("nobody-configured-this")
            .exact()
            .is_none());
    }

    /// Adding a low-tier profile must not silently repoint the platform default.
    #[test]
    fn the_default_profile_is_explicit_not_derived_from_the_lowest_tier() {
        let set = ProfileSet::from_profiles(vec![frontier_fixture()]);
        assert_eq!(
            set.default_model_id(),
            None,
            "a default nobody named is a default nobody reviewed"
        );
    }

    #[test]
    fn tiers_order_from_small_to_frontier() {
        assert!(Tier::LocalSmall < Tier::LocalMid);
        assert!(Tier::LocalMid < Tier::LocalHigh);
        assert!(Tier::LocalHigh < Tier::Frontier);
        assert!(!Tier::Frontier.is_local());
    }
}
