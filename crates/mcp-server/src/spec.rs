//! One declaration per tool, and one shape for what a tool answers with.
//!
//! AGENT-006 §2.3, §2.4, §2.4a, §2.7. Set S task S-1.1, S-1.2, S-1.19.
//!
//! Before this module, a tool's identity was spread across four places that had
//! to be kept in agreement by hand: a JSON literal in `lib.rs` for the name,
//! description and schema; a row in `taxonomy.rs` for the namespace and risk; a
//! name in `INTERNAL_AGENT_TOOLS` for exposure; and a name in one of
//! `ledger::audit`'s four lists for approval. Each of those is a place a tool can
//! be missing from, and the direction of every miss is permissive: an
//! unclassified tool routes nowhere, an unlisted tool is ungated. That is not
//! hypothetical — `promote_model` vs `promote_model_version` (S-0.2) left the
//! platform's one promotion tool ungated on both drivers for as long as the two
//! spellings coexisted.
//!
//! So: one [`ToolSpec`] per tool, and everything else is derived from it and
//! asserted. The declaration is the only thing a tool author writes.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::taxonomy::Risk;
use crate::ToolProfile;

// ── The declaration ──────────────────────────────────────────────────────────

/// Whether a tool is current, or a tombstone kept so old evidence resolves.
///
/// Deprecation is never a deletion (§2.7). A trial ticket cites the tool that
/// produced it; removing the tool would make every ticket citing it unresolvable
/// and the selection-bias arithmetic that reads those tickets would be comparing
/// runs it cannot name. A `Deprecated` tool stays in [`catalog`] — so
/// [`find`] answers for it — and is absent from [`exposed`], so no profile
/// offers it to a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Active,
    Deprecated {
        /// ISO date the tool stopped being offered.
        since: &'static str,
        /// What to call instead, if anything.
        superseded_by: Option<&'static str>,
        reason: &'static str,
    },
}

impl ToolStatus {
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Whether calling this tool registers a trial before it runs (§2.5, INV-16).
///
/// `Falsification` trials are counted separately because
/// `audit.selection_bias_verdict` folds them into N_eff differently: an attack
/// on a strategy is not a search for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialKind {
    None,
    Search,
    Falsification,
}

/// The approval envelope a call falls under.
///
/// Mirrors `ledger::audit::Envelope`, kept local so this crate does not depend on
/// `ledger` (and through it `sqlx` and `ed25519-dalek`) for a five-variant enum.
/// The two are reconciled by `every_tool_envelope_matches_the_ledger` in
/// `crates/invariants`, which fails if either side gains a variant or a tool's
/// declared envelope disagrees with the list `Envelope::for_action` matches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envelope {
    None,
    Spend,
    Promotion,
    SealedHoldout,
    Tier3Memory,
}

impl Envelope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Spend => "spend",
            Self::Promotion => "promotion",
            Self::SealedHoldout => "sealed_holdout",
            Self::Tier3Memory => "tier3_memory",
        }
    }

    #[must_use]
    pub fn requires_approval(self) -> bool {
        self != Self::None
    }
}

/// How the tool is implemented (§2.2). Routing and exposure do not depend on
/// this; the dispatcher does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Rust, in `tools/<ns>.rs`, behind an API route.
    Typed,
    /// A Python template run by the skill runner with data pre-materialised.
    Templated { template: &'static str },
    /// A server-side sequence of typed/templated calls; one trial family.
    Workflow { steps: &'static [&'static str] },
    /// Lives inside the loop and is never dispatched (`search_tools`,
    /// `record_finding`, `finish_task`).
    HarnessOwned,
}

/// What the tool's answer is allowed to weigh, and how its `summary` is written.
///
/// `summary_template` is the mechanism behind §2.4's "generated, not
/// hand-written": a format string over `detail_fields`, rendered by
/// [`render_summary`]. Sixty hand-written prose blobs drift from the numbers they
/// describe, and the summary is the part a flat-tier model reads *first* — a
/// summary that disagrees with its own `detail` is worse than none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputBudget {
    /// Soft ceiling on the rendered `summary`, in tokens (§2.4 default 400).
    pub summary_tokens: u32,
    /// `{slot}` template over `detail_fields`; `None` for tools not yet migrated
    /// to [`ToolResult`].
    pub summary_template: Option<&'static str>,
    /// `(field, unit)` for every field the template may name. A slot naming a
    /// field absent from here fails `a_summary_template_only_names_declared_fields`.
    pub detail_fields: &'static [(&'static str, &'static str)],
}

impl Default for OutputBudget {
    fn default() -> Self {
        Self {
            summary_tokens: 400,
            summary_template: None,
            detail_fields: &[],
        }
    }
}

/// Everything about one tool, in one place (§2.3).
#[derive(Clone, Debug)]
pub struct ToolSpec {
    /// `verb_noun`, unique across the catalogue, ≤ 32 chars.
    pub name: &'static str,
    /// Bumped when behaviour or the *meaning* of an output field changes; stamped
    /// on every trial ticket so evidence stays comparable (§2.7). Not bumped for
    /// a wording fix.
    pub version: u32,
    pub status: ToolStatus,
    /// §2.1. The routing unit: a plan step names a namespace and the registry
    /// exposes that slice.
    pub namespace: &'static str,
    pub risk: Risk,
    /// ≤ 2 sentences saying *when to use it*. Anything longer is documentation
    /// and belongs in the authoring guide (AGENT-005 §5.4).
    pub description: &'static str,
    pub input_schema: Value,
    pub output: OutputBudget,
    /// Which returned fields are ids the model may cite as evidence (P4). Empty
    /// means nothing this tool returns is citeable.
    pub evidence: &'static [&'static str],
    pub spends_trial: TrialKind,
    pub envelope: Envelope,
    /// Bounds *expense* rather than trials (§2.4b): a loop can exhaust a dollar
    /// budget with fifty cheap-looking dispatches without ever tripping the trial
    /// counter. `None` for reads.
    pub max_calls_per_run: Option<u32>,
    pub profiles: &'static [ToolProfile],
    pub tier: Tier,
    /// Heading in the guide's `## Tool notes`, when the prose that used to be in
    /// the description lives there.
    pub notes_section: Option<&'static str>,
}

/// How far from capital a tool reaches (AGENT-013 section 4; ADR-0037 section 1).
///
/// Named with the strings `harness::state::Ring` serialises to, and the same
/// three the desk's `lib/desk/rings.ts` indexes, so the value travels from a
/// `ToolSpec` to the surface unchanged.
///
/// It lives here rather than in `crates/api` because the desk enforces it: the
/// Tools panel, the catalogue's pin action and the composer's attach menu all
/// filter on `tool.ring`, and until now **nothing on this side assigned one**.
/// A filter is worth exactly what the field it reads is worth.
///
/// **Derived, never listed.** The alternative was an array of tool names beside
/// the catalogue, which is precisely the shape `internal_agent_tools` had until
/// S-1.3 — and the direction of a miss there was silent, because a tool absent
/// from a list is simply a tool nobody notices is absent. Here a miss would be
/// worse than silent: a tool that reaches paper and is not in the list reports
/// ring 0 and becomes clickable inside a Chat turn.
pub const RING_RESEARCH: &str = "research";
pub const RING_PLATFORM_ARTIFACTS: &str = "platform_artifacts";
pub const RING_PAPER: &str = "paper";

impl ToolSpec {
    /// The outermost ring this tool reaches.
    ///
    /// Read from what the tool already declares, so it cannot disagree with the
    /// gate:
    ///
    /// * [`Envelope::Promotion`] is defined as "moving a strategy toward
    ///   capital", which is ring 2 by definition. `arm_automation` is the
    ///   member that matters.
    /// * A write or a destructive call changes the platform's own artifacts —
    ///   strategies, models, experiments — which is ring 1.
    /// * Everything else reads, and a read reaches nothing. `Outbound` is here
    ///   too: the two web tools fetch, they do not write.
    #[must_use]
    pub fn ring(&self) -> &'static str {
        if self.envelope == Envelope::Promotion {
            return RING_PAPER;
        }
        match self.risk {
            Risk::Write | Risk::Destructive => RING_PLATFORM_ARTIFACTS,
            Risk::Read | Risk::Outbound => RING_RESEARCH,
        }
    }

    /// The five things a tool cannot omit. Everything else has a default that is
    /// the conservative reading: MCP-only exposure, no trial, no envelope, no
    /// evidence ids, `Active`, version 1, `Typed`.
    #[must_use]
    pub fn new(
        name: &'static str,
        namespace: &'static str,
        risk: Risk,
        description: &'static str,
        input_schema: Value,
    ) -> Self {
        Self {
            name,
            version: 1,
            status: ToolStatus::Active,
            namespace,
            risk,
            description,
            input_schema,
            output: OutputBudget::default(),
            evidence: &[],
            spends_trial: TrialKind::None,
            envelope: Envelope::None,
            max_calls_per_run: None,
            profiles: &[ToolProfile::Mcp],
            tier: Tier::Typed,
            notes_section: None,
        }
    }

    /// Also exposed to the in-app agent, not only the MCP client.
    #[must_use]
    pub fn internal(mut self) -> Self {
        self.profiles = &[ToolProfile::Mcp, ToolProfile::InternalAgent];
        self
    }

    #[must_use]
    pub fn version(mut self, v: u32) -> Self {
        self.version = v;
        self
    }

    #[must_use]
    pub fn deprecated(
        mut self,
        since: &'static str,
        superseded_by: Option<&'static str>,
        reason: &'static str,
    ) -> Self {
        self.status = ToolStatus::Deprecated {
            since,
            superseded_by,
            reason,
        };
        self
    }

    #[must_use]
    pub fn evidence(mut self, ids: &'static [&'static str]) -> Self {
        self.evidence = ids;
        self
    }

    #[must_use]
    pub fn spends_trial(mut self, kind: TrialKind) -> Self {
        self.spends_trial = kind;
        self
    }

    #[must_use]
    pub fn envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = envelope;
        self
    }

    #[must_use]
    pub fn max_calls_per_run(mut self, n: u32) -> Self {
        self.max_calls_per_run = Some(n);
        self
    }

    #[must_use]
    pub fn tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }

    #[must_use]
    pub fn notes(mut self, section: &'static str) -> Self {
        self.notes_section = Some(section);
        self
    }

    #[must_use]
    pub fn summary(
        mut self,
        template: &'static str,
        detail_fields: &'static [(&'static str, &'static str)],
    ) -> Self {
        self.output.summary_template = Some(template);
        self.output.detail_fields = detail_fields;
        self
    }

    /// The MCP wire descriptor, stamped with namespace and risk (ADR-0031).
    ///
    /// This is the only renderer; `lib.rs` no longer holds a second copy of the
    /// name, description or schema, so the two cannot disagree.
    #[must_use]
    pub fn descriptor(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "namespace": self.namespace,
            "risk": self.risk.as_str(),
        })
    }

    /// Whether `profile` offers this tool. A deprecated tool is offered to
    /// nobody, whatever its `profiles` still says (§2.7).
    #[must_use]
    pub fn exposed_to(&self, profile: ToolProfile) -> bool {
        self.status.is_active() && self.profiles.contains(&profile)
    }
}

// ── The registry ─────────────────────────────────────────────────────────────

/// Every tool the platform declares, in catalogue order.
///
/// Built once. `input_schema` is a `serde_json::Value`, which cannot be
/// constructed in a `const`, so this is a `OnceLock` rather than a static array —
/// the only concession the type makes to Rust.
pub fn catalog() -> &'static [ToolSpec] {
    static CATALOG: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut specs = Vec::new();
        specs.extend(crate::tools::core::specs());
        specs.extend(crate::tools::discovery::specs());
        specs.extend(crate::tools::market::specs());
        specs.extend(crate::tools::assets::specs());
        specs.extend(crate::tools::authoring::specs());
        specs.extend(crate::tools::builder::specs());
        specs.extend(crate::tools::backtests::specs());
        specs.extend(crate::tools::portfolio::specs());
        // S-3.18. Read-only over COMP-007's monitor tables; the three mutations
        // beside it require a human session and no tool carries that scope.
        specs.extend(crate::tools::control_limits::specs());
        specs.extend(crate::tools::models::specs());
        specs.extend(crate::tools::automations::specs());
        specs.extend(crate::tools::research::specs());
        specs.extend(crate::tools::audit::specs());
        specs.extend(crate::tools::venues::specs());
        specs.extend(crate::tools::strategy::specs());
        // Set U U-6.3: the screener, through the same route a person uses.
        specs.extend(crate::tools::selections::specs());
        specs.extend(crate::tools::falsification::specs());
        // S-3.12. Statistics over a run the ledger already holds, so both
        // declare `TrialKind::None`; see the module docs for why that is a
        // correctness claim rather than a default.
        specs.extend(crate::tools::path_risk::specs());
        specs.extend(crate::tools::lifecycle::specs());
        specs.extend(crate::tools::campaign::specs());
        specs.extend(crate::tools::research_tools::specs());
        // The batteries (S-8.1). Only the ones that can actually run: a
        // workflow with a step nothing performs is absent rather than offered
        // short — see `tools::workflows`.
        specs.extend(crate::tools::workflows::specs());
        // S-8.8. Hidden below the `full` code-execution tier by
        // `ToolRegistry::expose_within`, which gates the whole namespace.
        specs.extend(crate::tools::code::specs());
        // Last, and appended rather than interleaved: the templated tier's order is
        // the directory's, and a tool moving namespace should not reshuffle the
        // hand-written half of the catalogue in every diff.
        specs.extend(crate::tools::templated::specs());
        assert_no_duplicates(&specs);
        specs
    })
}

/// A tool declared twice is a contradiction, not a merge: the two declarations
/// disagree about *something* and whichever the lookup finds first wins silently.
///
/// This panics at first use rather than failing a test, because every caller of
/// [`catalog`] — routing, exposure, gating — would otherwise be reading one of
/// two answers with no way to tell which.
fn assert_no_duplicates(specs: &[ToolSpec]) {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dupes: Vec<String> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        if let Some(first) = seen.insert(spec.name, i) {
            dupes.push(format!(
                "`{}` is declared twice: catalogue index {first} ({}) and {i} ({})",
                spec.name, specs[first].namespace, spec.namespace
            ));
        }
    }
    assert!(dupes.is_empty(), "duplicate tool declarations: {dupes:#?}");
}

/// The spec for a tool name, **including deprecated tools**, so a ticket citing a
/// retired tool still resolves (§2.7).
#[must_use]
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    catalog().iter().find(|s| s.name == name)
}

/// The tools a profile offers. Deprecated tools are excluded.
#[must_use]
pub fn exposed(profile: ToolProfile) -> Vec<&'static ToolSpec> {
    catalog().iter().filter(|s| s.exposed_to(profile)).collect()
}

/// Every tool name, active and deprecated, sorted.
#[must_use]
pub fn names() -> Vec<&'static str> {
    let mut n: Vec<&str> = catalog().iter().map(|s| s.name).collect();
    n.sort_unstable();
    n
}

// ── The answer: ToolResult (§2.4) ────────────────────────────────────────────

/// What a tool answers with: an answer, not a data dump.
///
/// `summary` is what a flat-tier model reads first and often all it reads, so it
/// is rendered from [`OutputBudget::summary_template`] over `detail` rather than
/// written by hand. Every citeable number carries its `n`, and its `se` where it
/// has one — AGENT-001 §10.2's "every number with its n, SE and handle".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolResult {
    pub summary: String,
    /// Ids the model may cite. These are what `finish_task` checks an `evidence`
    /// claim against (§2.5).
    pub ids: BTreeMap<String, String>,
    /// Sample size behind the headline number. A numeric claim without one
    /// renders "(n unknown)" rather than reading as though it were measured.
    pub n: Option<u64>,
    /// Standard error of the headline estimate. Not money: a dispersion of a
    /// statistic, so `f64` is the right type here.
    pub se: Option<f64>,
    pub detail: Value,
    /// Set whenever the full result did not fit the profile's cap. The harness
    /// writes the file; this is the path the model asks for if it wants the rest.
    pub handle: Option<String>,
}

impl ToolResult {
    #[must_use]
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_id(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.ids.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }

    #[must_use]
    pub fn with_n(mut self, n: u64) -> Self {
        self.n = Some(n);
        self
    }

    #[must_use]
    pub fn with_se(mut self, se: f64) -> Self {
        self.se = Some(se);
        self
    }

    /// Render within a byte cap, dropping `detail` and naming a handle if the
    /// whole result does not fit.
    ///
    /// `max_bytes` is the *effective* cap — `min(profile cap, remaining body)` at
    /// the moment the result arrives, not the profile constant (S-1.18). A 32 KiB
    /// profile cap applied against 8 KiB of remaining body admits one result that
    /// evicts the working state, including the run's own findings.
    ///
    /// The summary is never dropped. A result that cannot fit its own summary is
    /// still returned with the summary truncated on a char boundary, because a
    /// tool that answers with nothing is indistinguishable from a tool that
    /// failed, and the model would retry it.
    #[must_use]
    pub fn render_within(&self, max_bytes: usize, handle_hint: &str) -> Value {
        let full = self.to_value(None);
        if serde_json::to_string(&full)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
            <= max_bytes
        {
            return full;
        }
        let handle = self
            .handle
            .clone()
            .unwrap_or_else(|| handle_hint.to_string());
        let trimmed = self.to_value(Some(&handle));
        if serde_json::to_string(&trimmed)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
            <= max_bytes
        {
            return trimmed;
        }
        // Even without `detail` it does not fit: keep the identity and as much
        // summary as the cap allows.
        let overhead = serde_json::to_string(
            &Self {
                summary: String::new(),
                ..self.clone()
            }
            .to_value(Some(&handle)),
        )
        .map(|s| s.len())
        .unwrap_or(0);
        let room = max_bytes.saturating_sub(overhead);
        let mut cut = room.min(self.summary.len());
        while cut > 0 && !self.summary.is_char_boundary(cut) {
            cut -= 1;
        }
        Self {
            summary: self.summary[..cut].to_string(),
            ..self.clone()
        }
        .to_value(Some(&handle))
    }

    fn to_value(&self, handle: Option<&str>) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("summary".into(), json!(self.summary));
        if !self.ids.is_empty() {
            obj.insert("ids".into(), json!(self.ids));
        }
        if let Some(n) = self.n {
            obj.insert("n".into(), json!(n));
        }
        if let Some(se) = self.se {
            obj.insert("se".into(), json!(se));
        }
        match handle {
            // Overflowed: the handle replaces `detail`, which is the whole point
            // of having one.
            Some(h) => {
                obj.insert("handle".into(), json!(h));
            }
            None => {
                if !self.detail.is_null() {
                    obj.insert("detail".into(), self.detail.clone());
                }
                if let Some(h) = &self.handle {
                    obj.insert("handle".into(), json!(h));
                }
            }
        }
        Value::Object(obj)
    }
}

// ── Summary templates (§2.4) ─────────────────────────────────────────────────

/// Why a template could not be rendered. Both variants are build-time problems,
/// not runtime ones: the first is caught by
/// `a_summary_template_only_names_declared_fields`, the second by the tool's own
/// test against its reference result.
#[derive(Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// The template names a field the spec never declared. This is the one the
    /// spec says must fail the build.
    UndeclaredField {
        slot: String,
    },
    /// Declared, but absent from this particular `detail`.
    MissingValue {
        slot: String,
    },
    Unterminated,
}

/// Slots named by a template, in order, with duplicates kept.
///
/// `{{` escapes a literal brace, so a template can contain one.
pub fn template_slots(template: &str) -> Result<Vec<String>, TemplateError> {
    let mut slots = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if rest[open + 1..].starts_with('{') {
            rest = &rest[open + 2..];
            continue;
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or(TemplateError::Unterminated)?;
        slots.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    Ok(slots)
}

/// Render a declared template over a result's own `detail`.
///
/// A slot that names an undeclared field is [`TemplateError::UndeclaredField`] —
/// the summary is never allowed to describe a number the schema does not define,
/// because that is exactly how a summary comes to disagree with its detail. The
/// unit declared beside the field is appended, so "1234" reads "1234 bars".
///
/// A numeric slot whose value is absent renders `(n unknown)` rather than being
/// omitted: a missing number that looks like prose reads as though it were
/// measured.
pub fn render_summary(
    template: &str,
    declared: &[(&str, &str)],
    detail: &Value,
) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        if rest[open + 1..].starts_with('{') {
            out.push('{');
            rest = &rest[open + 2..];
            continue;
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or(TemplateError::Unterminated)?;
        let slot = &after[..close];
        let unit = declared
            .iter()
            .find(|(f, _)| *f == slot)
            .map(|(_, u)| *u)
            .ok_or_else(|| TemplateError::UndeclaredField {
                slot: slot.to_string(),
            })?;
        match detail.get(slot) {
            Some(Value::Null) | None => out.push_str("(n unknown)"),
            Some(Value::String(s)) => out.push_str(s),
            Some(v) => {
                out.push_str(&v.to_string());
                if !unit.is_empty() {
                    out.push(' ');
                    out.push_str(unit);
                }
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

// ── The error contract (§2.4a) ───────────────────────────────────────────────

/// A typed refusal. Half of what a model learns, it learns from being told no.
///
/// `fix` is mandatory and is **an edit the caller can make**. "Invalid argument"
/// is not a fix; "omit `venue` — it is inferred from the instrument" is. The
/// distinction is load-bearing: a wrong call is corrected, not retried, and only
/// `retryable` errors consume the loop's retry budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    /// From a closed per-namespace set, so a harness can route on it and an eval
    /// can assert on it. Free-text codes are what
    /// `an_error_code_is_snake_case_and_short` rejects.
    pub code: &'static str,
    pub message: String,
    pub fix: String,
    pub retryable: bool,
    pub retry_after_s: Option<u32>,
    pub ids: BTreeMap<String, String>,
}

/// §2.4a: errors obey the output budget — no stack traces, no SQL.
pub const MAX_ERROR_BYTES: usize = 400;

/// Every error code this crate may return (§2.4a, S-1.19).
///
/// Closed, so a harness can route on a code and an eval can assert on one. The
/// alternative is what the UI already renders around: `error.code — error.fix`,
/// with every new tool inventing its own spelling of "bad argument", and nothing
/// downstream able to tell two failures apart or two spellings together.
///
/// A code added here is a deliberate act. A code that reaches a caller without
/// being here fails `every_error_code_is_declared`, which reads the source of this
/// crate — so the way to add one is to add it, not to write a string.
pub const ERROR_CODES: &[&str] = &[
    // The gate, before any handler runs.
    "call_budget_exhausted",
    "trial_registration_failed",
    "trial_registration_unavailable",
    // A chat turn reached a trial-spending tool (AGENT-013 §2.2; S-10.2). A
    // routable code rather than prose because the desk turns it into the
    // Research offer: "start a research run for this" is an action, and an
    // action needs something to key on.
    "chat_turn_spends_no_trials",
    // A `survey` reached a trial-spending tool (AGENT-013 §3). Its own code
    // rather than the chat one: "start a research run" is the right advice for
    // a chat turn and the wrong advice for a survey, which is one already.
    "survey_spends_no_trials",
    // A look with no commitment behind it (AGENT-013 §7.2; S-10.3). Routable
    // because the desk turns it into the pre-registration card the user reads
    // *before* the run spends.
    "preregistration_required",
    // Dispatch.
    "tool_deprecated",
    "unknown_tool",
    // The audit envelope.
    "audit_unavailable",
];

/// Whether a string is shaped like a code rather than like a sentence.
///
/// `snake_case`, short enough to read in a log line, and long enough to mean
/// something. A message is prose and belongs in `message`; a code is a routing
/// decision and belongs here.
#[must_use]
pub fn is_code_shaped(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 48
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl ToolError {
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            fix: fix.into(),
            retryable: false,
            retry_after_s: None,
            ids: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn retryable(mut self, after_s: Option<u32>) -> Self {
        self.retryable = true;
        self.retry_after_s = after_s;
        self
    }

    #[must_use]
    pub fn with_id(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.ids.insert(key.into(), value.into());
        self
    }

    /// The wire shape the UI already renders (`components/workspace/Panes.tsx`
    /// reads `error.code — error.fix`).
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut err = serde_json::Map::new();
        err.insert("code".into(), json!(self.code));
        err.insert("message".into(), json!(self.message));
        err.insert("fix".into(), json!(self.fix));
        err.insert("retryable".into(), json!(self.retryable));
        if let Some(s) = self.retry_after_s {
            err.insert("retry_after_s".into(), json!(s));
        }
        if !self.ids.is_empty() {
            err.insert("ids".into(), json!(self.ids));
        }
        json!({ "error": Value::Object(err) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &'static str) -> ToolSpec {
        ToolSpec::new(name, "core", Risk::Read, "d", json!({"type":"object"}))
    }

    // ── The declaration ──────────────────────────────────────────────────────

    #[test]
    fn every_catalogue_tool_has_exactly_one_spec() {
        let names = names();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(
            names, deduped,
            "a tool declared twice makes its risk ambiguous"
        );
        assert!(
            names.len() >= 60,
            "the catalogue lost tools: {}",
            names.len()
        );
    }

    /// Adversarial: the duplicate check *rejects*, and names both sites.
    ///
    /// The real catalogue having no duplicates proves nothing about whether the
    /// check works — `assert_no_duplicates` could be empty.
    #[test]
    #[should_panic(expected = "declared twice")]
    fn a_tool_declared_twice_is_rejected_naming_both_sites() {
        let mut a = spec("get_bars");
        a.namespace = "data";
        let mut b = spec("get_bars");
        b.namespace = "discovery";
        assert_no_duplicates(&[spec("list_lanes"), a, b]);
    }

    #[test]
    fn a_deprecated_tool_still_resolves_but_is_exposed_to_nobody() {
        let live = spec("x").internal();
        let dead = spec("y")
            .internal()
            .deprecated("2026-09-15", Some("x"), "superseded");

        assert!(live.exposed_to(ToolProfile::InternalAgent));
        assert!(!dead.exposed_to(ToolProfile::InternalAgent));
        assert!(!dead.exposed_to(ToolProfile::Mcp));
        // But the declaration is still there to be read, which is what an old
        // trial ticket citing it needs.
        assert_eq!(dead.name, "y");
        assert!(matches!(
            dead.status,
            ToolStatus::Deprecated {
                superseded_by: Some("x"),
                ..
            }
        ));
    }

    #[test]
    fn a_new_spec_defaults_to_the_conservative_reading() {
        let s = spec("t");
        assert_eq!(s.version, 1);
        assert_eq!(
            s.profiles,
            &[ToolProfile::Mcp],
            "a tool is not agent-exposed by accident"
        );
        assert_eq!(s.envelope, Envelope::None);
        assert_eq!(s.spends_trial, TrialKind::None);
        assert!(
            s.evidence.is_empty(),
            "nothing is citeable unless it says so"
        );
        assert_eq!(s.max_calls_per_run, None);
    }

    // ── Templates ────────────────────────────────────────────────────────────

    #[test]
    fn a_summary_renders_from_its_declared_fields() {
        let declared = [("bars", "bars"), ("ret_pct", "%"), ("instrument", "")];
        let detail = json!({ "bars": 1234, "ret_pct": 3.5, "instrument": "BTC-USD" });
        let s = render_summary(
            "{instrument}: {bars} over the window, {ret_pct}",
            &declared,
            &detail,
        )
        .unwrap();
        assert_eq!(s, "BTC-USD: 1234 bars over the window, 3.5 %");
    }

    /// Adversarial: the failure the mechanism exists to prevent.
    ///
    /// A template naming a field the schema does not define is how a summary
    /// comes to describe a number that is not there.
    #[test]
    fn a_template_naming_an_undeclared_field_is_rejected() {
        let declared = [("bars", "bars")];
        let err =
            render_summary("{bars} and {sharpe}", &declared, &json!({"bars": 1})).unwrap_err();
        assert_eq!(
            err,
            TemplateError::UndeclaredField {
                slot: "sharpe".into()
            }
        );
    }

    /// Adversarial: every template in the real catalogue names only declared
    /// fields. This is the build-time gate §2.4 asks for.
    #[test]
    fn a_summary_template_only_names_declared_fields() {
        let mut problems = Vec::new();
        for s in catalog() {
            let Some(t) = s.output.summary_template else {
                continue;
            };
            match template_slots(t) {
                Err(e) => problems.push(format!("{}: {e:?}", s.name)),
                Ok(slots) => {
                    for slot in slots {
                        if !s.output.detail_fields.iter().any(|(f, _)| *f == slot) {
                            problems.push(format!(
                                "{}: summary template names `{slot}`, which is not in \
                                 detail_fields",
                                s.name
                            ));
                        }
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn a_numeric_field_without_a_value_renders_n_unknown() {
        let declared = [("n", "")];
        let s = render_summary("observed on {n}", &declared, &json!({})).unwrap();
        assert_eq!(s, "observed on (n unknown)");
    }

    // ── ToolResult ───────────────────────────────────────────────────────────

    #[test]
    fn a_result_within_the_cap_keeps_its_detail_and_needs_no_handle() {
        let r = ToolResult::new("short")
            .with_detail(json!({"a": 1}))
            .with_n(10);
        let v = r.render_within(6_144, "work/x.json");
        assert_eq!(v.get("detail"), Some(&json!({"a": 1})));
        assert!(v.get("handle").is_none());
        assert_eq!(v.get("n"), Some(&json!(10)));
    }

    /// Adversarial: the smallest profile's cap (6,144 B, AGENT-006 §10.9) always
    /// yields a handle rather than a silently truncated body.
    #[test]
    fn a_result_over_the_smallest_cap_always_carries_a_handle() {
        let big: Vec<u32> = (0..5_000).collect();
        let r = ToolResult::new("a summary that survives").with_detail(json!({ "rows": big }));
        let v = r.render_within(6_144, "work/describe_returns-step7.json");
        assert_eq!(
            v.get("handle"),
            Some(&json!("work/describe_returns-step7.json"))
        );
        assert!(
            v.get("detail").is_none(),
            "detail must not be truncated in place"
        );
        assert_eq!(v.get("summary"), Some(&json!("a summary that survives")));
        assert!(serde_json::to_string(&v).unwrap().len() <= 6_144);
    }

    /// And the degenerate case: a cap too small even for the summary still
    /// returns an answer, because a tool that returns nothing is
    /// indistinguishable from one that failed and the model will retry it.
    #[test]
    fn a_cap_smaller_than_the_summary_truncates_rather_than_returning_nothing() {
        let r = ToolResult::new("x".repeat(500)).with_detail(json!({"a": 1}));
        let v = r.render_within(120, "work/x.json");
        let encoded = serde_json::to_string(&v).unwrap();
        assert!(encoded.len() <= 120, "{} bytes", encoded.len());
        assert!(v
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()));
    }

    // ── Errors ───────────────────────────────────────────────────────────────

    #[test]
    fn an_error_says_what_to_change_and_whether_to_retry() {
        let e = ToolError::new(
            "window_exceeds_coverage",
            "BTC-USD 1h has bars from 2024-03-01; you asked from 2023-01-01.",
            "start at 2024-03-01, or set auto_collect: true",
        )
        .with_id("instrument_id", "BTC-USD");
        let v = e.to_value();
        let err = v.get("error").unwrap();
        assert_eq!(err.get("code"), Some(&json!("window_exceeds_coverage")));
        assert_eq!(err.get("retryable"), Some(&json!(false)));
        assert!(err
            .get("fix")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("start at"));
        assert!(serde_json::to_string(&v).unwrap().len() <= MAX_ERROR_BYTES);
    }

    /// Adversarial: a free-text code is rejected. §2.4a's codes are a closed set
    /// a harness routes on; a sentence is not routable.
    #[test]
    fn an_error_code_is_snake_case_and_short() {
        assert!(is_code_shaped("window_exceeds_coverage"));
        assert!(
            !is_code_shaped("Invalid argument"),
            "free text is not a routable code"
        );
        assert!(!is_code_shaped(
            "the window you asked for exceeds the coverage we have on file"
        ));
        assert!(!is_code_shaped(""));
    }

    /// Every declared code is shaped like one. This is the cheap half.
    #[test]
    fn every_declared_error_code_is_shaped_like_a_code() {
        for code in ERROR_CODES {
            assert!(is_code_shaped(code), "{code} is not a routable code");
        }
        let mut sorted = ERROR_CODES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ERROR_CODES.len(),
            "a code is declared twice, so two failures render as one"
        );
    }

    /// And the half that matters: a code CONSTRUCTED anywhere in this crate is a
    /// declared one. A free-text code fails here, which is the build.
    ///
    /// Read from the source rather than exercised, because reaching every error
    /// path needs a live platform for all of them — and the property is about
    /// what the source is allowed to contain, which is exactly what a scan can
    /// answer.
    #[test]
    fn every_error_code_this_crate_constructs_is_declared() {
        // Every file that can build one. Named rather than globbed: a new file
        // that constructs errors has to be added here, which is the moment to
        // notice it is doing so.
        const SOURCES: &[(&str, &str)] = &[
            ("spec.rs", include_str!("spec.rs")),
            ("lib.rs", include_str!("lib.rs")),
            ("gate.rs", include_str!("gate.rs")),
        ];
        let mut undeclared: Vec<String> = Vec::new();
        for (file, src) in SOURCES {
            // Production code only, and this file most of all: the scan's own
            // source contains the string it searches for, and the fixtures below
            // construct deliberately-invalid codes to prove the shape check
            // rejects them. Counting either would make the test fail on itself.
            let src = src.replace("\r\n", "\n");
            let src = match src.find("\n#[cfg(test)]") {
                Some(i) => src[..i].to_string(),
                None => src,
            };
            for (i, _) in src.match_indices("ToolError::new(") {
                let rest = &src[i + "ToolError::new(".len()..];
                // The code is the first string literal after the paren, whether it
                // is on that line or the next.
                let Some(open) = rest.find('"') else { continue };
                // Anything but whitespace between them means this is not a literal
                // code — a variable, say — and that is itself a violation.
                let gap = &rest[..open];
                if !gap.trim().is_empty() {
                    undeclared.push(format!("{file}: a computed error code"));
                    continue;
                }
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else {
                    continue;
                };
                let code = &after[..close];
                if !ERROR_CODES.contains(&code) {
                    undeclared.push(format!("{file}: `{code}`"));
                }
            }
        }
        assert!(
            undeclared.is_empty(),
            "error codes constructed but not declared in ERROR_CODES: {undeclared:?}.              Add them there — a code nothing can route on is a sentence."
        );
    }

    // ── rings ───────────────────────────────────────────────────────────────

    /// Every tool gets one of exactly three rings, and the desk indexes all three.
    #[test]
    fn every_tool_reports_a_ring_the_surface_knows() {
        for spec in catalog() {
            let r = spec.ring();
            assert!(
                [RING_RESEARCH, RING_PLATFORM_ARTIFACTS, RING_PAPER].contains(&r),
                "`{}` reports the ring `{r}`, which `lib/desk/rings.ts` cannot index — so the                  desk would compare it against `undefined` and let it through",
                spec.name
            );
        }
    }

    /// The direction that matters: nothing that writes may claim to reach nothing.
    ///
    /// The desk lets a Chat turn offer any ring-0 tool. A write reporting
    /// `research` would therefore be clickable in a turn that may not write, and
    /// the failure would be a successful call rather than an error.
    #[test]
    fn nothing_that_writes_claims_the_innermost_ring() {
        for spec in catalog() {
            if matches!(spec.risk, Risk::Write | Risk::Destructive) {
                assert_ne!(
                    spec.ring(),
                    RING_RESEARCH,
                    "`{}` is {:?} and reports ring 0, which the desk offers inside a Chat turn",
                    spec.name,
                    spec.risk
                );
            }
        }
    }

    /// And the converse: a read must not be pushed out to paper, or the desk
    /// hides a harmless tool and the model is told it has less than it has.
    #[test]
    fn a_read_reaches_nothing() {
        for spec in catalog() {
            if spec.risk == Risk::Read && spec.envelope != Envelope::Promotion {
                assert_eq!(spec.ring(), RING_RESEARCH, "`{}`", spec.name);
            }
        }
    }

    /// Calling a trained model is reachable from a Chat turn; training and
    /// promoting are not.
    ///
    /// The distinction is the point, and it is derived rather than declared:
    /// inference writes no version, spends no trial and changes nothing a later
    /// run resolves differently, so it is a `Read` and lands in ring 0. Training
    /// writes a version, promotion changes what live strategies resolve to.
    ///
    /// Asserted here because the mode gate filters on exactly two properties —
    /// ring and trials — and a later edit giving `call_model` a trial or an
    /// envelope would remove it from Chat silently. The failure mode this guards
    /// is the opposite one too: a `train_model` that drifted to `Read` would
    /// become callable in a Chat turn, which is the door AGENT-013 §4 closes.
    #[test]
    fn inference_is_reachable_from_chat_and_training_is_not() {
        let by_name = |n: &str| {
            catalog()
                .iter()
                .find(|s| s.name == n)
                .unwrap_or_else(|| panic!("`{n}` is not in the catalogue"))
        };

        let call = by_name("call_model");
        assert_eq!(
            call.ring(),
            RING_RESEARCH,
            "call_model must be ring 0 or a Chat turn cannot name it"
        );
        assert_eq!(
            call.spends_trial,
            TrialKind::None,
            "call_model must spend no trial: Chat spends none, and a trial here would hide \
             inference behind Research"
        );
        assert_eq!(
            call.envelope,
            Envelope::None,
            "call_model must carry no envelope: asking a finished model what it thinks is not \
             an action a human needs to approve"
        );

        for name in ["train_model", "promote_model_version"] {
            let spec = by_name(name);
            let reachable_from_chat =
                spec.ring() == RING_RESEARCH && spec.spends_trial == TrialKind::None;
            assert!(
                !reachable_from_chat,
                "`{name}` is reachable from a Chat turn (ring {}, trial {:?}). Training and \
                 promotion are writes; Chat is a read-only mode.",
                spec.ring(),
                spec.spends_trial
            );
        }
    }

    /// The promotion envelope is the paper ring, and at least one tool holds it —
    /// otherwise this rule is untested prose and the ring-2 branch is dead code.
    #[test]
    fn the_promotion_envelope_is_the_only_door_to_paper() {
        let paper: Vec<&str> = catalog()
            .iter()
            .filter(|s| s.ring() == RING_PAPER)
            .map(|s| s.name)
            .collect();
        assert!(
            !paper.is_empty(),
            "no tool reaches paper, so the ring-2 branch is never taken and this derivation is              asserting nothing"
        );
        for spec in catalog() {
            assert_eq!(
                spec.ring() == RING_PAPER,
                spec.envelope == Envelope::Promotion,
                "`{}` disagrees: ring {} against envelope {:?}. The two must move together, or                  the gate a human sees and the ring the desk filters on are different claims.",
                spec.name,
                spec.ring(),
                spec.envelope
            );
        }
    }
}
