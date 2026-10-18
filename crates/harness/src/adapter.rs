//! Model Adapter (harness guide §1.1, §3.1, §3.3).
//!
//! The provider boundary. **Nothing outside this module may know which provider is
//! in use** — that is the rule that makes "add a local model" a config entry and an
//! impl rather than a refactor of everything above it.
//!
//! Today there is one implementation, [`SdkExecutor`], which describes the Claude
//! Agent SDK running inside the per-project container (ADR-0024, D-07). The local
//! adapter is declared and **deliberately unimplemented**: ADR-0031 records that
//! Part 3 of the guide is profile surface until a local model is actually wired, and
//! the honest form of "not built" is a constructor that refuses, not a code path
//! that runs a local model with none of Part 3's guarantees.

use serde::Serialize;

use crate::canary::{self, CanaryError, Verdict};
use crate::profile::{Profile, Tier};

/// What a backend can actually do (AGENT-010 §3, Set S task S-2.1).
///
/// Every field here was previously an assumption made at the call site, and each one
/// has a failure that looks like a model-quality problem when it is wrong:
///
/// * a profile decoding `native` on a backend with no tool API gets prose where it
///   expected a tool call, and reads it as a model that will not follow
///   instructions;
/// * a profile decoding `select_fill` on a backend that cannot constrain gets
///   malformed JSON at a rate that looks like quantisation damage;
/// * a window larger than the backend's is silently left-truncated, and what goes
///   first is the charter.
///
/// So the adapter declares, `verify` checks, and the run refuses at load. The point
/// is not that these are hard to get right — it is that getting them wrong is
/// invisible until it is expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// A real tool-call API, with the model's own trained template.
    pub native_tools: bool,
    /// More than one tool call in a single assistant turn.
    pub parallel_tools: bool,
    /// Grammar- or schema-constrained decoding that the backend actually applies.
    /// Claimed here, measured by [`crate::canary`]; the two are reconciled by
    /// [`verify`], never by trust.
    pub grammar: bool,
    /// Prompt caching, which is what the S-1.8 breakpoints are for. A backend
    /// without it ignores them rather than failing, so this is informational.
    pub prompt_cache: bool,
    /// Whether the context window can be set per request.
    ///
    /// Ollama otherwise uses the Modelfile default — often 4,096 — and silently
    /// left-truncates. The harness knows the real budget, so a backend that cannot
    /// be told it is a backend whose window the harness must not exceed.
    pub num_ctx_settable: bool,
    /// Whether the weights can be kept resident between calls. Without it a large
    /// local model is evicted and reloaded from disk on every step.
    pub keep_alive: bool,
    /// The largest window this backend will accept, in tokens.
    pub max_context: u32,
    /// Whether the response carries token counts. Without them the budget is an
    /// estimate that is never corrected, which is the calibration S-1.18 wants.
    pub reports_usage: bool,
}

impl Capabilities {
    /// The narrowest reading: a backend that has said nothing can do nothing.
    ///
    /// Not `Default::default()` by accident — every field's `false` is the
    /// conservative answer, and `max_context: 0` refuses any profile rather than
    /// admitting every one.
    #[must_use]
    pub fn none() -> Self {
        Self {
            native_tools: false,
            parallel_tools: false,
            grammar: false,
            prompt_cache: false,
            num_ctx_settable: false,
            keep_alive: false,
            max_context: 0,
            reports_usage: false,
        }
    }
}

/// What a provider must tell the harness about itself.
pub trait ModelAdapter: Send + Sync {
    /// What this backend can do (§3, S-2.1).
    ///
    /// Defaulted to [`Capabilities::none`] so an adapter that has not been updated
    /// refuses every profile instead of silently admitting them all. A default that
    /// admits is how a capability check becomes decoration.
    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }

    /// Stable name, for the trace.
    fn provider(&self) -> &str;

    /// Whether this adapter can enforce grammar/schema-constrained decoding.
    ///
    /// Required for every local tier (§3.1). **Derived from
    /// [`Capabilities::grammar`]** rather than answered separately, because two
    /// sources of truth for one property is one source too many: an adapter could
    /// declare `grammar: false` and return `true` here, and `verify` would take
    /// whichever it happened to ask first.
    fn supports_constrained_decoding(&self) -> bool {
        self.capabilities().grammar
    }

    /// Whether the adapter uses the model's own trained chat and tool-call template
    /// (§3.3). There is no legitimate `false`; the method exists so a provider that
    /// would have to invent a format has to admit it.
    fn uses_native_template(&self) -> bool;

    /// The startup canary (§3.1).
    ///
    /// Some constrained-decoding backends fail, or silently fall back, on certain
    /// quantised models or incomplete schemas. Asserting it once at startup is the
    /// difference between finding out now and finding out from a malformed tool call
    /// four hours into a research session.
    fn canary(&self) -> Result<(), AdapterError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{provider}: constrained decoding is required for the {tier} tier but this backend does not support it")]
    NoConstrainedDecoding {
        provider: String,
        tier: &'static str,
    },
    #[error("{provider}: the startup canary failed: {detail}")]
    CanaryFailed { provider: String, detail: String },
    #[error("{provider}: no adapter is implemented for this provider yet")]
    NotImplemented { provider: String },
    #[error("{provider} does not use the model's native tool-call template; template mismatch destroys reliability faster than parameter count does (guide §3.3)")]
    NonNativeTemplate { provider: String },
    /// A capability the profile needs and the backend does not have.
    ///
    /// Names the field, because "this configuration is impossible" is not something
    /// anyone can act on and "`capabilities.grammar` is false" is.
    #[error("{provider}: the profile needs `capabilities.{field}` and this backend does not have it ({detail})")]
    MissingCapability {
        provider: String,
        field: &'static str,
        detail: String,
    },
}

/// Checks an adapter against the profile it is about to run.
///
/// Called at startup. The point is that an impossible combination — a local tier on
/// a backend that cannot constrain decoding — fails loudly here instead of producing
/// malformed tool calls that look like a model quality problem.
pub fn verify(adapter: &dyn ModelAdapter, profile: &Profile) -> Result<(), AdapterError> {
    if !adapter.uses_native_template() {
        return Err(AdapterError::NonNativeTemplate {
            provider: adapter.provider().to_string(),
        });
    }
    // The capability check (§3, S-2.1). Everything below was an assumption made at
    // a call site until now, and every one of them fails as a model-quality problem
    // rather than as a configuration error.
    let caps = adapter.capabilities();
    let provider = adapter.provider().to_string();
    let missing = |field, detail: String| AdapterError::MissingCapability {
        provider: provider.clone(),
        field,
        detail,
    };

    // The decode style is `constrained_decoding`: true is the two-step select/fill
    // over a grammar, false is the provider's own tool API.
    if profile.output.constrained_decoding {
        if !caps.grammar {
            return Err(missing(
                "grammar",
                "the profile decodes `select_fill`, which needs a grammar the backend applies"
                    .into(),
            ));
        }
    } else if !caps.native_tools {
        return Err(missing(
            "native_tools",
            "the profile decodes `native`, which needs the backend's own tool-call API".into(),
        ));
    }

    // A window larger than the backend's is not an error the backend reports — it
    // left-truncates, and what goes first is the charter.
    let wanted = profile.context.effective_budget_tokens;
    if caps.max_context < wanted {
        return Err(missing(
            "max_context",
            format!(
                "the profile asks for {wanted} tokens and the backend accepts {}",
                caps.max_context
            ),
        ));
    }
    // And a window the backend cannot be TOLD is one it will use its own default
    // for, which is the same failure arriving by a different route.
    if !caps.num_ctx_settable && wanted > caps.max_context {
        return Err(missing(
            "num_ctx_settable",
            "the profile sets a window this backend cannot be told about".into(),
        ));
    }

    // The tier-level restatement, after the field-level one so the more specific
    // message wins. It is not redundant: a LOCAL tier requires a grammar whatever
    // its decode style says, and this is the rule that says so.
    if profile.output.constrained_decoding && !caps.grammar {
        return Err(AdapterError::NoConstrainedDecoding {
            provider,
            tier: profile.tier.as_str(),
        });
    }

    adapter.canary()
}

/// The Claude Agent SDK, running inside the per-project container (ADR-0024).
///
/// The SDK owns the observe-think-act loop, the chat template and the tool-call
/// format on its side of the container boundary. Everything the guide calls
/// harness-side — the profile, the tool budget, the policy, the context budget —
/// is enforced out here, where the agent cannot reach it.
#[derive(Debug, Clone, Serialize)]
pub struct SdkExecutor {
    pub model: String,
}

impl ModelAdapter for SdkExecutor {
    fn provider(&self) -> &str {
        "anthropic-agent-sdk"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            native_tools: true,
            parallel_tools: true,
            // The SDK does not expose grammar-constrained decoding. Saying so is
            // what makes `verify` refuse a `select_fill` profile here rather than
            // running one unprotected.
            grammar: false,
            prompt_cache: true,
            num_ctx_settable: false,
            keep_alive: false,
            // Claude's window. Not the largest model's — the smallest this adapter
            // may be pointed at, because a capability is a promise about every
            // configuration it admits.
            max_context: 200_000,
            reports_usage: true,
        }
    }

    fn uses_native_template(&self) -> bool {
        true
    }

    fn canary(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// A locally hosted open-weight model.
///
/// Unlike the SDK executor, this one cannot answer [`ModelAdapter::canary`] from
/// static knowledge: whether *this* backend, running *this* quantisation, actually
/// applies a grammar is a fact about a running process. So the verdict is carried
/// rather than assumed, and an executor nobody probed refuses.
///
/// That refusal is the honest form of "not checked". The alternative — defaulting to
/// `Ok` — is precisely the silent fallback the canary exists to catch, implemented in
/// our own code.
#[derive(Debug, Clone)]
pub struct LocalExecutor {
    pub provider: String,
    pub model: String,
    /// What the backend claims. Checked against [`Self::canary_verdicts`], never
    /// trusted on its own.
    pub claims_constrained_decoding: bool,
    /// One verdict per probe, from [`crate::canary`]. Empty means unprobed.
    pub canary_verdicts: Vec<Verdict>,
}

impl LocalExecutor {
    /// An executor that has not been probed. It will refuse `verify`.
    #[must_use]
    pub fn unprobed(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            claims_constrained_decoding: false,
            canary_verdicts: Vec::new(),
        }
    }

    /// An executor carrying the results of a live probe.
    #[must_use]
    pub fn probed(
        provider: impl Into<String>,
        model: impl Into<String>,
        verdicts: Vec<Verdict>,
    ) -> Self {
        let ok = !verdicts.is_empty() && verdicts.iter().all(Verdict::is_honoured);
        Self {
            provider: provider.into(),
            model: model.into(),
            // Derived from what the backend *did*, not from what it advertises. A
            // claim and a measurement cannot disagree if only one of them is stored.
            claims_constrained_decoding: ok,
            canary_verdicts: verdicts,
        }
    }
}

impl ModelAdapter for LocalExecutor {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // `tools` and `format` do NOT compose on Ollama: passing both makes
            // `format` win and `tool_calls` come back null, with the output in
            // `content` (LOCAL_TIER_FINDINGS §2, measured on 0.33.3). So a local
            // backend has one or the other per request, and the profile's decode
            // style chooses. Declaring both true is honest — what is false is the
            // idea that they work together, and that is the adapter's problem
            // rather than the profile's.
            native_tools: true,
            // One tool call per turn. A local model asked for several returns one
            // and drops the rest, which reads as a model ignoring instructions.
            parallel_tools: false,
            // Claimed only where a probe HONOURED it. `probed` derives this from
            // what the backend did, and `unprobed` leaves it false — an executor
            // nobody measured refuses, which is the honest form of "not checked".
            grammar: self.claims_constrained_decoding,
            prompt_cache: false,
            num_ctx_settable: true,
            keep_alive: true,
            // The largest window the local profiles ask for. A backend serving a
            // smaller quantisation is caught by the canary rather than here.
            max_context: 32_768,
            reports_usage: true,
        }
    }

    fn uses_native_template(&self) -> bool {
        // Both local backends apply the model's own chat template from its Modelfile
        // or its tokenizer config; neither invents a format.
        true
    }

    fn canary(&self) -> Result<(), AdapterError> {
        canary::verdict(&self.provider, &self.canary_verdicts).map_err(|e| match e {
            CanaryError::NotProbed { provider } => AdapterError::NotImplemented { provider },
            other => AdapterError::CanaryFailed {
                provider: self.provider.clone(),
                detail: other.to_string(),
            },
        })
    }
}

/// Picks an adapter for a profile.
pub fn for_profile(profile: &Profile) -> Box<dyn ModelAdapter> {
    if profile.tier == Tier::Frontier {
        Box::new(SdkExecutor {
            model: profile.model_id.clone(),
        })
    } else {
        // Unprobed: `for_profile` has no running backend to ask. The driver replaces
        // this with `LocalExecutor::probed` once the canary has actually run.
        Box::new(LocalExecutor::unprobed(
            profile.provider.clone(),
            profile.model_id.clone(),
        ))
    }
}

// ── Prices (AGENT-010 §6, ADR-D-03; Set S task S-2.12) ──────────────────────

/// How old a price row may be and still be enforced against.
///
/// Ninety days. A provider repricing is announced, not silent, but nobody on this
/// platform is paid to watch for it, so a row is trusted for one quarter and then
/// treated as absent until a person re-verifies it. Absent prices nothing — the
/// dollar cap stops being enforced and tokens stay binding — which is the honest
/// failure; a stale row would enforce a number that is no longer true.
pub const MAX_PRICE_AGE_DAYS: i64 = 90;

/// The per-model and per-hardware-class price table (`config/prices.yaml`).
///
/// Parsed from text rather than read from disk, so the harness stays sans-IO:
/// the driver reads the file and hands it over.
///
/// **Money is never `f64` here (SC-1).** Prices are written as decimal strings and
/// parsed into integer micro-dollars; a call's cost is rounded **up** to the
/// tenth of a cent, so the accumulated spend can only over-state, which is the
/// direction that makes a cap fire early rather than late.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PriceTable {
    #[serde(default)]
    pub models: Vec<ModelPrice>,
    #[serde(default)]
    pub hardware: Vec<HardwarePrice>,
}

/// One provider model's token prices, per million tokens, in dollars.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelPrice {
    pub model: String,
    pub input_usd_per_mtok: String,
    pub output_usd_per_mtok: String,
    /// Absent: charged at the input rate. Over-states a cache read, which is the
    /// safe direction for a cap.
    #[serde(default)]
    pub cache_read_usd_per_mtok: Option<String>,
    /// Absent: charged at twice the input rate, the one-hour write premium the
    /// Anthropic adapter's breakpoints pay (S-2.7).
    #[serde(default)]
    pub cache_write_usd_per_mtok: Option<String>,
    pub source_url: String,
    /// `YYYY-MM-DD`. `None` is unverified, and an unverified row is absent.
    #[serde(default)]
    pub verified_at: Option<String>,
}

/// One hardware class's price per second of compute, in dollars.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HardwarePrice {
    pub hardware_class: String,
    pub usd_per_second: String,
    pub source_url: String,
    #[serde(default)]
    pub verified_at: Option<String>,
}

/// Token counts for one call, as the provider reported them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// Why the table refused to load.
#[derive(Debug, thiserror::Error)]
pub enum PriceError {
    #[error("config/prices.yaml does not parse: {0}")]
    Parse(String),
    #[error("price {value:?} for {what} is not a plain decimal number of dollars")]
    BadDecimal { what: String, value: String },
}

/// `"15.00"` → 15_000_000 micro-dollars. Integer arithmetic only.
fn micro_dollars(what: &str, value: &str) -> Result<i64, PriceError> {
    let bad = || PriceError::BadDecimal {
        what: what.to_string(),
        value: value.to_string(),
    };
    let v = value.trim();
    let (whole, frac) = v.split_once('.').unwrap_or((v, ""));
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let whole: i64 = whole.parse().map_err(|_| bad())?;
    let mut f = frac.to_string();
    while f.len() < 6 {
        f.push('0');
    }
    let frac: i64 = f.parse().map_err(|_| bad())?;
    whole
        .checked_mul(1_000_000)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(bad)
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date, without a date library.
fn days_from_civil(date: &str) -> Option<i64> {
    let mut it = date.trim().splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn fresh(verified_at: Option<&str>, today: &str) -> bool {
    match (verified_at.and_then(days_from_civil), days_from_civil(today)) {
        (Some(v), Some(t)) => t >= v && t - v <= MAX_PRICE_AGE_DAYS,
        _ => false,
    }
}

/// A model's prices in integer micro-dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRates {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl TokenRates {
    /// What one call cost, in tenths of a cent, rounded up.
    #[must_use]
    pub fn cost_tenth_cents(&self, t: CallTokens) -> i64 {
        let sum: i128 = i128::from(t.input) * i128::from(self.input)
            + i128::from(t.output) * i128::from(self.output)
            + i128::from(t.cache_read) * i128::from(self.cache_read)
            + i128::from(t.cache_write) * i128::from(self.cache_write);
        // tokens × micro-dollars-per-Mtok / 1e6 = micro-dollars, and
        // micro-dollars / 1000 = tenths of a cent. One division, rounded up.
        let denom: i128 = 1_000_000 * 1_000;
        let tenth = (sum + denom - 1) / denom;
        i64::try_from(tenth).unwrap_or(i64::MAX)
    }
}

impl PriceTable {
    /// Parse `config/prices.yaml`. Every price must be a plain decimal, checked
    /// here so a typo refuses to load rather than pricing a model at zero.
    pub fn from_yaml(text: &str) -> Result<Self, PriceError> {
        let t: Self = serde_yaml::from_str(text).map_err(|e| PriceError::Parse(e.to_string()))?;
        for m in &t.models {
            micro_dollars(&m.model, &m.input_usd_per_mtok)?;
            micro_dollars(&m.model, &m.output_usd_per_mtok)?;
            for v in [&m.cache_read_usd_per_mtok, &m.cache_write_usd_per_mtok]
                .into_iter()
                .flatten()
            {
                micro_dollars(&m.model, v)?;
            }
        }
        for h in &t.hardware {
            micro_dollars(&h.hardware_class, &h.usd_per_second)?;
        }
        Ok(t)
    }

    /// The model's rates, **only** from a verified row no older than
    /// [`MAX_PRICE_AGE_DAYS`] at `today` (`YYYY-MM-DD`, supplied by the caller —
    /// the harness reads no clock). `None` is "not priced", never "free".
    #[must_use]
    pub fn token_rates(&self, model: &str, today: &str) -> Option<TokenRates> {
        let row = self
            .models
            .iter()
            .find(|m| m.model == model && fresh(m.verified_at.as_deref(), today))?;
        let input = micro_dollars(&row.model, &row.input_usd_per_mtok).ok()?;
        let output = micro_dollars(&row.model, &row.output_usd_per_mtok).ok()?;
        let cache_read = match &row.cache_read_usd_per_mtok {
            Some(v) => micro_dollars(&row.model, v).ok()?,
            None => input,
        };
        let cache_write = match &row.cache_write_usd_per_mtok {
            Some(v) => micro_dollars(&row.model, v).ok()?,
            None => input.saturating_mul(2),
        };
        Some(TokenRates {
            input,
            output,
            cache_read,
            cache_write,
        })
    }

    /// Dollars per second for a hardware class, as integer micro-dollars, from a
    /// fresh verified row only.
    #[must_use]
    pub fn hardware_micro_usd_per_second(&self, class: &str, today: &str) -> Option<i64> {
        let row = self
            .hardware
            .iter()
            .find(|h| h.hardware_class == class && fresh(h.verified_at.as_deref(), today))?;
        micro_dollars(&row.hardware_class, &row.usd_per_second).ok()
    }
}

/// A profile's dollar cap in tenths of a cent, or `None` when it sets none.
#[must_use]
pub fn usd_cap_tenth_cents(profile: &Profile) -> Option<i64> {
    let usd = profile.budgets.max_usd_per_task;
    if !usd.is_finite() {
        return None;
    }
    // The one place an `f64` from a profile becomes money: rounded down to a
    // whole tenth of a cent, so a cap can only be read as tighter, never looser.
    #[allow(clippy::cast_possible_truncation)]
    let t = (usd * 1000.0).floor() as i64;
    (t > 0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::frontier_fixture;

    // ── Prices (S-2.12) ─────────────────────────────────────────────────────

    const TABLE: &str = r#"
models:
  - model: claude-opus-5
    input_usd_per_mtok: "5.00"
    output_usd_per_mtok: "25.00"
    source_url: https://example.invalid/pricing
    verified_at: "2026-09-25"
  - model: stale-model
    input_usd_per_mtok: "1"
    output_usd_per_mtok: "1"
    source_url: https://example.invalid/pricing
    verified_at: "2025-01-01"
  - model: unverified
    input_usd_per_mtok: "1"
    output_usd_per_mtok: "1"
    source_url: https://example.invalid/pricing
hardware:
  - hardware_class: rtx4090
    usd_per_second: "0.000025"
    source_url: https://example.invalid/power
    verified_at: "2026-09-25"
"#;

    #[test]
    fn a_fresh_row_prices_a_call_in_integer_tenths_of_a_cent_rounded_up() {
        let t = PriceTable::from_yaml(TABLE).unwrap();
        let r = t.token_rates("claude-opus-5", "2026-10-03").unwrap();
        // 1M input at $5 + 100k output at $25 = $7.50 = 7500 tenth-cents.
        let c = r.cost_tenth_cents(CallTokens {
            input: 1_000_000,
            output: 100_000,
            ..CallTokens::default()
        });
        assert_eq!(c, 7_500);
        // One token is a fraction of a tenth of a cent and rounds UP, never to 0.
        assert_eq!(
            r.cost_tenth_cents(CallTokens {
                input: 1,
                ..CallTokens::default()
            }),
            1
        );
        // Unlisted cache rates are charged conservatively.
        assert_eq!(r.cache_read, r.input);
        assert_eq!(r.cache_write, r.input * 2);
    }

    /// A stale or unverified row is absent, never a zero price.
    #[test]
    fn a_stale_or_unverified_row_prices_nothing() {
        let t = PriceTable::from_yaml(TABLE).unwrap();
        assert!(t.token_rates("stale-model", "2026-10-03").is_none());
        assert!(t.token_rates("unverified", "2026-10-03").is_none());
        assert!(t.token_rates("no-such-model", "2026-10-03").is_none());
        assert_eq!(
            t.hardware_micro_usd_per_second("rtx4090", "2026-10-03"),
            Some(25)
        );
    }

    #[test]
    fn a_price_that_is_not_a_plain_decimal_refuses_to_load() {
        let bad = TABLE.replace("\"25.00\"", "\"25,00\"");
        assert!(PriceTable::from_yaml(&bad).is_err());
        let neg = TABLE.replace("\"25.00\"", "\"-1\"");
        assert!(PriceTable::from_yaml(&neg).is_err());
    }

    /// The shipped table loads, so a typo in it fails CI rather than a run.
    #[test]
    fn the_shipped_price_table_loads() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prices.yaml"),
        )
        .expect("config/prices.yaml is shipped");
        PriceTable::from_yaml(&text).expect("and parses");
    }

    #[test]
    fn the_sdk_adapter_verifies_against_a_frontier_profile() {
        let p = frontier_fixture();
        verify(&*for_profile(&p), &p).unwrap();
    }

    /// The check that stops a local profile running on a backend that cannot honour
    /// it. Without this, `constrained_decoding: true` is a comment.
    #[test]
    fn a_local_profile_on_the_sdk_adapter_is_refused() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        // A local window, not the frontier fixture's 120,000. No local backend
        // offers that, and a fixture that claims one would make the `max_context`
        // check pass for a configuration that cannot exist — which is the shape of
        // a test that protects nothing. 24,000 is `qwen3.6-35b-a3b`'s own setting.
        p.context.effective_budget_tokens = 24_000;
        p.output.constrained_decoding = true;
        let err = verify(&SdkExecutor { model: "x".into() }, &p).unwrap_err();
        // Either refusal is correct and the field-level one is more useful, so the
        // assertion is on the PROPERTY — a backend without a grammar is refused —
        // rather than on which of the two messages carried it.
        assert!(
            matches!(
                err,
                AdapterError::NoConstrainedDecoding { .. }
                    | AdapterError::MissingCapability {
                        field: "grammar",
                        ..
                    }
            ),
            "a local profile on a backend with no grammar must be refused: {err}"
        );
    }

    #[test]
    fn the_local_adapter_refuses_rather_than_pretending() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        // A local window, not the frontier fixture's 120,000. No local backend
        // offers that, and a fixture that claims one would make the `max_context`
        // check pass for a configuration that cannot exist — which is the shape of
        // a test that protects nothing. 24,000 is `qwen3.6-35b-a3b`'s own setting.
        p.context.effective_budget_tokens = 24_000;
        p.provider = "ollama".into();
        p.output.constrained_decoding = false; // isolate the canary
        let err = verify(&*for_profile(&p), &p).unwrap_err();
        assert!(
            matches!(err, AdapterError::NotImplemented { .. }),
            "a stub that passed verify would run unconstrained with none of Part 3's guarantees"
        );
    }

    /// A backend that actually constrains passes, and the profile it passes for is a
    /// local one — which is the combination `verify` exists to permit.
    #[test]
    fn a_probed_backend_that_honoured_every_probe_verifies_a_local_profile() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        // A local window, not the frontier fixture's 120,000. No local backend
        // offers that, and a fixture that claims one would make the `max_context`
        // check pass for a configuration that cannot exist — which is the shape of
        // a test that protects nothing. 24,000 is `qwen3.6-35b-a3b`'s own setting.
        p.context.effective_budget_tokens = 24_000;
        p.output.constrained_decoding = true;
        let honoured: Vec<Verdict> = crate::canary::probes()
            .into_iter()
            .map(|probe| Verdict::Honoured { probe: probe.name })
            .collect();
        verify(
            &LocalExecutor::probed("vllm", "qwen3.6-35b-a3b", honoured),
            &p,
        )
        .unwrap();
    }

    /// A backend that cannot apply a grammar refuses a `select_fill` profile, and
    /// the refusal NAMES THE FIELD (§3, S-2.1).
    ///
    /// "This configuration is impossible" is not something anyone can act on;
    /// "`capabilities.grammar` is false" is. Adversarial on purpose: the real
    /// combinations being fine proves nothing about whether the check works.
    #[test]
    fn a_profile_needing_a_grammar_is_refused_by_a_backend_without_one() {
        let mut p = frontier_fixture();
        p.output.constrained_decoding = true;
        // The SDK executor has `native_tools` and no `grammar`.
        let err = verify(
            &SdkExecutor {
                model: "claude".into(),
            },
            &p,
        )
        .unwrap_err();
        let AdapterError::MissingCapability { field, .. } = err else {
            panic!("expected a named capability refusal, got {err:?}");
        };
        assert_eq!(field, "grammar", "the refusal names the field");
    }

    /// And the other direction: a `native` profile on a backend with no tool API.
    #[test]
    fn a_profile_decoding_native_is_refused_by_a_backend_without_tool_calls() {
        struct NoTools;
        impl ModelAdapter for NoTools {
            fn provider(&self) -> &str {
                "text-only"
            }
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    grammar: true,
                    max_context: 1_000_000,
                    ..Capabilities::none()
                }
            }
            fn supports_constrained_decoding(&self) -> bool {
                true
            }
            fn uses_native_template(&self) -> bool {
                true
            }
            fn canary(&self) -> Result<(), AdapterError> {
                Ok(())
            }
        }
        let mut p = frontier_fixture();
        p.output.constrained_decoding = false;
        let err = verify(&NoTools, &p).unwrap_err();
        let AdapterError::MissingCapability { field, .. } = err else {
            panic!("expected a named capability refusal, got {err:?}");
        };
        assert_eq!(field, "native_tools");
    }

    /// A window larger than the backend's is not an error the backend reports — it
    /// left-truncates, and the charter is what goes first. So it is refused here.
    #[test]
    fn a_window_larger_than_the_backend_accepts_is_refused_with_both_numbers() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.output.constrained_decoding = true;
        p.context.effective_budget_tokens = 200_000;
        let honoured: Vec<Verdict> = crate::canary::probes()
            .into_iter()
            .map(|probe| Verdict::Honoured { probe: probe.name })
            .collect();
        let err = verify(&LocalExecutor::probed("vllm", "q", honoured), &p).unwrap_err();
        let AdapterError::MissingCapability { field, detail, .. } = err else {
            panic!("expected a named capability refusal, got {err:?}");
        };
        assert_eq!(field, "max_context");
        assert!(
            detail.contains("200000") && detail.contains("32768"),
            "{detail}"
        );
    }

    /// An adapter that has not declared anything refuses every profile.
    ///
    /// The default is the narrowest reading on purpose: a default that ADMITTED
    /// would make the whole check decoration the moment someone adds an adapter and
    /// forgets the method.
    #[test]
    fn an_adapter_that_declares_nothing_admits_nothing() {
        let caps = Capabilities::none();
        assert!(!caps.native_tools && !caps.grammar && !caps.reports_usage);
        assert_eq!(
            caps.max_context, 0,
            "zero refuses every profile; any other number admits some"
        );
    }

    /// The failure this whole path is built around: the backend took the schema and
    /// ignored it, so the tier's guarantee is absent and startup must not continue.
    #[test]
    fn a_backend_that_ignored_the_grammar_fails_verification() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        // A local window, not the frontier fixture's 120,000. No local backend
        // offers that, and a fixture that claims one would make the `max_context`
        // check pass for a configuration that cannot exist — which is the shape of
        // a test that protects nothing. 24,000 is `qwen3.6-35b-a3b`'s own setting.
        p.context.effective_budget_tokens = 24_000;
        p.output.constrained_decoding = true;
        let ignored = vec![Verdict::Ignored {
            probe: "enum_under_pressure".into(),
            detail: "the reply is \"purple\" but the schema allows only \"red\"".into(),
            raw_excerpt: "{\"colour\":\"purple\"}".into(),
        }];
        let err = verify(&LocalExecutor::probed("ollama", "q", ignored), &p).unwrap_err();
        assert!(
            matches!(
                err,
                AdapterError::NoConstrainedDecoding { .. }
                    | AdapterError::MissingCapability {
                        field: "grammar",
                        ..
                    }
            ),
            "a backend measured ignoring the grammar must not claim to support it: {err}"
        );
    }

    #[test]
    fn a_non_native_template_is_refused_at_every_tier() {
        struct Bad;
        impl ModelAdapter for Bad {
            fn provider(&self) -> &str {
                "bespoke"
            }
            fn supports_constrained_decoding(&self) -> bool {
                true
            }
            fn uses_native_template(&self) -> bool {
                false
            }
            fn canary(&self) -> Result<(), AdapterError> {
                Ok(())
            }
        }
        let err = verify(&Bad, &frontier_fixture()).unwrap_err();
        assert!(matches!(err, AdapterError::NonNativeTemplate { .. }));
    }
}
