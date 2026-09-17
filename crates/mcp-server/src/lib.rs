//! MCP server library — thin front door to the strategy platform.
//!
//! Per ADR-0010, every tool that touches platform state goes through the
//! platform's own authenticated HTTP API (`ApiClient`); this crate holds no
//! database pools or managers of its own. Only the incremental draft builder
//! keeps process-local state (a scratchpad that persists nothing).
//!
//! No privileged path; no order-placement tool.
//!
//! # Every dispatch is on the record
//!
//! [`dispatch_tool`] is the one function all three front doors go through — the
//! external MCP process, the SDK driver and the GOVERNOR loop — and it cannot
//! run a tool without the platform having opened an audit record for it first
//! (SPEC §15, ADR-P2-21, ADR-P2-34, AT-64). The record is opened *by the
//! platform*, not written here: this crate asks, and is told the id. An audit a
//! client writes is an audit a client can decline to write.
//!
//! The seal is [`AuditEnvelope`], which only [`AuditSink::open`] can mint, and
//! [`McpContext`]'s private `audit` field, which means a context — and therefore
//! a dispatch — cannot exist without one. A new dispatch path that forgets to
//! audit does not fail a test; it fails to compile.

// The tool-definitions json! literal is large enough to exceed the default
// macro recursion limit.
#![recursion_limit = "256"]

pub mod gate;
pub mod idempotency;
pub mod spec;
pub mod taxonomy;
pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use tools::builder::StrategyDraft;

/// The full authoring protocol document served by `get_authoring_guide`.
pub fn authoring_guide() -> &'static str {
    include_str!("../docs/authoring_guide.md")
}

/// Short server instructions surfaced in the MCP `initialize` response.
pub fn server_instructions() -> &'static str {
    "This server exposes a trading-strategy research platform: design trading \
     algorithms as canonical JSON strategy definitions, backtest them against real \
     historical market data, read the results, and iterate. There is deliberately \
     no order-placement tool — this server never trades.\n\
     \n\
     Workflow: list_instruments (what data exists) → get_bars (inspect the \
     market) → write a definition → validate_strategy → create_strategy \
     (upserts by strategy_id slug; use versioned slugs like my_strat_v2 when \
     iterating) → create_experiment → run_sweep → wait_for_backtest (call \
     repeatedly for long runs; each call blocks up to timeout_seconds and \
     returns progress on timeout) → read results → compare_backtests across \
     variants → iterate. Every run is registered as a counted trial before it \
     executes; there is no way to run one off the books.\n\
     \n\
     Backtests run in the background and can take from minutes to over an hour. \
     Expressions use a tiny frozen grammar (feature('ema_7') > feature('ema_21')), \
     and the engine computes far more than moving averages — volatility \
     estimators, channels, ranks, calendar terms and more. Call list_features to \
     see what it will actually resolve; do not assume. If an instrument has no \
     stored history, init_asset onboards it (background seeding job).\n\
     \n\
     Call get_authoring_guide before authoring anything."
}

// ── HTTP client to the platform API ──────────────────────────────────────────

/// Error from a platform API call.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status, `None` for transport-level failures.
    pub status: Option<u16>,
    /// Response body (or a synthesized `{ "error": ... }` object).
    pub body: Value,
}

impl ApiError {
    fn network(e: impl std::fmt::Display) -> Self {
        Self {
            status: None,
            body: json!({ "error": "network_error", "detail": e.to_string() }),
        }
    }

    /// Render as a tool-result error object.
    pub fn to_tool_error(&self) -> Value {
        let mut obj = json!({
            "error": self
                .body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("api_error"),
            "detail": self.body,
        });
        if let (Some(status), Some(map)) = (self.status, obj.as_object_mut()) {
            map.insert("status".into(), json!(status));
        }
        obj
    }
}

/// Thin authenticated HTTP client for the platform API (ADR-0010 front door).
#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    bearer: String,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, bearer: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            bearer: bearer.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    /// Build from `PLATFORM_API_URL` (default `http://127.0.0.1:7080`) and
    /// `PLATFORM_API_TOKEN` (required).
    pub fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("PLATFORM_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:7080".to_string());
        let bearer = std::env::var("PLATFORM_API_TOKEN").map_err(|_| {
            "PLATFORM_API_TOKEN is not set — mint a service token via \
             POST /auth/service-tokens (or Settings → Credentials → API Access) \
             and export it before starting the MCP server"
                .to_string()
        })?;
        if bearer.trim().is_empty() {
            return Err("PLATFORM_API_TOKEN is empty".to_string());
        }
        Ok(Self::new(base_url, bearer))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, ApiError> {
        let resp = req
            .header("authorization", format!("Bearer {}", self.bearer))
            .send()
            .await
            .map_err(ApiError::network)?;
        let status = resp.status();
        let text = resp.text().await.map_err(ApiError::network)?;
        let body: Value = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
        };
        if status.is_success() {
            Ok(body)
        } else {
            Err(ApiError {
                status: Some(status.as_u16()),
                body: if body.is_null() {
                    json!({ "error": format!("http_{}", status.as_u16()) })
                } else {
                    body
                },
            })
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.send(self.http.get(format!("{}{path}", self.base_url)))
            .await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.send(
            self.http
                .post(format!("{}{path}", self.base_url))
                .json(&body),
        )
        .await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, ApiError> {
        self.send(self.http.delete(format!("{}{path}", self.base_url)))
            .await
    }
}

// ── The audit envelope every dispatch runs inside ────────────────────────────

/// The `pre`/`post` audit record a dispatch cannot run without
/// (SPEC §15, ADR-P2-21, ADR-P2-34, AT-64).
///
/// # Why this is not written here
///
/// The row is written by the **platform**, over the same authenticated HTTP API
/// every tool call already goes through. This process — whichever front door it
/// is — asks the platform to open a record and is told the id; it cannot choose
/// the tenant, the actor, or the approval envelope, because the platform derives
/// all three from the session the bearer token resolves to. An audit a client
/// writes is an audit a client can decline to write, and an audit a client
/// *stamps* is an audit a client can attribute to somebody else.
///
/// # Why there is no way past it
///
/// [`AuditEnvelope`] is sealed: [`AuditSink::open`] is its only constructor.
/// [`dispatch_under`] takes one by reference, so a caller that has not opened a
/// record cannot call it, and [`dispatch_tool`] opens one itself and **refuses
/// to dispatch** when the platform does not answer. There is no `AuditSink::none`,
/// no `Default`, and no flag: the sink is built inside [`McpContext::new`] from
/// the context's own `ApiClient`, and the field is private, so a context without
/// one cannot be constructed at all.
#[derive(Clone)]
pub struct AuditSink {
    api: ApiClient,
}

/// Proof that the platform opened an audit record for this call.
///
/// Sealed on purpose: only [`AuditSink::open`] mints one, so holding it *is* the
/// evidence that a `pre` row exists. Passing one to [`dispatch_under`] is how the
/// type system, rather than a convention, keeps dispatch and audit together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditEnvelope(Uuid);

impl AuditEnvelope {
    #[must_use]
    pub fn id(self) -> Uuid {
        self.0
    }
}

/// The platform would not open an audit record, so nothing was dispatched.
#[derive(Debug)]
pub struct AuditUnavailable {
    pub detail: Value,
}

impl AuditSink {
    /// The only constructor. Every record it opens is written platform-side.
    #[must_use]
    fn through_platform(api: ApiClient) -> Self {
        Self { api }
    }

    /// Ask the platform to open the `pre` half — before anyone has decided
    /// anything about the call.
    ///
    /// # Errors
    /// The platform refused or could not be reached. The caller must not
    /// dispatch: a look nobody counted is the one thing this platform claims it
    /// cannot take.
    pub async fn open(
        &self,
        action: &str,
        arguments: &Value,
    ) -> Result<AuditEnvelope, AuditUnavailable> {
        let body = json!({ "action": action, "arguments": arguments });
        match self.api.post("/api/agent/audit", body).await {
            Ok(v) => match v.get("audit_id").and_then(Value::as_str).and_then(|s| s.parse().ok()) {
                Some(id) => Ok(AuditEnvelope(id)),
                None => Err(AuditUnavailable {
                    detail: json!({ "reason": "the platform opened no audit record", "response": v }),
                }),
            },
            Err(e) => Err(AuditUnavailable { detail: e.to_tool_error() }),
        }
    }

    /// Close the record with what came back.
    ///
    /// The **verdict is the platform's**, derived from the result: a caller that
    /// classified its own refusals would be marking its own homework, and
    /// `denied` is the verdict the trail exists to make provable.
    pub async fn close(&self, envelope: AuditEnvelope, result: &Value) {
        let body = json!({ "result": result });
        if let Err(e) = self
            .api
            .post(&format!("/api/agent/audit/{}", envelope.id()), body)
            .await
        {
            tracing::error!(
                audit_id = %envelope.id(),
                error = ?e.body,
                "the audit trail's post-record failed; the pre-record stands alone"
            );
        }
    }
}

// ── Context ──────────────────────────────────────────────────────────────────

/// Shared context injected into every MCP tool call.
#[derive(Clone)]
pub struct McpContext {
    /// Authenticated client for the platform API.
    pub api: ApiClient,
    /// In-memory draft store for the step-by-step strategy builder (scratchpad;
    /// nothing here persists until `finalize_strategy` posts to the platform).
    pub draft_store: Arc<Mutex<HashMap<Uuid, StrategyDraft>>>,
    /// The trajectory every tool step of this context is logged under (SPEC §14.6).
    pub trajectory: Arc<Trajectory>,
    /// The audit trail every dispatch through this context runs inside.
    ///
    /// **Private on purpose.** A public field would let any crate build an
    /// `McpContext` with a struct literal and skip `new`, which is the one place
    /// the sink is attached. Private makes `McpContext::new` the only way to have
    /// a context at all, and therefore the only way to reach `dispatch_tool`.
    audit: AuditSink,
    /// Call budgets and the trial ledger, consulted before any handler runs
    /// (INV-16, AGENT-006 §2.4b/§2.5). One gate per run: `max_calls_per_run`
    /// means this run's budget, so a fresh context is a fresh budget.
    pub gate: Arc<gate::CallGate>,
    /// What this context has already written, so it does not write it twice
    /// (§2.4b, S-1.19).
    ///
    /// Per context, like the gate, and for the same reason: one run, one record.
    pub idempotency: Arc<idempotency::Ledger>,
    /// The run this context belongs to, which is part of every idempotency key.
    ///
    /// `Uuid::nil()` for a context with no run behind it — the MCP front door
    /// serving an external client. Keys are still unique within it, because the
    /// tool and the arguments are in the material too.
    pub run_id: Uuid,
}

impl McpContext {
    /// A context whose trials are registered through the platform API.
    ///
    /// This is the constructor every front door should use. [`Self::unledgered`]
    /// exists for the case where there is genuinely no ledger behind the door,
    /// and it does not weaken the invariant — it makes every trial-spending tool
    /// refuse.
    pub fn new(api: ApiClient) -> Self {
        let registrar = Arc::new(gate::ApiTrialRegistrar::new(api.clone()));
        Self {
            audit: AuditSink::through_platform(api.clone()),
            api,
            draft_store: Arc::new(Mutex::new(HashMap::new())),
            trajectory: Arc::new(Trajectory::new()),
            gate: Arc::new(gate::CallGate::new(registrar)),
            idempotency: Arc::new(idempotency::Ledger::default()),
            run_id: Uuid::nil(),
        }
    }

    /// A context with no trial ledger behind it: reads work and every
    /// trial-spending tool refuses with `trial_registration_unavailable`.
    ///
    /// Not a bypass — the opposite. It is the honest shape for a front door
    /// pointed at a platform that cannot register a trial, and it is what tests
    /// use when they want to prove the refusal rather than the happy path.
    pub fn unledgered(api: ApiClient) -> Self {
        Self {
            // Still audited. The ledger is what this constructor does without;
            // the audit trail is not optional on any path.
            audit: AuditSink::through_platform(api.clone()),
            api,
            draft_store: Arc::new(Mutex::new(HashMap::new())),
            trajectory: Arc::new(Trajectory::new()),
            gate: Arc::new(gate::CallGate::without_registrar()),
            idempotency: Arc::new(idempotency::Ledger::default()),
            run_id: Uuid::nil(),
        }
    }

    /// The same context, under this run's sealed pre-registration (§7.2;
    /// S-10.3).
    ///
    /// The gate is rebuilt rather than mutated because it is behind an `Arc` and
    /// shared with nothing yet at construction time — and because a gate whose
    /// registration could change mid-run would be a gate a run could open by
    /// sealing a registration after its first refusal, which is the sequence the
    /// whole mechanism exists to prevent.
    #[must_use]
    pub fn under_preregistration(mut self, prereg: gate::Prereg) -> Self {
        let registrar = Arc::new(gate::ApiTrialRegistrar::new(self.api.clone()));
        self.gate = Arc::new(gate::CallGate::new(registrar).under_preregistration(prereg));
        self
    }

    /// The same context, under this run's spending rule (§2.2, §3).
    ///
    /// Preserves the registration the context was already given, for the reason
    /// `for_chat_turn` does: the two builders are written to be used together,
    /// which is exactly the shape that loses one of them.
    #[must_use]
    pub fn spending(mut self, spending: gate::Spending) -> Self {
        let registrar = Arc::new(gate::ApiTrialRegistrar::new(self.api.clone()));
        let prereg = self.gate.preregistration().clone();
        self.gate = Arc::new(
            gate::CallGate::new(registrar)
                .under_preregistration(prereg)
                .spending(spending),
        );
        self
    }

    /// The same context, for a run that may not spend a trial (AGENT-013 §2.2;
    /// Set S task S-10.2).
    ///
    /// A builder on the ordinary constructor rather than a third one, so the
    /// ledger wiring is not duplicated and the default stays the one that
    /// spends. See `CallGate::for_chat_turn` for why the direction of the
    /// default matters.
    #[must_use]
    pub fn for_chat_turn(mut self) -> Self {
        let registrar = Arc::new(gate::ApiTrialRegistrar::new(self.api.clone()));
        // The registration this context was already given survives. Rebuilding
        // the gate from scratch here would silently drop it, and the two
        // builders are written to be used together — which is exactly the shape
        // that loses one of them.
        let prereg = self.gate.preregistration().clone();
        self.gate = Arc::new(
            gate::CallGate::new(registrar)
                .under_preregistration(prereg)
                .for_chat_turn(),
        );
        self
    }

    /// What this context already returned for this call, if anything.
    ///
    /// The step is the trajectory's current length, so a retry of the step that is
    /// running collapses and the same call at a later step does not — placing the
    /// same order twice is what a trader means when they do it twice.
    #[must_use]
    pub fn replay(&self, tool: &str, params: &Value) -> Option<Value> {
        self.idempotency.replay(&idempotency::key(
            self.run_id,
            u32::try_from(self.trajectory.steps_taken().max(0)).unwrap_or(0),
            tool,
            params,
        ))
    }

    /// Record what a write returned, so a repeat of it returns the same thing.
    pub fn remember(&self, tool: &str, params: &Value, result: &Value) {
        self.idempotency.record(
            &idempotency::key(self.run_id, u32::try_from(self.trajectory.steps_taken().max(0)).unwrap_or(0), tool, params),
            result,
        );
    }

    /// The audit trail this context dispatches inside.
    ///
    /// Callers that must record a refusal they made *themselves* — the SDK
    /// driver's policy gate denies four actions without ever dispatching them —
    /// open and close the record around their own decision and hand the envelope
    /// to [`dispatch_under`] on the path that does dispatch.
    #[must_use]
    pub fn audit(&self) -> &AuditSink {
        &self.audit
    }

    /// Record one tool step. Every step `dispatch_tool` runs is recorded
    /// automatically; harness-side tools that never reach dispatch (a driver's
    /// inline wait, its termination tool) call this themselves.
    pub async fn record_step(
        &self,
        tool_name: &str,
        arguments: &Value,
        result: &Value,
        latency_ms: u128,
        tokens: Option<(u64, u64)>,
    ) {
        let step_idx = self
            .trajectory
            .next_step
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = json!({
            "traj_id": self.trajectory.id,
            "step_idx": step_idx,
            "campaign_id": *self.trajectory.campaign_id.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
            "tool_name": tool_name,
            "tool_schema_hash": tool_schema_hash(tool_name),
            "tool_semver": TOOL_SEMVER,
            "arguments": arguments,
            "result_summary": result,
            "error": result.get("error"),
            "latency_ms": i32::try_from(latency_ms).unwrap_or(i32::MAX),
            "tokens_in": tokens.map(|t| i32::try_from(t.0).unwrap_or(i32::MAX)),
            "tokens_out": tokens.map(|t| i32::try_from(t.1).unwrap_or(i32::MAX)),
        });
        if let Err(e) = self.api.post("/api/agent/trajectory", body).await {
            self.trajectory
                .unrecorded
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tracing::warn!(traj_id = %self.trajectory.id, step_idx, tool = tool_name, error = ?e.body, "trajectory step not recorded");
        }
    }
}

/// One agent session's trajectory identity.
pub struct Trajectory {
    pub id: Uuid,
    next_step: std::sync::atomic::AtomicI32,
    /// Set when the session is working inside a campaign.
    pub campaign_id: Mutex<Option<Uuid>>,
    /// Steps whose recording failed; surfaced rather than silently lost.
    pub unrecorded: std::sync::atomic::AtomicU64,
}

impl Trajectory {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            next_step: std::sync::atomic::AtomicI32::new(0),
            campaign_id: Mutex::new(None),
            unrecorded: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn steps_taken(&self) -> i32 {
        self.next_step.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Version of the tool surface a trajectory step was taken against.
pub const TOOL_SEMVER: &str = env!("CARGO_PKG_VERSION");

/// `sha256:` over the canonical JSON of the tool's full stamped descriptor — name,
/// description, input schema, namespace and risk — so a trajectory records exactly
/// what the model was offered. Tools with no descriptor (a harness's own
/// termination tool) hash their name alone.
pub fn tool_schema_hash(tool_name: &str) -> String {
    static HASHES: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    let map = HASHES.get_or_init(|| {
        tool_definitions_for(ToolProfile::Mcp)
            .as_array()
            .map(|defs| {
                defs.iter()
                    .filter_map(|d| {
                        Some((
                            d.get("name")?.as_str()?.to_string(),
                            dataplane::content_hash(d).ok()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    map.get(tool_name).cloned().unwrap_or_else(|| {
        dataplane::content_hash(&json!({ "name": tool_name, "descriptor": null }))
            .unwrap_or_default()
    })
}

/// Whether live automations are permitted via the MCP server.
///
/// Off by default; set `MCP_ALLOW_LIVE_AUTOMATIONS=true` (or `=1`) to enable.
pub fn mcp_live_automations_allowed() -> bool {
    std::env::var("MCP_ALLOW_LIVE_AUTOMATIONS")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

/// Progress emitted by long-running tools (`wait_for_backtest`), forwarded to
/// streaming clients as MCP `notifications/progress` frames.
#[derive(Clone, Debug)]
pub struct ProgressUpdate {
    /// 0–100.
    pub progress: f64,
    pub message: String,
}

/// Dispatch an MCP JSON-RPC tool call and return the result as a JSON Value.
///
/// A result carrying a top-level `"error"` key should be surfaced to the MCP
/// client with `isError: true`.
/// A strategy definition, however the caller chose to send it.
///
/// Accepts an OBJECT or a string containing JSON, and that is not politeness — it is
/// the difference between constrained decoding protecting this argument and not.
///
/// Declared as `type: string`, the grammar guarantees the envelope is well-formed
/// JSON and guarantees nothing whatsoever about the strategy inside it: the sampler
/// is free to emit any characters between the quotes. A 7B writing a whole strategy
/// definition into a JSON string produced `JSON parse error: trailing characters at
/// line 1 column 600` on attempt after attempt, and no amount of harness quality can
/// fix that, because the harness never sees a malformed argument — it sees a
/// perfectly valid string whose contents the platform then rejects.
///
/// Declared as `type: object`, the same sampler *cannot* emit malformed JSON. The
/// structure becomes a decoding guarantee instead of a hope. This is the third
/// instance of the same bug in one session — `backtest_ids` collapsed to a string,
/// `label_horizon` left unenumerated — and they share a lesson: on a constrained
/// tier, any argument typed as a free-form string is an argument with no grammar at
/// all.
///
/// Strings are still accepted, because the UI and the human MCP client send them and
/// they are not decoding under a grammar.
fn definition_arg(params: &Value) -> String {
    match params.get("definition_json") {
        Some(Value::String(s)) => s.clone(),
        Some(v @ Value::Object(_)) => serde_json::to_string(v).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Dispatch a tool call under a fresh audit envelope, gated, and record the step
/// in the agent trajectory. There is no unaudited, no ungated and no unrecorded
/// variant.
///
/// The order is the mechanism (SPEC §15, ADR-P2-21, AT-64; AGENT-006 §2.5,
/// INV-16):
///
/// 1. the `pre` audit record — written **before** anything else, so a request the
///    platform refuses has already been recorded as having been made. That is the
///    only way a denial leaves any evidence at all;
/// 2. resolve the [`spec::ToolSpec`] — an unknown name never reaches a handler;
/// 3. [`gate::CallGate::admit`] — the call budget, then the ledger row;
/// 4. **only then** the handler;
/// 5. the `post` audit record with what the platform decided, and the ticket id
///    attached to the result, so what the model cites and what the ledger
///    recorded are the same row.
///
/// Step 4 is unreachable except through step 3, asserted structurally by
/// `the_only_call_to_the_dispatch_table_is_behind_the_gate` rather than left to
/// review.
///
/// **A dispatch the platform would not record does not happen.** If the `pre`
/// write fails, this returns `audit_unavailable` and the tool is never run. That
/// is a reversal of an earlier fail-open stance and it is deliberate
/// (ADR-P2-34): the audit store *is* the platform store, so "the trail is down
/// but the platform is up" is not a state this system has. A refusal is recorded
/// in the trajectory like any other step — a call that was refused is evidence
/// about the run, and dropping it would make the trajectory disagree with the
/// ledger about what was attempted.
pub async fn dispatch_tool(
    ctx: &McpContext,
    tool_name: &str,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
) -> Value {
    let envelope = match ctx.audit.open(tool_name, params).await {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(
                tool = tool_name,
                detail = ?e.detail,
                "refusing to dispatch: the platform opened no audit record"
            );
            return json!({
                "error": "audit_unavailable",
                "detail": e.detail,
                "fix": "the platform's audit trail must be reachable before any tool runs; \
                        this call was refused rather than run off the books"
            });
        }
    };
    let result = dispatch_under(ctx, tool_name, params, progress, &envelope).await;
    ctx.audit.close(envelope, &result).await;
    result
}

/// Dispatch under an audit envelope the caller opened and will close.
///
/// The `_envelope` argument is never read. It is a **proof obligation**: only
/// [`AuditSink::open`] can mint an [`AuditEnvelope`], so a caller that reaches
/// this function has demonstrably had a `pre` record written for the call. A new
/// dispatch path that forgets to audit does not fail a test — it fails to
/// compile.
///
/// The caller closes rather than this function, because the callers that use it
/// are the ones whose envelope has to span more than the dispatch: the SDK
/// driver opens the record before its policy gate decides, so the four §15
/// actions it refuses are recorded as asked-for and denied.
pub async fn dispatch_under(
    ctx: &McpContext,
    tool_name: &str,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
    _envelope: &AuditEnvelope,
) -> Value {
    let started = std::time::Instant::now();
    let result = match spec::find(tool_name) {
        None => json!({ "error": "unknown_tool", "tool": tool_name }),
        Some(s) if !s.status.is_active() => spec::ToolError::new(
            "tool_deprecated",
            format!("`{tool_name}` is no longer offered."),
            match s.status {
                spec::ToolStatus::Deprecated {
                    superseded_by: Some(next),
                    ..
                } => format!("call `{next}` instead"),
                _ => "there is no replacement; the capability was withdrawn".to_string(),
            },
        )
        .to_value(),
        // A write already made under this key returns its original result, and the
        // platform is not touched a second time (§2.4b, S-1.19).
        //
        // Ahead of the gate on purpose. A replay is not a new call: it must not
        // spend a call budget, must not register a second trial, and must not
        // reach a handler. Behind the gate it would do all three and still be
        // called idempotent.
        //
        // Reads are excluded. `Risk::Read` is idempotent by definition, so
        // recording one buys nothing and costs a copy of every result — and a read
        // whose answer CHANGED between two calls is information the run wants,
        // not a duplicate to collapse.
        Some(s) if s.risk != taxonomy::Risk::Read && ctx.replay(s.name, params).is_some() => {
            ctx.replay(s.name, params).unwrap_or_else(|| json!({}))
        }
        Some(s) => match ctx.gate.admit(s, params).await {
            Err(e) => e.to_value(),
            Ok(ticket) => {
                let mut out = dispatch_untraced(ctx, tool_name, params, progress).await;
                if let (Some(t), Some(obj)) = (ticket, out.as_object_mut()) {
                    obj.entry("ids")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .map(|ids| ids.insert("ticket_id".into(), json!(t.ticket_id)));
                }
                // Recorded only on success. A failed write did not happen, and
                // replaying its error would turn one transient failure into a
                // permanent one for the rest of the run.
                if s.risk != taxonomy::Risk::Read && out.get("error").is_none() {
                    ctx.remember(s.name, params, &out);
                }
                out
            }
        },
    };
    ctx.record_step(
        tool_name,
        params,
        &result,
        started.elapsed().as_millis(),
        None,
    )
    .await;
    result
}

async fn dispatch_untraced(
    ctx: &McpContext,
    tool_name: &str,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
) -> Value {
    match tool_name {
        // ── Protocol ───────────────────────────────────────────────────────────
        "get_authoring_guide" => json!({ "guide": authoring_guide() }),
        "estimate_cost" => tools::core::estimate_cost(&ctx.api, params).await,
        // Static reference data, answered here rather than over HTTP — the same
        // exception `list_lanes` takes (see `tools::venues`).
        "get_venue_constraints" => tools::venues::get_venue_constraints(params),
        "list_features" => tools::strategy::list_features(params),
        "preview_signals" => tools::strategy::preview_signals(&ctx.api, params).await,

        // ── Discovery ──────────────────────────────────────────────────────────
        "list_lanes" => {
            let lanes = tools::discovery::list_lanes();
            json!({ "lanes": lanes })
        }
        "list_instruments" => tools::discovery::list_instruments(&ctx.api).await,

        // ── Authoring ──────────────────────────────────────────────────────────
        "validate_strategy" => {
            let definition_json = definition_arg(params);
            let result = tools::authoring::validate_strategy(&definition_json);
            serde_json::to_value(result).unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        "create_strategy" => {
            let definition_json = definition_arg(params);
            tools::authoring::create_strategy(&ctx.api, &definition_json).await
        }
        "get_strategy" => tools::authoring::get_strategy(&ctx.api, params).await,
        "list_strategies" => tools::authoring::list_strategies(&ctx.api).await,
        "list_compatible_strategies" => {
            tools::authoring::list_compatible_strategies(&ctx.api, params).await
        }

        // ── Market data ────────────────────────────────────────────────────────
        "get_bars" => tools::market::get_bars(&ctx.api, params).await,
        "get_instrument" => tools::market::get_instrument(&ctx.api, params).await,
        "list_asset_classes" => tools::market::list_asset_classes(&ctx.api).await,

        // ── Asset onboarding ───────────────────────────────────────────────────
        "list_initialized_assets" => tools::assets::list_initialized_assets(&ctx.api).await,
        "init_asset" => tools::assets::init_asset(&ctx.api, params).await,
        "get_asset_init_job" => tools::assets::get_asset_init_job(&ctx.api, params).await,

        // ── Portfolio observability (read-only) ────────────────────────────────
        "get_dashboard_rollup" => tools::portfolio::get_dashboard_rollup(&ctx.api).await,
        "get_paper_activity" => tools::portfolio::get_paper_activity(&ctx.api, params).await,
        "get_trading_status" => tools::portfolio::get_trading_status(&ctx.api).await,
        "get_order" => tools::portfolio::get_order(&ctx.api, params).await,

        // ── Model registry and the training pipeline ───────────────────────────
        "list_models" => tools::models::list_models(&ctx.api).await,
        "get_model" => tools::models::get_model(&ctx.api, params).await,
        "list_feature_sets" => tools::models::list_feature_sets(&ctx.api).await,
        "train_model" => tools::models::train_model(&ctx.api, params, progress.clone()).await,
        "get_training_run" => tools::models::get_training_run(&ctx.api, params).await,
        "promote_model_version" => tools::models::promote_model_version(&ctx.api, params).await,

        // ── Strategy Builder ───────────────────────────────────────────────────
        "new_strategy_draft" => tools::builder::new_strategy_draft(ctx),
        "discard_draft" => tools::builder::discard_draft(ctx, params),
        "set_strategy_meta" => tools::builder::set_strategy_meta(ctx, params),
        "add_strategy_input" => tools::builder::add_strategy_input(ctx, params),
        "add_condition_node" => tools::builder::add_condition_node(ctx, params),
        "add_signal_node" => tools::builder::add_signal_node(ctx, params),
        "add_strategy_action" => tools::builder::add_strategy_action(ctx, params),
        "set_risk_overrides" => tools::builder::set_risk_overrides(ctx, params),
        "get_draft_summary" => tools::builder::get_draft_summary(ctx, params),
        "finalize_strategy" => tools::builder::finalize_strategy(ctx, params).await,

        // ── Backtests ──────────────────────────────────────────────────────────
        "list_backtests" => tools::backtests::list_backtests(&ctx.api, params).await,
        "get_backtest" => tools::backtests::get_backtest(&ctx.api, params).await,
        // `create_backtest` is deliberately NOT routed. Dispatching compute
        // without a registered trial row is INV-16; the sanctioned path is
        // create_experiment → run_sweep, which registers every member.
        "stop_backtest" => tools::backtests::stop_backtest(&ctx.api, params).await,
        "rerun_backtest" => tools::backtests::rerun_backtest(&ctx.api, params).await,
        "delete_backtest" => tools::backtests::delete_backtest(&ctx.api, params).await,
        "compare_backtests" => tools::backtests::compare_backtests(&ctx.api, params).await,
        "wait_for_backtest" => {
            tools::backtests::wait_for_backtest(&ctx.api, params, progress).await
        }

        // ── Automations ────────────────────────────────────────────────────────
        "list_automations" => tools::automations::list_automations_tool(&ctx.api).await,
        "create_automation" => tools::automations::create_automation(&ctx.api, params).await,
        "arm_automation" => tools::automations::arm_automation(&ctx.api, params).await,
        "disarm_automation" => tools::automations::disarm_automation(&ctx.api, params).await,

        // ── Research (FEAT-003): Experiments, sweeps, gates, diagnostics ───────
        "backtest_strategy" => {
            tools::research::backtest_strategy(&ctx.api, params, progress.clone()).await
        }
        "create_experiment" => tools::research::create_experiment(&ctx.api, params).await,
        "list_experiments" => tools::research::list_experiments(&ctx.api).await,
        "get_experiment" => tools::research::get_experiment(&ctx.api, params).await,
        "run_sweep" => tools::research::run_sweep(&ctx.api, params).await,
        "get_sweep" => tools::research::get_sweep(&ctx.api, params).await,
        "cancel_sweep" => tools::research::cancel_sweep(&ctx.api, params).await,
        "run_study" => tools::research::run_study(&ctx.api, params).await,
        "list_studies" => tools::research::list_studies(&ctx.api, params).await,
        "get_carried_forward" => tools::research::get_carried_forward(&ctx.api, params).await,
        "get_diagnostics" => tools::research::get_diagnostics(&ctx.api, params).await,
        "get_funnel" => tools::research::get_funnel(&ctx.api, params).await,
        "advance_gate" => tools::research::advance_gate(&ctx.api, params).await,
        "get_null_picker" => tools::research::get_null_picker(&ctx.api, params).await,
        "choose_null" => tools::research::choose_null(&ctx.api, params).await,

        // ── Falsification (EXTERNAL-REVIEW-2026-09 §4) and its evidence tools ──
        "define_campaign" => tools::campaign::define_campaign(&ctx.api, params).await,
        "get_campaign" => tools::campaign::get_campaign(&ctx.api, params).await,
        "preregister" => tools::lifecycle::preregister(&ctx.api, params).await,
        "hypothesise" => tools::lifecycle::hypothesise(&ctx.api, params).await,
        "design" => tools::lifecycle::design(&ctx.api, params).await,
        "propose" => tools::lifecycle::propose(&ctx.api, params).await,
        "open_question" => tools::lifecycle::open_question(&ctx.api, params).await,
        "report_kill" => tools::falsification::report_kill(&ctx.api, params).await,
        "report_evaluator_defect" => {
            tools::falsification::report_evaluator_defect(&ctx.api, params).await
        }
        "check_tradeable_frontier" => {
            tools::falsification::check_tradeable_frontier(&ctx.api, params).await
        }
        "run_oracle_test" => tools::falsification::run_oracle_test(&ctx.api, params).await,
        "get_trade_list" => tools::falsification::get_trade_list(&ctx.api, params).await,
        "probe_data_quality" => tools::falsification::probe_data_quality(&ctx.api, params).await,
        "find_similar_trials" => tools::falsification::find_similar_trials(&ctx.api, params).await,

        // ── Web access (ADR-P7-01) and the skill library (ADR-P7-03) ──────────
        "web_search" => tools::research_tools::web_search(&ctx.api, params).await,
        "fetch_url" => tools::research_tools::fetch_url(&ctx.api, params).await,
        "create_skill" => tools::research_tools::create_skill(&ctx.api, params).await,
        "list_skills" => tools::research_tools::list_skills(&ctx.api, params).await,
        "run_skill" => tools::research_tools::run_skill(&ctx.api, params).await,

        // ── Audit (S-3.6, S-3.7) ───────────────────────────────────────────────
        "deflated_sharpe" => tools::audit::deflated_sharpe(&ctx.api, params).await,
        "min_track_record" => tools::audit::min_track_record(&ctx.api, params).await,
        "check_holdout_budget" => tools::audit::check_holdout_budget(&ctx.api, params).await,
        "spend_holdout_look" => tools::audit::spend_holdout_look(&ctx.api, params).await,
        "probability_of_overfit" => tools::audit::probability_of_overfit(&ctx.api, params).await,
        "family_wise_test" => tools::audit::family_wise_test(&ctx.api, params).await,
        "metric_integrity" => tools::audit::metric_integrity(&ctx.api, params).await,

        // ── The templated tier ─────────────────────────────────────────────────
        //
        // One arm for all of them, and it has to come last: a templated tool is
        // a Python file, and adding one must not mean editing this file. The
        // guard is `find`, so a name only reaches here if the catalogue already
        // declares it — this is not a fallback that would swallow a typo.
        name if tools::templated::find(name).is_some() => {
            tools::templated::dispatch(&ctx.api, name, params).await
        }

        unknown => {
            json!({ "error": "unknown_tool", "tool": unknown })
        }
    }
}

// ── Tool definitions ─────────────────────────────────────────────────────────

/// Which tool subset a front door exposes.
///
/// `InternalAgent` is the reduced set used by the in-app agent: no draft
/// builder (the agent writes full JSON definitions) and no automations, which
/// keeps the tool context small enough for local models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolProfile {
    Mcp,
    InternalAgent,
}

/// The tools the internal-agent profile exposes, sorted.
///
/// Derived from `ToolSpec.profiles` (AGENT-006 §2.3 P2). It was a hand-written
/// array of 43 names until Set S task S-1.3, which is one more place a tool could
/// be missing from — and the direction of the miss was silent: a tool absent here
/// was simply invisible to the in-app agent, with nothing failing.
///
/// A deprecated tool is absent whatever its `profiles` still says (§2.7).
#[must_use]
pub fn internal_agent_tools() -> Vec<&'static str> {
    let mut names: Vec<&str> = spec::exposed(ToolProfile::InternalAgent)
        .iter()
        .map(|s| s.name)
        .collect();
    names.sort_unstable();
    names
}

/// The complete list of tools exposed by this MCP server (MCP profile).
pub fn tool_definitions() -> Value {
    tool_definitions_for(ToolProfile::Mcp)
}

/// Tool definitions filtered by profile.
///
/// Rendered from [`spec::catalog`], which is the only place a tool's name,
/// description, schema, namespace and risk are written down. There is no second
/// copy to disagree with, which is what retired `stamp()` — it existed to graft
/// the taxonomy's namespace and risk onto a JSON literal that did not carry them.
pub fn tool_definitions_for(profile: ToolProfile) -> Value {
    Value::Array(
        spec::exposed(profile)
            .iter()
            .map(|s| s.descriptor())
            .collect(),
    )
}

#[cfg(test)]
mod catalogue_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every dispatch arm has a spec and every spec has a dispatch arm
    /// (AGENT-006 §2.3, §10.4; Set S task S-1.4).
    ///
    /// Dispatch is a `match` on the name and stays one — generating it is an
    /// explicit non-goal (§11). What must not happen is a spec whose tool cannot
    /// be called, or an arm no profile can reach: the first is a tool the model
    /// is told about and cannot use, the second is a code path no test covers.
    ///
    /// Read from the source text rather than exercised, because exercising it
    /// needs a live `ApiClient` for all sixty.
    #[test]
    fn every_dispatch_arm_has_a_spec_and_every_spec_has_an_arm() {
        const SRC: &str = include_str!("lib.rs");
        let body = SRC
            .split_once("async fn dispatch_untraced")
            .expect("dispatch_untraced exists")
            .1
            .split_once("unknown =>")
            .expect("the catch-all arm exists")
            .0;

        let mut arms: BTreeSet<&str> = BTreeSet::new();
        for line in body.lines() {
            let t = line.trim_start();
            if !t.starts_with('"') {
                continue;
            }
            let Some(rest) = t.strip_prefix('"') else {
                continue;
            };
            let Some((name, after)) = rest.split_once('"') else {
                continue;
            };
            if after.trim_start().starts_with("=>") {
                arms.insert(name);
            }
        }
        assert!(
            arms.len() > 50,
            "the arm scan found only {}; it has drifted",
            arms.len()
        );

        // The templated tier is routed by one guarded arm rather than sixty
        // literals, so its names are covered by construction. They are added
        // here rather than exempted: the test still fails if a *templated* spec
        // stops being findable, which is the way that arm can silently stop
        // routing.
        for t in tools::templated::templates() {
            arms.insert(t.header().name.as_str());
        }

        let declared: BTreeSet<&str> = spec::names().into_iter().collect();
        let no_spec: Vec<&&str> = arms.difference(&declared).collect();
        assert!(
            no_spec.is_empty(),
            "dispatch arms with no ToolSpec — callable but invisible to routing, \
             exposure and gating: {no_spec:?}"
        );
        let no_arm: Vec<&&str> = declared.difference(&arms).collect();
        assert!(
            no_arm.is_empty(),
            "specs with no dispatch arm — the model is told about them and they \
             return unknown_tool: {no_arm:?}"
        );
    }

    /// AGENT-005 §5.4: a rendered descriptor is ≤ 1,200 bytes.
    ///
    /// The whole catalogue is sent on every hosted call, so a description is the
    /// one piece of prose in this system whose cost is multiplied by every turn
    /// of every run. The rule is: ≤ 2 sentences saying *when* to use it. The
    /// "how to think about this" paragraphs live in the guide's `## Tool notes`
    /// and are fetched by the steps that need them.
    #[test]
    fn no_tool_descriptor_exceeds_the_rich_budget() {
        let mut over: Vec<(String, usize)> = Vec::new();
        for s in spec::catalog() {
            let bytes = serde_json::to_string(&json!({
                "name": s.name,
                "description": s.description,
                "inputSchema": s.input_schema,
            }))
            .expect("descriptor serialises")
            .len();
            if bytes > 1_200 {
                over.push((s.name.to_string(), bytes));
            }
        }
        over.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
        assert!(
            over.is_empty(),
            "over the 1,200-byte rich budget (AGENT-005 §5.4) — move the prose to \
             `## Tool notes` in the authoring guide and keep when-to-use: {over:?}"
        );
    }

    /// The flat budget: the first sentence alone must carry the tool, because a
    /// flat-tier render is all a 7B sees.
    #[test]
    fn every_first_sentence_fits_the_flat_budget() {
        let mut over: Vec<(String, usize)> = Vec::new();
        for s in spec::catalog() {
            let first = s
                .description
                .split_once(". ")
                .map_or(s.description, |(a, _)| a);
            if first.len() > 400 {
                over.push((s.name.to_string(), first.len()));
            }
        }
        assert!(
            over.is_empty(),
            "first sentence over 400 B (AGENT-005 §5.4): {over:?}"
        );
    }

    /// The routing unit stays routable (AGENT-006 §2.1, §10.4;
    /// `api/src/agent/bridge.rs` `no_namespace_is_too_large_to_expose_on_a_local_step`).
    ///
    /// A namespace larger than a local step's exposure budget truncates
    /// deterministically, so some tool in it becomes permanently invisible —
    /// reachable only through `search_tools`, which the model has to think to
    /// use.
    #[test]
    fn no_namespace_holds_more_than_sixteen_tools() {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for s in spec::catalog() {
            *counts.entry(s.namespace).or_default() += 1;
        }
        let over: Vec<(&&str, &usize)> = counts.iter().filter(|(_, n)| **n > 16).collect();
        assert!(
            over.is_empty(),
            "namespaces over the 16-tool routing cap: {over:?}"
        );
    }

    /// Every `notes_section` resolves to a heading in the guide (§10.3).
    ///
    /// A pointer into documentation that does not exist is worse than no
    /// pointer: the model spends a call on `get_authoring_guide` and gets
    /// nothing, then has no reason to trust the next pointer either.
    #[test]
    fn every_notes_section_resolves_to_a_heading() {
        let guide = authoring_guide();
        let missing: Vec<&str> = spec::catalog()
            .iter()
            .filter_map(|s| s.notes_section)
            .filter(|section| !guide.contains(&format!("### {section}")))
            .collect();
        assert!(
            missing.is_empty(),
            "notes_section values with no `### <name>` heading in \
             docs/authoring_guide.md: {missing:?}"
        );
    }

    /// §2.5, the static half: a write in a namespace that can run the simulator
    /// either declares a trial kind or is on this list with a reason.
    ///
    /// The *enforcement* half is [`gate::CallGate::admit`], called by
    /// `dispatch_tool` before any handler runs (Set S task S-3.13). This test
    /// still matters because the gate can only enforce what a spec declares: a
    /// simulator-running tool that declares `TrialKind::None` would pass the gate
    /// honestly and be wrong at the declaration.
    #[test]
    fn a_write_that_can_run_the_simulator_declares_a_trial() {
        use spec::TrialKind;
        // Each entry is a write in one of the three namespaces that does not run
        // anything: it records a verdict or cancels work already counted.
        const RECORDS_ONLY: &[(&str, &str)] = &[
            (
                "create_experiment",
                "creates the container; the trials are the Studies",
            ),
            (
                "define_campaign",
                "creates the campaign and starts its driver; every trial is                  registered by the phase child that spends it (Backtest at                  BASELINE, Study at EXPERIMENT, GateAdvance at GATE), so                  declaring one here would charge the first look twice. What                  stands in for a charge at definition time is the approval: it                  carries Envelope::Spend because one call commits a whole trial                  budget",
            ),
            ("cancel_sweep", "stops work already registered"),
            (
                "stop_backtest",
                "cancels a run whose trial was registered when it started",
            ),
        ];
        let mut problems = Vec::new();
        for s in spec::catalog() {
            let risky = matches!(s.namespace, "research" | "falsification" | "backtest");
            let writes = matches!(s.risk, taxonomy::Risk::Write);
            if !(risky && writes) {
                continue;
            }
            let exempt = RECORDS_ONLY.iter().any(|(n, _)| *n == s.name);
            if s.spends_trial == TrialKind::None && !exempt {
                problems.push(format!(
                    "`{}` is a write in `{}` and declares no trial — if it cannot run \
                     the simulator, add it to RECORDS_ONLY with the reason",
                    s.name, s.namespace
                ));
            }
            if s.spends_trial != TrialKind::None && exempt {
                problems.push(format!("`{}` declares a trial and is also exempt", s.name));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// §2.5, the enforcement half, asserted **structurally**: the dispatch table
    /// has exactly one caller and that caller is behind the gate (Set S S-3.13,
    /// INV-16, board §4).
    ///
    /// A behavioural test can only prove the paths it thinks to try. This one
    /// proves there is no second path: `dispatch_untraced` is called once, from
    /// inside the `Ok(ticket)` arm of `gate.admit`. A future refactor that adds a
    /// convenient second call site — a "fast path" for reads, a test helper that
    /// skips the ledger — fails here before it can ship.
    #[test]
    fn the_only_call_to_the_dispatch_table_is_behind_the_gate() {
        const SRC: &str = include_str!("lib.rs");
        // Only the shipping half of the file: this module names the call site in
        // string literals, which are assertions about it, not entrances to it.
        let shipping = SRC
            .split_once("mod catalogue_tests {")
            .expect("the test module is the last thing in this file")
            .0;
        let sites = shipping.matches("dispatch_untraced(ctx").count();
        assert_eq!(
            sites, 1,
            "`dispatch_untraced` has {sites} call sites; every handler in this \
             crate must be reachable only through `CallGate::admit`, or a \
             trial-spending tool has an unregistered path to compute"
        );

        let dispatcher = SRC
            .split_once("pub async fn dispatch_tool")
            .expect("dispatch_tool exists")
            .1
            .split_once("async fn dispatch_untraced")
            .expect("dispatch_untraced follows it")
            .0;
        let admit = dispatcher
            .find("ctx.gate.admit(")
            .expect("dispatch_tool consults the gate");
        let call = dispatcher
            .find("dispatch_untraced(ctx")
            .expect("dispatch_tool reaches the table");
        assert!(
            admit < call,
            "the gate is consulted after the handler runs, which registers the \
             trial for work already done"
        );
    }

    /// The three tests below go through [`dispatch_under`] rather than
    /// [`dispatch_tool`], with an envelope minted here.
    ///
    /// This **is** a second mint, and AT-64 knows about it by name rather than by
    /// category — `static_invariants` asserts there is exactly one, that it is
    /// inside `#[cfg(test)]`, and that nothing else in the file forges one. The
    /// reason it has to exist: the audit
    /// `pre` record is written before anything else on the dispatch path, and
    /// `AuditSink` has exactly one constructor, which posts to the platform — by
    /// design, so there is no offline sink to reach for. Against an unreachable
    /// base URL every one of these calls therefore stops at `audit_unavailable`
    /// and never reaches the thing under test. Holding an envelope stands for the
    /// `pre` row having been written, which is what `dispatch_under` requires and
    /// all these tests need to be true.
    fn envelope() -> AuditEnvelope {
        AuditEnvelope(Uuid::nil())
    }

    /// Adversarial, end to end: a tool declaring a trial cannot reach its handler
    /// on a context with no ledger. Together with the structural test above, this
    /// is the pair the phase's definition of done asks for — the mechanism
    /// refuses, and there is no route around the mechanism.
    #[tokio::test]
    async fn a_trial_spending_tool_is_refused_on_an_unledgered_context() {
        let ctx = McpContext::unledgered(ApiClient::new("http://127.0.0.1:1", "t"));
        // `backtest_strategy` declares `TrialKind::Search`; its handler would post
        // to the platform. The refusal happens first, so the unreachable base URL
        // above is never contacted — which is itself the assertion.
        let out = dispatch_under(&ctx, "backtest_strategy", &json!({}), None, &envelope()).await;
        let err = out.get("error").expect("refused");
        assert_eq!(
            err.get("code"),
            Some(&json!("trial_registration_unavailable"))
        );
    }

    /// And the complement: a read on the same unledgered context is not blocked.
    /// A gate that refuses everything is easy and useless.
    #[tokio::test]
    async fn a_read_is_not_blocked_by_the_absence_of_a_ledger() {
        let ctx = McpContext::unledgered(ApiClient::new("http://127.0.0.1:1", "t"));
        let out = dispatch_under(&ctx, "list_lanes", &json!({}), None, &envelope()).await;
        assert!(out.get("lanes").is_some(), "{out}");
    }

    /// An unknown name is refused before dispatch rather than by the table's
    /// catch-all, so a tool that is not in the catalogue can never run even if
    /// someone adds an arm for it without a spec.
    #[tokio::test]
    async fn a_name_with_no_spec_never_reaches_the_table() {
        let ctx = McpContext::unledgered(ApiClient::new("http://127.0.0.1:1", "t"));
        let out = dispatch_under(&ctx, "x_skill", &json!({}), None, &envelope()).await;
        assert_eq!(out.get("error"), Some(&json!("unknown_tool")));
    }
}
