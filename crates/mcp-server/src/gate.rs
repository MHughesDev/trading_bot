//! The gate every tool call passes through before its handler runs.
//!
//! AGENT-006 §2.4b, §2.5; Set S tasks S-3.13, S-3.19; INV-16; board §4.
//!
//! Two budgets and one invariant live here.
//!
//! **The invariant (INV-16, decision D-S7).** A tool that declares
//! [`TrialKind::Search`] or [`TrialKind::Falsification`] runs the simulator, and
//! there is no way to run the simulator without a counted trial. Until this
//! module, `spends_trial` was a *declaration*: `lib.rs`'s
//! `a_write_that_can_run_the_simulator_declares_a_trial` said so in as many
//! words — "this asserts the declaration, not the enforcement". A declaration
//! that nothing reads is a comment. Here it is read: [`CallGate::admit`] writes
//! the ledger row **before** the handler is reached, and if it cannot, the
//! handler is never reached at all.
//!
//! The direction of that failure is the whole design. A registrar that is absent,
//! erroring, or unreachable produces a *refusal*, never a silent pass — because
//! the alternative is a run whose evidence exists and whose trial does not, which
//! is precisely the selection-bias hole the ledger was built to close. There is
//! no flag, no environment variable and no test fixture that turns this off;
//! `no_construction_of_the_gate_can_disable_trial_enforcement` in this module and
//! `the_only_call_to_the_dispatch_table_is_behind_the_gate` in `lib.rs` are what
//! keep that true as the file changes.
//!
//! **The budgets.** `spends_trial` bounds simulator runs and nothing else, so a
//! loop can exhaust a dollar budget with fifty cheap-looking dispatches without
//! ever moving the trial counter (§2.4b). [`ToolSpec::max_calls_per_run`] bounds
//! *expense*; exceeding it is a typed error naming the budget and the count, not
//! a silent refusal.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::spec::{ToolError, ToolSpec, TrialKind};

/// A future returned by a trait method, boxed so the trait stays object-safe
/// without pulling in `async-trait` for one method.
pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// What the ledger is told before the tool runs.
#[derive(Clone, Debug)]
pub struct TrialRequest<'a> {
    pub tool: &'static str,
    /// Stamped on the ticket so evidence stays comparable across a tool's
    /// behaviour changes (§2.7).
    pub tool_version: u32,
    pub kind: TrialKind,
    pub arguments: &'a Value,
    /// The run's sealed pre-registration (AGENT-013 §7.2; Set S task S-10.3).
    ///
    /// Carried on **every** ticket the run spends, so a look and the commitment
    /// it was made under are joined by the ticket rather than by a timestamp and
    /// an assumption.
    pub prereg_hash: Option<&'a str>,
}

/// The row that now exists. Its id travels back on the tool's result so the
/// model can cite the trial that produced the number.
///
/// **Not** `ledger::TrialTicket`, and deliberately not named like it. This is
/// the receipt the ledger handed back across the API — an id and a family — and
/// the sealed type is the thing the ledger itself mints and never lets out.
/// AT-19 is a name scan over `TrialTicket {`, which is what stops a second
/// module from minting one; a local mirror sharing the name would both trip that
/// scan falsely and, worse, make a real violation look exactly like this file.
/// The same reasoning is why `spec::Envelope` is reconciled by a test rather than
/// guarded by its name: there is no name-based invariant on `Envelope`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialReceipt {
    pub ticket_id: String,
    /// The family the trial was sealed into, when the caller is inside one. A
    /// family is fixed at hypothesis registration, never at call time — see
    /// `audit::deflated_sharpe`, which refuses an unsealed ticket.
    pub family_id: Option<String>,
}

/// Writes the trial row. Implemented over the platform API in production
/// (`ApiTrialRegistrar`) and over a counter in tests.
///
/// The trait exists so this crate does not depend on `crates/ledger` — and
/// through it on `sqlx` and `ed25519-dalek` — for one method, which is the same
/// reason [`crate::spec::Envelope`] is a local mirror.
pub trait TrialRegistrar: Send + Sync {
    fn register<'a>(
        &'a self,
        req: TrialRequest<'a>,
    ) -> BoxFuture<'a, Result<TrialReceipt, ToolError>>;
}

/// Registers trials through the platform's own API, which is the front door
/// every other write in this crate already uses (ADR-0010).
pub struct ApiTrialRegistrar {
    api: crate::ApiClient,
}

impl ApiTrialRegistrar {
    #[must_use]
    pub fn new(api: crate::ApiClient) -> Self {
        Self { api }
    }
}

impl TrialRegistrar for ApiTrialRegistrar {
    fn register<'a>(
        &'a self,
        req: TrialRequest<'a>,
    ) -> BoxFuture<'a, Result<TrialReceipt, ToolError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "tool": req.tool,
                "tool_version": req.tool_version,
                "kind": trial_kind_str(req.kind),
                "arguments": req.arguments,
            });
            match self.api.post("/api/ledger/trials", body).await {
                Ok(v) => {
                    let ticket_id = v
                        .get("ticket_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if ticket_id.is_empty() {
                        // A 200 with no id is not a registered trial. Treating it
                        // as one would be exactly the silent pass this module
                        // exists to prevent.
                        return Err(registration_failed("the ledger returned no ticket_id"));
                    }
                    Ok(TrialReceipt {
                        ticket_id,
                        family_id: v
                            .get("family_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    })
                }
                Err(e) => Err(registration_failed(
                    e.body
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("the ledger did not accept the trial"),
                )),
            }
        })
    }
}

/// The wire spelling of a trial kind. `Falsification` is separate because
/// `audit.selection_bias_verdict` folds it into N_eff differently: an attack on a
/// strategy is not a search for one.
#[must_use]
pub fn trial_kind_str(kind: TrialKind) -> &'static str {
    match kind {
        TrialKind::None => "none",
        TrialKind::Search => "search",
        TrialKind::Falsification => "falsification",
    }
}

fn registration_failed(why: &str) -> ToolError {
    ToolError::new(
        "trial_registration_failed",
        format!("the trial was not registered: {why}"),
        "retry once the ledger is reachable; this run cannot proceed unregistered",
    )
    .retryable(Some(15))
}

/// The per-run gate: call budgets, then the trial row, then the handler.
///
/// One gate per run. `calls` is per-tool for this run only, which is what
/// `max_calls_per_run` means — a fresh run gets a fresh budget.
pub struct CallGate {
    registrar: Option<Arc<dyn TrialRegistrar>>,
    calls: Mutex<BTreeMap<&'static str, u32>>,
    /// Whether this run is a Chat turn (AGENT-013 §2.2; Set S task S-10.2).
    ///
    /// A Chat turn spends no trials, ever. The primary mechanism is exposure:
    /// ring 0 plus `spends_trial` filtering means the model never sees such a
    /// tool and cannot name one. This is the second mechanism, and it exists
    /// because the two have different failure modes — exposure is a routing
    /// property that a future code path could route around, and this is a
    /// refusal at the door every handler goes through.
    ///
    /// Belt and braces is the spec's own phrasing for this pair, and the reason
    /// is worth restating: the thing being prevented is a Chat turn quietly
    /// consuming the trial budget that makes every selection-adjusted statistic
    /// on this platform mean anything.
    chat_turn: bool,
    /// What this run has committed to before looking (§7.2; S-10.3).
    prereg: Prereg,
}

/// Whether this run has to have committed before it may look.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Prereg {
    /// No registration is demanded. The default, because the gate profile
    /// decides and a gate constructed without one has been told nothing.
    #[default]
    NotRequired,
    /// The run must have sealed a registration, and this is its hash.
    Sealed(String),
    /// The run must have sealed one and has not. Every trial-spending tool
    /// refuses, naming the registration as the missing piece.
    Missing,
}

impl CallGate {
    /// A gate with a registrar. Every tool that declares a trial can run.
    #[must_use]
    pub fn new(registrar: Arc<dyn TrialRegistrar>) -> Self {
        Self {
            registrar: Some(registrar),
            calls: Mutex::new(BTreeMap::new()),
            chat_turn: false,
            prereg: Prereg::NotRequired,
        }
    }

    /// The registration this run is spending under (§7.2; S-10.3).
    ///
    /// [`Prereg::Missing`] is a *state*, not an absence: a run that should have
    /// sealed one and did not must refuse, and it must refuse differently from a
    /// run nobody asked to. Collapsing the two into `Option<String>` would make
    /// "not required" and "required and missing" the same value, which is the
    /// shape that silently lets the second behave like the first.
    #[must_use]
    pub fn under_preregistration(mut self, prereg: Prereg) -> Self {
        self.prereg = prereg;
        self
    }

    /// The same gate, for a run that may not spend a trial (§2.2).
    ///
    /// A builder rather than a second constructor so the registrar argument is
    /// not duplicated, and so the *default* is the one that spends — a default
    /// of `chat_turn: true` would silently disable the simulator for every
    /// caller that had not been updated, which is a failure nobody would read as
    /// a policy decision.
    #[must_use]
    pub fn for_chat_turn(mut self) -> Self {
        self.chat_turn = true;
        self
    }

    /// A gate with **no** registrar: reads work, and every trial-spending tool
    /// refuses.
    ///
    /// This is the honest shape for a front door that is not wired to a ledger —
    /// an MCP client pointed at a platform with no trial endpoint, say. It is
    /// deliberately not called `disabled` or `unchecked`: nothing here is
    /// disabled, and the tools that need a trial simply cannot run.
    #[must_use]
    pub fn without_registrar() -> Self {
        Self {
            registrar: None,
            calls: Mutex::new(BTreeMap::new()),
            chat_turn: false,
            prereg: Prereg::NotRequired,
        }
    }

    /// How many times this run has called `tool`.
    #[must_use]
    pub fn calls_made(&self, tool: &str) -> u32 {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tool)
            .copied()
            .unwrap_or(0)
    }

    /// Admit a call, or refuse it with a typed error.
    ///
    /// On `Ok(Some(ticket))` the ledger row already exists. On `Ok(None)` the
    /// tool declared no trial. On `Err` **the handler must not run** — that is
    /// the contract `lib.rs` keeps and
    /// `the_only_call_to_the_dispatch_table_is_behind_the_gate` proves.
    pub async fn admit(
        &self,
        spec: &'static ToolSpec,
        arguments: &Value,
    ) -> Result<Option<TrialReceipt>, ToolError> {
        // Count first, then judge. Counting after the verdict would let a tool at
        // its budget be retried forever, each attempt refused and none recorded.
        let count = {
            let mut calls = self
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = calls.entry(spec.name).or_insert(0);
            *entry = entry.saturating_add(1);
            *entry
        };
        if let Some(budget) = spec.max_calls_per_run {
            if count > budget {
                return Err(ToolError::new(
                    "call_budget_exhausted",
                    format!(
                        "`{}` is capped at {budget} calls per run; this is call {count}.",
                        spec.name
                    ),
                    format!(
                        "use a result you already have, or widen the work per call — \
                         {budget} is the whole run's budget for this tool"
                    ),
                ));
            }
        }

        if spec.spends_trial == TrialKind::None {
            return Ok(None);
        }

        // A Chat turn spends no trials (§2.2). Refused before the registrar, so
        // no row is written and no budget is consumed — a refusal that had
        // already registered the trial would be the exact cost this prevents.
        if self.chat_turn {
            return Err(ToolError::new(
                "chat_turn_spends_no_trials",
                format!(
                    "`{}` spends a trial and this is a chat turn.",
                    spec.name
                ),
                "start a research run for this question — the offer on this turn opens one                  with the same question and constraints",
            ));
        }

        // A look with no commitment behind it (§7.2). Refused before the
        // registrar for the same reason the chat-turn refusal is: a refusal that
        // had already written the ledger row would have spent the trial it was
        // refusing.
        if self.prereg == Prereg::Missing {
            return Err(ToolError::new(
                "preregistration_required",
                format!(
                    "`{}` spends a trial and this run has sealed no pre-registration.",
                    spec.name
                ),
                "call lifecycle.preregister first: the objective, the benchmark, the                  delta_practical and the falsifier for your primary hypothesis. A                  falsifier written after a result is not a falsifier",
            ));
        }

        let Some(registrar) = self.registrar.as_ref() else {
            return Err(ToolError::new(
                "trial_registration_unavailable",
                format!(
                    "`{}` runs the simulator and this front door has no trial ledger.",
                    spec.name
                ),
                "run it from a session with a ledger; there is no unregistered path",
            ));
        };
        registrar
            .register(TrialRequest {
                tool: spec.name,
                tool_version: spec.version,
                kind: spec.spends_trial,
                arguments,
                prereg_hash: match &self.prereg {
                    Prereg::Sealed(h) => Some(h.as_str()),
                    // `Missing` cannot reach here — it refused above — and
                    // `NotRequired` means no run-scale registration exists to
                    // stamp. The per-Experiment hash is still computed by the
                    // ledger itself.
                    Prereg::NotRequired | Prereg::Missing => None,
                },
            })
            .await
            .map(Some)
    }
}

impl std::fmt::Debug for CallGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallGate")
            .field("has_registrar", &self.registrar.is_some())
            .field("chat_turn", &self.chat_turn)
            .field("prereg", &self.prereg)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Envelope, ToolSpec};
    use crate::taxonomy::Risk;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn leak(spec: ToolSpec) -> &'static ToolSpec {
        Box::leak(Box::new(spec))
    }

    fn trial_tool(name: &'static str, kind: TrialKind) -> &'static ToolSpec {
        leak(ToolSpec::new(name, "falsification", Risk::Write, "d", json!({})).spends_trial(kind))
    }

    /// Records every registration, so a test can assert one happened *and* that
    /// the handler saw its id.
    #[derive(Default)]
    struct Recording {
        registered: AtomicU32,
    }

    impl TrialRegistrar for Recording {
        fn register<'a>(
            &'a self,
            req: TrialRequest<'a>,
        ) -> BoxFuture<'a, Result<TrialReceipt, ToolError>> {
            Box::pin(async move {
                let n = self.registered.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(TrialReceipt {
                    ticket_id: format!("ticket-{}-{n}", req.tool),
                    family_id: None,
                })
            })
        }
    }

    /// A ledger that is up but refusing — the realistic failure, and the one
    /// whose wrong handling is unrecoverable.
    struct Refusing;

    impl TrialRegistrar for Refusing {
        fn register<'a>(
            &'a self,
            _req: TrialRequest<'a>,
        ) -> BoxFuture<'a, Result<TrialReceipt, ToolError>> {
            Box::pin(async move { Err(registration_failed("the family is sealed")) })
        }
    }

    // ── A chat turn spends no trials (AGENT-013 §2.2; S-10.2) ───────────────

    /// The second mechanism, refusing at the door.
    ///
    /// Not a substitute for the exposure filter — the model should never see the
    /// tool — but a filter on a routing property is a filter a later code path
    /// can route around, and the thing being protected is the trial budget every
    /// selection-adjusted statistic on this platform depends on.
    #[tokio::test]
    async fn a_chat_turn_cannot_spend_a_trial_even_if_it_reaches_the_door() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone()).for_chat_turn();
        let err = gate
            .admit(trial_tool("run_study", TrialKind::Falsification), &json!({}))
            .await
            .expect_err("a chat turn spends no trials");
        assert_eq!(err.code, "chat_turn_spends_no_trials");
        assert_eq!(
            rec.registered.load(Ordering::SeqCst),
            0,
            "refused before the registrar: a refusal that had already written the row              would be the exact cost this prevents"
        );
    }

    /// And it can still read.
    ///
    /// A chat turn that could not call anything would not be a chat turn; the
    /// rule is about trials, not about tools.
    #[tokio::test]
    async fn a_chat_turn_can_still_call_a_tool_that_spends_nothing() {
        let gate = CallGate::new(Arc::new(Recording::default())).for_chat_turn();
        let read = leak(ToolSpec::new(
            "get_bars",
            "data",
            Risk::Read,
            "read bars",
            json!({}),
        ));
        assert!(gate.admit(read, &json!({})).await.expect("a read").is_none());
    }

    /// The default spends. A default of "chat" would silently disable the
    /// simulator for every caller that had not been updated — a failure nobody
    /// would read as a policy decision.
    #[tokio::test]
    async fn the_default_gate_is_not_a_chat_turn() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone());
        assert!(gate
            .admit(trial_tool("run_study", TrialKind::Falsification), &json!({}))
            .await
            .is_ok());
        assert_eq!(rec.registered.load(Ordering::SeqCst), 1);
    }

    // ── A look needs a commitment (AGENT-013 §7.2; S-10.3) ──────────────────

    /// A run that should have registered and did not cannot look.
    #[tokio::test]
    async fn a_run_with_no_sealed_registration_cannot_spend_a_trial() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone()).under_preregistration(Prereg::Missing);
        let err = gate
            .admit(trial_tool("run_study", TrialKind::Falsification), &json!({}))
            .await
            .expect_err("no commitment, no look");
        assert_eq!(err.code, "preregistration_required");
        assert!(
            err.fix.contains("falsifier"),
            "the fix names the part that matters: {}",
            err.fix
        );
        assert_eq!(
            rec.registered.load(Ordering::SeqCst),
            0,
            "a refusal that had written the row would have spent the trial it refused"
        );
    }

    /// Every ticket the run spends carries the hash.
    #[tokio::test]
    async fn the_registration_hash_rides_on_every_ticket() {
        #[derive(Default)]
        struct Capturing {
            seen: Mutex<Vec<Option<String>>>,
        }
        impl TrialRegistrar for Capturing {
            fn register<'a>(
                &'a self,
                req: TrialRequest<'a>,
            ) -> BoxFuture<'a, Result<TrialReceipt, ToolError>> {
                self.seen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(req.prereg_hash.map(str::to_string));
                Box::pin(async move {
                    Ok(TrialReceipt {
                        ticket_id: "t1".into(),
                        family_id: None,
                    })
                })
            }
        }
        let cap = Arc::new(Capturing::default());
        let gate = CallGate::new(cap.clone())
            .under_preregistration(Prereg::Sealed("sha256:abc".into()));
        gate.admit(trial_tool("run_study", TrialKind::Falsification), &json!({}))
            .await
            .expect("admitted");
        let seen = cap
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(seen, vec![Some("sha256:abc".to_string())]);
    }

    /// "Not required" and "required and missing" are different states.
    ///
    /// Collapsing them into an `Option` is the shape that silently lets the
    /// second behave like the first.
    #[tokio::test]
    async fn a_run_nobody_asked_to_register_is_not_refused() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone());
        assert!(gate
            .admit(trial_tool("run_study", TrialKind::Falsification), &json!({}))
            .await
            .is_ok());
        assert_eq!(rec.registered.load(Ordering::SeqCst), 1);
    }

    // ── The invariant ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_trial_spending_tool_gets_its_ledger_row_before_it_runs() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone());
        let ticket = gate
            .admit(
                trial_tool("ablation_sweep", TrialKind::Falsification),
                &json!({}),
            )
            .await
            .expect("admitted")
            .expect("a trial-spending tool carries a ticket");
        assert_eq!(ticket.ticket_id, "ticket-ablation_sweep-1");
        assert_eq!(rec.registered.load(Ordering::SeqCst), 1);
    }

    /// Adversarial: the attempt to dispatch compute with no ledger behind it is
    /// **refused**, and the refusal says there is no other path.
    #[tokio::test]
    async fn a_simulator_tool_with_no_ledger_is_refused_rather_than_run_unregistered() {
        let gate = CallGate::without_registrar();
        let err = gate
            .admit(
                trial_tool("parameter_fragility_map", TrialKind::Falsification),
                &json!({}),
            )
            .await
            .expect_err("a simulator run with no trial row must be refused");
        assert_eq!(err.code, "trial_registration_unavailable");
        assert!(err.fix.contains("no unregistered path"), "{}", err.fix);
        assert!(
            !err.retryable,
            "retrying will not conjure a ledger; a retryable refusal invites a loop"
        );
    }

    /// Adversarial: a registrar that *fails* is not a registrar that is absent,
    /// and both refuse. The failure mode this rules out is the tempting one —
    /// "the ledger is down, run it anyway and reconcile later" — which produces
    /// evidence with no trial behind it.
    #[tokio::test]
    async fn a_failed_registration_refuses_the_call() {
        let gate = CallGate::new(Arc::new(Refusing));
        let err = gate
            .admit(
                trial_tool("timing_jitter_sensitivity", TrialKind::Falsification),
                &json!({}),
            )
            .await
            .expect_err("a failed registration must refuse");
        assert_eq!(err.code, "trial_registration_failed");
    }

    /// Adversarial, structural: there is no constructor, flag or environment
    /// variable that yields a gate which admits a trial-spending tool without
    /// registering it. Read from the source so a future constructor has to face
    /// this test.
    #[test]
    fn no_construction_of_the_gate_can_disable_trial_enforcement() {
        // Normalised, because `include_str!` reads the working copy and a
        // checkout under `core.autocrlf` hands back CRLF: the split below then
        // finds nothing and this test fails on line endings, not on the code.
        let src = include_str!("gate.rs").replace("\r\n", "\n");
        let body = src
            .split_once("impl CallGate {")
            .expect("CallGate has an impl block")
            .1
            .split_once("\n}\n")
            .expect("the impl block ends")
            .0;
        let body = body.to_string();
        for forbidden in ["std::env::var", "env!(", "option_env!", "cfg!(test)"] {
            assert!(
                !body.contains(forbidden),
                "`{forbidden}` in CallGate would make trial enforcement conditional; \
                 INV-16 has no exemptions"
            );
        }
        // And the only early `Ok(None)` is the one guarded by `TrialKind::None`.
        let oks = body.matches("return Ok(None)").count() + body.matches("Ok(None)\n").count();
        assert!(
            oks <= 2,
            "more than one path returns `no trial needed`; each is a place a \
             trial-spending tool could slip through"
        );
    }

    #[tokio::test]
    async fn a_read_tool_needs_no_ticket_and_no_registrar() {
        let gate = CallGate::without_registrar();
        let spec = leak(ToolSpec::new(
            "get_bars",
            "data",
            Risk::Read,
            "d",
            json!({}),
        ));
        assert_eq!(gate.admit(spec, &json!({})).await.unwrap(), None);
    }

    // ── The call budget (§2.4b) ──────────────────────────────────────────────

    #[tokio::test]
    async fn a_call_within_its_budget_is_admitted() {
        let gate = CallGate::without_registrar();
        let spec = leak(
            ToolSpec::new("run_python", "code", Risk::Write, "d", json!({})).max_calls_per_run(3),
        );
        for _ in 0..3 {
            assert!(gate.admit(spec, &json!({})).await.is_ok());
        }
        assert_eq!(gate.calls_made("run_python"), 3);
    }

    /// Adversarial: the fifty-cheap-dispatches loop §2.4b names. The error must
    /// carry both numbers — a refusal that does not say "3 of 3" is a refusal the
    /// model will simply retry.
    #[tokio::test]
    async fn exceeding_the_call_budget_is_a_typed_error_naming_budget_and_count() {
        let gate = CallGate::without_registrar();
        let spec = leak(
            ToolSpec::new("estimate_cost", "core", Risk::Read, "d", json!({})).max_calls_per_run(2),
        );
        assert!(gate.admit(spec, &json!({})).await.is_ok());
        assert!(gate.admit(spec, &json!({})).await.is_ok());
        let err = gate.admit(spec, &json!({})).await.expect_err("over budget");
        assert_eq!(err.code, "call_budget_exhausted");
        assert!(err.message.contains('2'), "{}", err.message);
        assert!(err.message.contains('3'), "{}", err.message);
        assert!(!err.retryable);
    }

    /// Adversarial: a refused call still counts. Otherwise a tool sitting at its
    /// budget is retryable forever and the budget bounds nothing.
    #[tokio::test]
    async fn a_refused_call_still_counts_against_the_budget() {
        let gate = CallGate::without_registrar();
        let spec = leak(
            ToolSpec::new("train_model", "model", Risk::Write, "d", json!({})).max_calls_per_run(1),
        );
        assert!(gate.admit(spec, &json!({})).await.is_ok());
        for expected in 2..=4 {
            let err = gate.admit(spec, &json!({})).await.expect_err("over budget");
            assert!(
                err.message.contains(&expected.to_string()),
                "call {expected} should be named: {}",
                err.message
            );
        }
        assert_eq!(gate.calls_made("train_model"), 4);
    }

    /// The budget is checked *before* the trial is registered, so an over-budget
    /// call does not burn a trial it will never use.
    #[tokio::test]
    async fn an_over_budget_trial_tool_does_not_register_a_trial() {
        let rec = Arc::new(Recording::default());
        let gate = CallGate::new(rec.clone());
        let spec = leak(
            ToolSpec::new(
                "regime_failure_search",
                "falsification",
                Risk::Write,
                "d",
                json!({}),
            )
            .spends_trial(TrialKind::Falsification)
            .max_calls_per_run(1),
        );
        assert!(gate.admit(spec, &json!({})).await.is_ok());
        assert!(gate.admit(spec, &json!({})).await.is_err());
        assert_eq!(
            rec.registered.load(Ordering::SeqCst),
            1,
            "an over-budget call must not spend a trial"
        );
    }

    #[test]
    fn a_sealed_holdout_tool_declares_its_envelope() {
        // Guards the pairing the gate cannot check itself: the envelope is
        // enforced by `ledger::audit`, and this is the local half.
        let spec = ToolSpec::new("spend_holdout_look", "audit", Risk::Write, "d", json!({}))
            .envelope(Envelope::SealedHoldout);
        assert!(spec.envelope.requires_approval());
    }
}
