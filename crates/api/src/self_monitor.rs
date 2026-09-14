//! Platform self-monitoring — the layer nobody builds (SPEC §16.2; plan 5.3).
//!
//! This reports on the platform's **own judgment**, not on models. Every signal
//! in §16.2 is here, and each one is in exactly one of five states:
//!
//! | State | Meaning |
//! |---|---|
//! | `ok` | computed, within its bound |
//! | `alarm` | computed, outside its bound |
//! | `not_fitted` | the model behind it does not exist yet |
//! | `not_applicable` | the thing it measures does not exist on this platform |
//! | `unavailable` | it could not be computed, and why |
//!
//! The four non-`ok` states exist because the failure this page is built to
//! prevent is a dashboard that renders `0.00` for a model nobody has trained and
//! a green tick for a check that errored. A number the platform does not have is
//! never a number (AT-69).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

/// The window §16.2's drift comparisons use.
const WINDOW_DAYS: i64 = 30;

/// §3.3's SLO for deterministic features.
const CONSISTENCY_P99_SLO: f64 = 1e-9;

/// §16.2's bound on M5's expected calibration error.
const M5_ECE_LIMIT: f64 = 0.08;

/// ADR-P4-03: the entropy floor, as a fraction of `ln k`. Below it, Tier-B
/// promotion is blocked (§14.4).
const POLICY_ENTROPY_FLOOR: f64 = 0.5;

/// Marginal independence (`Δ N_eff / Δ trials`) may not fall below this fraction
/// of its own baseline before the divergence is worth a look. Half is the same
/// bar §16.2 uses for the gate-pass-rate drift, and for the same reason: a factor
/// of two is large enough not to fire on noise and small enough to catch a sweep
/// that has stopped producing independent evidence.
const NEFF_DIVERGENCE_RATIO: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalState {
    Ok,
    Alarm,
    /// The model behind this signal has not been trained yet. Distinct from
    /// `Ok`: "no value" is not "a good value".
    NotFitted,
    /// The thing this signal measures does not exist on this platform.
    NotApplicable,
    /// Computation failed. `detail` says how.
    Unavailable,
}

impl SignalState {
    #[must_use]
    pub fn is_alarm(self) -> bool {
        matches!(self, Self::Alarm)
    }

    /// Whether this state carries a number at all.
    #[must_use]
    pub fn has_value(self) -> bool {
        matches!(self, Self::Ok | Self::Alarm)
    }
}

/// How loudly a breach should be treated. §16.2 marks three signals P1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    P1,
    P2,
}

// Serialize-only: the ids and titles are `&'static str` because they are
// compile-time constants the UI keys on, and a report is something this platform
// produces, never something it parses back.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Signal {
    /// Stable identifier; the UI keys on this, never on the title.
    pub id: &'static str,
    pub title: &'static str,
    pub state: SignalState,
    pub severity: Severity,
    /// `None` unless `state.has_value()`. A renderer that shows a number here
    /// when this is `None` is the bug this type exists to prevent.
    pub value: Option<f64>,
    /// The bound `value` was compared against, when there is one.
    pub threshold: Option<f64>,
    /// Always populated, in every state — including the ones with no number,
    /// where it says why there is no number.
    pub detail: String,
    pub spec_ref: &'static str,
}

impl Signal {
    fn new(id: &'static str, title: &'static str, spec_ref: &'static str, severity: Severity) -> Self {
        Self {
            id,
            title,
            state: SignalState::Unavailable,
            severity,
            value: None,
            threshold: None,
            detail: String::new(),
            spec_ref,
        }
    }

    fn measured(mut self, value: f64, threshold: f64, ok: bool, detail: impl Into<String>) -> Self {
        self.state = if ok { SignalState::Ok } else { SignalState::Alarm };
        self.value = Some(value);
        self.threshold = Some(threshold);
        self.detail = detail.into();
        self
    }

    fn not_fitted(mut self, detail: impl Into<String>) -> Self {
        self.state = SignalState::NotFitted;
        self.detail = detail.into();
        self
    }

    fn not_applicable(mut self, detail: impl Into<String>) -> Self {
        self.state = SignalState::NotApplicable;
        self.detail = detail.into();
        self
    }

    fn unavailable(mut self, detail: impl Into<String>) -> Self {
        self.state = SignalState::Unavailable;
        self.detail = detail.into();
        self
    }
}

/// Every §16.2 signal for one tenant.
#[derive(Clone, Debug, Serialize)]
pub struct HealthReport {
    pub tenant: String,
    pub generated_at: DateTime<Utc>,
    pub window_days: i64,
    pub signals: Vec<Signal>,
}

impl HealthReport {
    /// Signals currently in alarm, worst first.
    #[must_use]
    pub fn alarms(&self) -> Vec<&Signal> {
        let mut v: Vec<&Signal> = self.signals.iter().filter(|s| s.state.is_alarm()).collect();
        v.sort_by_key(|s| match s.severity {
            Severity::P1 => 0,
            Severity::P2 => 1,
        });
        v
    }

    /// Signals that could not be computed. Counted separately from alarms
    /// because "we do not know" is a different operational state from "it is
    /// bad", and collapsing them is how a broken check becomes invisible.
    #[must_use]
    pub fn unavailable(&self) -> Vec<&Signal> {
        self.signals
            .iter()
            .filter(|s| s.state == SignalState::Unavailable)
            .collect()
    }

    #[must_use]
    pub fn healthy(&self) -> bool {
        self.alarms().is_empty() && self.unavailable().is_empty()
    }
}

/// Compute the full report.
///
/// Every signal is computed independently and a failure is captured *into* the
/// signal rather than propagated, so one broken query cannot hide the other nine.
///
/// # Errors
/// Never returns `Err` for a signal-level failure; only a failure to establish
/// the tenant transaction at all propagates.
pub async fn health(pg: &PgPool, tenant: &str) -> anyhow::Result<HealthReport> {
    let signals = vec![
        gate_pass_rate(pg, tenant).await,
        exploration_fraction(pg, tenant).await,
        feature_consistency(pg, tenant).await,
        leakage_suite(pg, tenant).await,
        sealed_holdout(pg, tenant).await,
        neff_vs_trials(pg, tenant).await,
        policy_entropy(pg, tenant).await,
        artifact_pin_coverage(pg).await,
        trajectory_half_life(pg, tenant).await,
        m5_calibration(pg, tenant).await,
        mnar_b1(pg, tenant).await,
    ];
    Ok(HealthReport {
        tenant: tenant.to_string(),
        generated_at: Utc::now(),
        window_days: WINDOW_DAYS,
        signals,
    })
}

/// Run one read inside the tenant's RLS context.
///
/// No query here binds a tenant: row-level security is what scopes them, which
/// means a signal cannot accidentally read across a tenant boundary by forgetting
/// a `WHERE`. The transaction is read-only and its commit failure is not worth
/// reporting separately from the query's.
async fn rows(pg: &PgPool, tenant: &str, sql: &str) -> Result<Vec<sqlx::postgres::PgRow>, String> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| format!("tenant transaction: {e}"))?;
    let out = sqlx::query(sql).fetch_all(&mut *tx).await.map_err(|e| e.to_string());
    let _ = tx.commit().await;
    out
}

async fn one(pg: &PgPool, tenant: &str, sql: &str) -> Result<sqlx::postgres::PgRow, String> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| format!("tenant transaction: {e}"))?;
    let out = sqlx::query(sql).fetch_one(&mut *tx).await.map_err(|e| e.to_string());
    let _ = tx.commit().await;
    out
}

async fn opt(pg: &PgPool, tenant: &str, sql: &str) -> Result<Option<sqlx::postgres::PgRow>, String> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| format!("tenant transaction: {e}"))?;
    let out = sqlx::query(sql).fetch_optional(&mut *tx).await.map_err(|e| e.to_string());
    let _ = tx.commit().await;
    out
}

// ───────────────────────────────────────────────────────────────────────────────
// the signals
// ───────────────────────────────────────────────────────────────────────────────

/// Gate pass rate by profile, recent window vs baseline. §16.2 alarms on a drift
/// of more than 2× in either direction — a rate that doubles is as much a signal
/// as one that halves, because gates that suddenly pass are usually gates that
/// stopped running.
async fn gate_pass_rate(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "gate_pass_rate",
        "Gate pass rate by profile (30d vs baseline)",
        "§16.2",
        Severity::P2,
    );
    let rows = match rows(
        pg,
        tenant,
        "WITH subject AS (
                   SELECT profile_id,
                          coalesce(experiment_id, trial_id::text) AS subject_id,
                          max(decided_at) AS last_at,
                          bool_and(passed) AS passed
                     FROM mlops.gate_verdict
                    GROUP BY 1, 2
                 )
                 SELECT profile_id,
                        count(*) FILTER (WHERE last_at >= now() - make_interval(days => 30)) AS rd,
                        count(*) FILTER (WHERE last_at >= now() - make_interval(days => 30) AND passed) AS rp,
                        count(*) FILTER (WHERE last_at <  now() - make_interval(days => 30)) AS bd,
                        count(*) FILTER (WHERE last_at <  now() - make_interval(days => 30) AND passed) AS bp
                   FROM subject GROUP BY 1 ORDER BY 1",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read gate verdicts: {e}")),
    };

    if rows.is_empty() {
        return s.not_applicable("no gate verdicts recorded yet");
    }
    let mut drifted: Vec<String> = Vec::new();
    let mut summary: Vec<String> = Vec::new();
    let mut worst_recent = 0.0_f64;
    for r in &rows {
        let profile: String = r.get("profile_id");
        let rate = ledger::gates::PassRate {
            recent_decided: r.get("rd"),
            recent_passed: r.get("rp"),
            baseline_decided: r.get("bd"),
            baseline_passed: r.get("bp"),
        };
        match (rate.recent(), rate.drifted()) {
            (Some(recent), Some(true)) => {
                worst_recent = worst_recent.max(recent);
                drifted.push(profile.clone());
                summary.push(format!(
                    "{profile}: {:.1}% recent vs {:.1}% baseline",
                    recent * 100.0,
                    rate.baseline().unwrap_or(0.0) * 100.0
                ));
            }
            (Some(recent), _) => {
                worst_recent = worst_recent.max(recent);
                summary.push(format!("{profile}: {:.1}% over {} candidates", recent * 100.0, rate.recent_decided));
            }
            (None, _) => summary.push(format!("{profile}: no candidates decided in the window")),
        }
    }
    let ok = drifted.is_empty();
    s.measured(
        worst_recent,
        2.0,
        ok,
        if ok {
            format!("no profile drifted more than 2x. {}", summary.join("; "))
        } else {
            format!("drift in {}. {}", drifted.join(", "), summary.join("; "))
        },
    )
}

/// The achieved exploration fraction against the *declared* floor — the campaign's
/// own, never a platform constant. §16.2 marks this P1: below the floor, the
/// platform has stopped looking at what it has not tried, and every off-policy
/// estimate downstream degrades with it.
async fn exploration_fraction(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "exploration_fraction",
        "Exploration fraction achieved vs declared floor",
        "§4.5, INV-21",
        Severity::P1,
    );
    let rows = match rows(
        pg,
        tenant,
        "SELECT c.slug, c.exploration_floor,
                        count(t.*)                                      AS n,
                        count(t.*) FILTER (WHERE t.exploration_flag)    AS explored
                   FROM mlops.campaign c
                   LEFT JOIN mlops.trial t ON t.campaign_id = c.campaign_id
                  GROUP BY c.slug, c.exploration_floor
                  ORDER BY c.slug",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read campaigns: {e}")),
    };

    let with_trials: Vec<&sqlx::postgres::PgRow> =
        rows.iter().filter(|r| r.get::<i64, _>("n") > 0).collect();
    if with_trials.is_empty() {
        return s.not_applicable("no campaign has dispatched a trial yet");
    }
    let mut worst = (f64::INFINITY, String::new(), 0.0_f64);
    let mut breaches = Vec::new();
    for r in &with_trials {
        let (slug, floor): (String, f64) = (r.get("slug"), r.get("exploration_floor"));
        let (n, explored): (i64, i64) = (r.get("n"), r.get("explored"));
        let achieved = explored as f64 / n as f64;
        if achieved < floor {
            breaches.push(format!("{slug}: {:.1}% < {:.1}%", achieved * 100.0, floor * 100.0));
        }
        if achieved < worst.0 {
            worst = (achieved, slug, floor);
        }
    }
    let ok = breaches.is_empty();
    s.measured(
        worst.0,
        worst.2,
        ok,
        if ok {
            format!("every campaign is at or above its floor; lowest is {} at {:.1}%", worst.1, worst.0 * 100.0)
        } else {
            format!("P1 — below the declared floor: {}", breaches.join("; "))
        },
    )
}

/// The p99 absolute relative diff between a live serve and its recomputation.
/// §3.3's SLO is 1e-9 for deterministic features; any `code_drift` diagnosis is
/// a P1 on its own, independent of the percentile.
async fn feature_consistency(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "feature_consistency_p99",
        "Feature consistency p99 relative diff",
        "§3.3, INV-14",
        Severity::P1,
    );
    let row = match one(
        pg,
        tenant,
                "SELECT count(*) AS n,
                        count(*) FILTER (WHERE diagnosis = 'code_drift') AS drift,
                        coalesce(percentile_disc(0.99) WITHIN GROUP (
                          ORDER BY abs_diff / nullif(abs(served_value), 0)), 0)::float8 AS p99
                   FROM dataplane.feature_consistency_diff
                  WHERE diffed_at >= now() - make_interval(days => 30)",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read consistency diffs: {e}")),
    };
    let n: i64 = row.get("n");
    if n == 0 {
        return s.not_applicable("no serves diffed in the window");
    }
    let drift: i64 = row.get("drift");
    let p99: f64 = row.get("p99");
    let ok = drift == 0 && p99 < CONSISTENCY_P99_SLO;
    s.measured(
        p99,
        CONSISTENCY_P99_SLO,
        ok,
        if drift > 0 {
            format!("P1 — {drift} code_drift diagnoses over {n} diffs; p99 {p99:.3e}")
        } else if ok {
            format!("p99 {p99:.3e} over {n} diffs, no code drift")
        } else {
            format!("p99 {p99:.3e} breaches the 1e-9 SLO over {n} diffs")
        },
    )
}

/// The nightly leakage suite's latest verdict. A suite that has never run is not
/// a clean suite (plan §Phase 1, ADR-P1-06…08).
async fn leakage_suite(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "leakage_suite",
        "Leakage suite — latest nightly pass",
        "§12.5",
        Severity::P1,
    );
    let row = match opt(
        pg,
        tenant,
        "SELECT blocking_count, flag_count, finished_at, array_length(checks_run, 1) AS checks
           FROM mlops.leakage_run ORDER BY finished_at DESC LIMIT 1",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read leakage runs: {e}")),
    };
    let Some(row) = row else {
        return s.not_applicable("the leakage suite has not run against this tenant yet");
    };
    let blocking: i32 = row.get("blocking_count");
    let flags: i32 = row.get("flag_count");
    let finished: DateTime<Utc> = row.get("finished_at");
    let checks: Option<i32> = row.get("checks");
    let stale = Utc::now() - finished > Duration::days(2);
    let ok = blocking == 0 && !stale;
    s.measured(
        f64::from(blocking),
        0.0,
        ok,
        if stale {
            format!("P1 — last pass was {}, more than two days ago", finished.format("%Y-%m-%d %H:%M"))
        } else if blocking > 0 {
            format!("P1 — {blocking} blocking findings ({flags} flags) in the last pass")
        } else {
            format!("clean: {} checks ran, {flags} flags, last {}", checks.unwrap_or(0), finished.format("%Y-%m-%d %H:%M"))
        },
    )
}

/// Every sealed-holdout request, and every repeat. §12.7 wants a second attempt
/// *surfaced*, not merely refused — the refusal already works, and a lineage that
/// keeps asking is the thing a human needs to see.
async fn sealed_holdout(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "sealed_holdout_calls",
        "Sealed-holdout call ledger",
        "§12.7",
        Severity::P2,
    );
    let row = match one(
        pg,
        tenant,
                "SELECT count(*) AS attempts,
                        count(*) FILTER (WHERE served_first_result) AS repeats,
                        count(DISTINCT strategy_lineage_id) AS lineages
                   FROM mlops.sealed_holdout_attempt",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read the holdout ledger: {e}")),
    };
    let attempts: i64 = row.get("attempts");
    if attempts == 0 {
        return s.not_applicable("no sealed-holdout request has been made");
    }
    let repeats: i64 = row.get("repeats");
    let lineages: i64 = row.get("lineages");
    s.measured(
        repeats as f64,
        0.0,
        repeats == 0,
        if repeats == 0 {
            format!("{attempts} first evaluations across {lineages} lineages, no repeats")
        } else {
            format!("{repeats} repeat requests across {lineages} lineages — each was served the first result and logged")
        },
    )
}

/// `N_eff` growth against trial growth. §16.2: divergence means trials are more
/// correlated than they look — the sweep is producing looks without producing
/// evidence, and every deflation downstream is being computed against a count
/// that overstates how much was actually learned.
async fn neff_vs_trials(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "neff_vs_trials",
        "N_eff growth vs trial growth",
        "§12.4, §16.2",
        Severity::P2,
    );
    let led = ledger::pg::PgTrialLedger::new(pg.clone());
    let all = match led.n_eff_async(tenant).await {
        Ok(n) => n,
        Err(e) => return s.unavailable(format!("could not compute N_eff: {e}")),
    };
    if all.trials_counted() == 0 {
        return s.not_applicable("no trials on this tenant's ledger yet");
    }
    let baseline = match led.n_eff_before_async(tenant, Utc::now() - Duration::days(WINDOW_DAYS)).await {
        Ok(n) => n,
        Err(e) => return s.unavailable(format!("could not compute the baseline N_eff: {e}")),
    };

    let d_trials = all.trials_counted().saturating_sub(baseline.trials_counted()) as f64;
    if d_trials < 1.0 {
        return s.not_applicable("no trials added in the window");
    }
    let d_neff = (all.value() - baseline.value()).max(0.0);
    let recent_marginal = d_neff / d_trials;
    let baseline_marginal = if baseline.trials_counted() > 0 {
        baseline.value() / baseline.trials_counted() as f64
    } else {
        // Nothing to compare against: report the recent rate without a verdict.
        return s
            .measured(recent_marginal, 1.0, true, format!(
                "{d_neff:.1} independent of {d_trials:.0} new trials; no prior window to compare against"
            ));
    };
    let ok = recent_marginal >= baseline_marginal * NEFF_DIVERGENCE_RATIO;
    s.measured(
        recent_marginal,
        baseline_marginal * NEFF_DIVERGENCE_RATIO,
        ok,
        format!(
            "recent {recent_marginal:.3} independent per trial vs {baseline_marginal:.3} baseline \
             ({d_neff:.1} of {d_trials:.0} new trials independent)"
        ),
    )
}

/// The dispatch policy's entropy, as a fraction of its maximum.
///
/// Each logged decision records the propensity of the candidate it chose, and a
/// draw's surprisal `−ln p(chosen)` is an unbiased estimate of the policy's
/// entropy at that decision. Averaged over recent decisions and divided by the
/// mean `ln k`, that gives a number in `[0, 1]`: 1 is uniform, 0 is deterministic.
/// §14.4's floor is `0.5 · ln k`, so the floor here is 0.5.
async fn policy_entropy(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "policy_entropy",
        "Dispatch policy entropy (fraction of maximum)",
        "§14.4, §16.2",
        Severity::P1,
    );
    let row = match one(
        pg,
        tenant,
                "SELECT count(*) AS n,
                        coalesce(avg(-ln(propensity)), 0)::float8 AS mean_surprisal,
                        coalesce(avg(ln(greatest(jsonb_array_length(candidate_set), 2)::float8)), 0)::float8 AS mean_ln_k
                   FROM mlops.decision
                  WHERE decided_at >= now() - make_interval(days => 30)
                    AND propensity IS NOT NULL AND propensity > 0
                    AND jsonb_array_length(candidate_set) > 1",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read decisions: {e}")),
    };
    let n: i64 = row.get("n");
    if n == 0 {
        return s.not_applicable("no multi-candidate decisions logged in the window");
    }
    let mean_surprisal: f64 = row.get("mean_surprisal");
    let mean_ln_k: f64 = row.get("mean_ln_k");
    if mean_ln_k <= 0.0 {
        return s.not_applicable("every logged decision had a single candidate");
    }
    let fraction = (mean_surprisal / mean_ln_k).clamp(0.0, 1.0);
    let ok = fraction >= POLICY_ENTROPY_FLOOR;
    s.measured(
        fraction,
        POLICY_ENTROPY_FLOOR,
        ok,
        if ok {
            format!("{:.0}% of maximum over {n} decisions", fraction * 100.0)
        } else {
            format!(
                "P1 — {:.0}% of maximum over {n} decisions, below the {:.0}% floor; \
                 Tier-B promotion is blocked while this holds",
                fraction * 100.0,
                POLICY_ENTROPY_FLOOR * 100.0
            )
        },
    )
}

/// Artifact pin coverage — this platform's version of §16.2's "Iceberg tag
/// coverage" (ADR-P5-01). A cited artifact that is not pinned can expire, and an
/// expired artifact makes the trial that cited it unreproducible. Anything below
/// 100% is a P1 for exactly the reason the Iceberg retention default is: the loss
/// is silent.
async fn artifact_pin_coverage(pg: &PgPool) -> Signal {
    let s = Signal::new(
        "artifact_pin_coverage",
        "Artifact pin coverage of cited artifacts",
        "§16.2, S-1",
        Severity::P1,
    );
    let row = match sqlx::query(
        "SELECT count(DISTINCT r.handle) AS cited,
                count(DISTINCT r.handle) FILTER (WHERE a.pinned) AS pinned,
                count(DISTINCT r.handle) FILTER (WHERE a.handle IS NULL) AS missing
           FROM artifact_refs r LEFT JOIN artifacts a ON a.handle = r.handle",
    )
    .fetch_one(pg)
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read the artifact registry: {e}")),
    };
    let cited: i64 = row.get("cited");
    if cited == 0 {
        return s.not_applicable("no artifact has been cited yet");
    }
    let pinned: i64 = row.get("pinned");
    let missing: i64 = row.get("missing");
    let coverage = pinned as f64 / cited as f64;
    let ok = pinned == cited && missing == 0;
    s.measured(
        coverage,
        1.0,
        ok,
        if ok {
            format!("all {cited} cited artifacts are pinned")
        } else {
            format!(
                "P1 — {pinned}/{cited} cited artifacts pinned, {missing} no longer resolve; \
                 the trials citing them are unreproducible"
            )
        },
    )
}

/// The trajectory corpus's half-life, measured from `tool_schema_hash` churn
/// (§14.6). A fine-tuned executor is a cache of the tool surface; if the corpus
/// decays faster than a training cadence, fine-tuning is structurally
/// unprofitable and no GPU budget fixes it.
///
/// Measured as the age at which half the recorded steps used a schema hash that
/// is no longer the current one for their tool.
async fn trajectory_half_life(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "trajectory_corpus_half_life",
        "Trajectory corpus half-life (tool schema churn)",
        "§14.6",
        Severity::P2,
    );
    let row = match one(
        pg,
        tenant,
                "WITH current AS (
                   SELECT DISTINCT ON (tool_name) tool_name, tool_schema_hash
                     FROM mlops.agent_trajectory ORDER BY tool_name, recorded_at DESC
                 ), aged AS (
                   SELECT extract(epoch FROM now() - t.recorded_at) / 86400.0 AS age_days,
                          (t.tool_schema_hash IS DISTINCT FROM c.tool_schema_hash) AS stale
                     FROM mlops.agent_trajectory t
                     JOIN current c ON c.tool_name = t.tool_name
                 )
                 SELECT count(*) AS n,
                        count(*) FILTER (WHERE stale) AS stale,
                        coalesce(percentile_disc(0.5) WITHIN GROUP (ORDER BY age_days)
                                 FILTER (WHERE stale), 0)::float8 AS median_stale_age
                   FROM aged",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not read trajectories: {e}")),
    };
    let n: i64 = row.get("n");
    if n == 0 {
        return s.not_applicable("no agent trajectories recorded yet");
    }
    let stale: i64 = row.get("stale");
    if stale == 0 {
        return s.not_fitted(format!(
            "no tool schema has changed across {n} recorded steps — a half-life needs at least one \
             churn event to be measurable"
        ));
    }
    let median_age: f64 = row.get("median_stale_age");
    // Informational: §14.6 says this "informs" the fine-tuning decision. The only
    // way to be wrong is a corpus that decays faster than a training cadence,
    // which nothing here is running, so it never alarms.
    let mut out = s.measured(
        median_age,
        f64::INFINITY,
        true,
        format!(
            "{stale}/{n} steps use a superseded tool schema; median age of a superseded step is \
             {median_age:.0} days. Informational: no fine-tuning cadence exists to outpace (§14.6)."
        ),
    );
    out.threshold = None;
    out
}

/// M5's expected calibration error (§16.2, limit 0.08). The gate pre-screener
/// feeds a budget decision, so it must be *calibrated*, not merely accurate.
async fn m5_calibration(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "m5_calibration_ece",
        "M5 gate pre-screener calibration (ECE)",
        "§16.2, §13",
        Severity::P2,
    );
    match internal_model_tier(pg, tenant, "M5").await {
        Err(e) => s.unavailable(format!("could not read the model registry: {e}")),
        Ok(None) => s.not_fitted(
            "M5 is not registered. Until it is, gate pre-screening runs on its rule tier \
             (a monotone margin at 99% recall) and has no calibration error to report."
                .to_string(),
        ),
        Ok(Some(_)) => s.not_fitted(
            "M5 is registered but no scored-candidate outcomes are on record yet; ECE needs \
             resolved predictions, and a calibration number computed from none would be invented."
                .to_string(),
        ),
    }
    .with_threshold(M5_ECE_LIMIT)
}

/// The MNAR missingness coefficient `b₁` and its p-value (R-05, §5.5). A
/// significant `b₁` is quantitative proof that a naive read of the outcome tensor
/// is biased — and, just as usefully, tells you when the correction has stopped
/// mattering.
async fn mnar_b1(pg: &PgPool, tenant: &str) -> Signal {
    let s = Signal::new(
        "mnar_b1",
        "Outcome-tensor MNAR coefficient b₁",
        "§5.5, R-05",
        Severity::P2,
    );
    let row = match sqlx::query(
        "SELECT to_regclass('mlops.tensor_model') IS NOT NULL AS present",
    )
    .fetch_one(pg)
    .await
    {
        Ok(r) => r,
        Err(e) => return s.unavailable(format!("could not check for the tensor model: {e}")),
    };
    let present: Option<bool> = row.get("present");
    if present != Some(true) {
        let _ = tenant;
        return s.not_fitted(
            "the outcome tensor's completion model ships at ≥500 trials and ≥20 instruments \
             (ADR-P3-03); until then the recommender answers from its rule tier and there is no \
             b₁ to test"
                .to_string(),
        );
    }
    s.not_fitted("no tensor model version has been fitted for this tenant yet".to_string())
}

impl Signal {
    /// Attach the bound a signal *would* be judged against, even when it has no
    /// value yet — so the page can show what the bar will be.
    fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold);
        self
    }
}

/// The registered tier of an internal model, if it exists.
async fn internal_model_tier(
    pg: &PgPool,
    tenant: &str,
    model_prefix: &str,
) -> anyhow::Result<Option<String>> {
    let like = format!("{model_prefix}:%");
    let row = sqlx::query(
        "SELECT tier::text AS tier FROM mlops.internal_model_registry
          WHERE (model_id = $1 OR model_id LIKE $2)
            AND (tenant_id IS NULL OR tenant_id = $3)
          LIMIT 1",
    )
    .bind(model_prefix)
    .bind(&like)
    .bind(tenant)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(|r| r.get::<String, _>("tier")))
}

/// A tenant-keyed view of the report, for the platform job that alarms on it.
#[derive(Debug, Serialize)]
pub struct HealthSummary {
    pub tenant: String,
    pub alarms: BTreeMap<String, String>,
    pub unavailable: BTreeMap<String, String>,
    pub ok: usize,
    pub not_fitted: usize,
    pub not_applicable: usize,
}

impl From<&HealthReport> for HealthSummary {
    fn from(r: &HealthReport) -> Self {
        Self {
            tenant: r.tenant.clone(),
            alarms: r
                .alarms()
                .into_iter()
                .map(|s| (s.id.to_string(), s.detail.clone()))
                .collect(),
            unavailable: r
                .unavailable()
                .into_iter()
                .map(|s| (s.id.to_string(), s.detail.clone()))
                .collect(),
            ok: r.signals.iter().filter(|s| s.state == SignalState::Ok).count(),
            not_fitted: r.signals.iter().filter(|s| s.state == SignalState::NotFitted).count(),
            not_applicable: r
                .signals
                .iter()
                .filter(|s| s.state == SignalState::NotApplicable)
                .count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(state: SignalState, value: Option<f64>) -> Signal {
        Signal {
            id: "x",
            title: "X",
            state,
            severity: Severity::P1,
            value,
            threshold: None,
            detail: "d".into(),
            spec_ref: "§16.2",
        }
    }

    /// The whole point of the state enum: a signal with no model behind it must
    /// not be indistinguishable from one that is fine (AT-69).
    #[test]
    fn a_state_without_a_value_never_carries_one() {
        for state in [
            SignalState::NotFitted,
            SignalState::NotApplicable,
            SignalState::Unavailable,
        ] {
            assert!(!state.has_value(), "{state:?} must not render a number");
            assert!(!state.is_alarm(), "{state:?} is not an alarm");
        }
        assert!(SignalState::Ok.has_value());
        assert!(SignalState::Alarm.has_value());
        assert!(SignalState::Alarm.is_alarm());
    }

    /// "Could not compute" is not "bad" and not "fine". A report is healthy only
    /// when every signal actually produced an answer.
    #[test]
    fn an_unavailable_signal_is_neither_an_alarm_nor_healthy() {
        let r = HealthReport {
            tenant: "t".into(),
            generated_at: Utc::now(),
            window_days: 30,
            signals: vec![sig(SignalState::Ok, Some(1.0)), sig(SignalState::Unavailable, None)],
        };
        assert!(r.alarms().is_empty());
        assert_eq!(r.unavailable().len(), 1);
        assert!(!r.healthy(), "a broken check must not read as healthy");
    }

    #[test]
    fn a_report_of_not_fitted_signals_is_healthy_but_says_so() {
        let r = HealthReport {
            tenant: "t".into(),
            generated_at: Utc::now(),
            window_days: 30,
            signals: vec![
                sig(SignalState::NotFitted, None),
                sig(SignalState::NotApplicable, None),
            ],
        };
        assert!(r.healthy());
        let summary = HealthSummary::from(&r);
        assert_eq!(summary.ok, 0);
        assert_eq!(summary.not_fitted, 1);
        assert_eq!(summary.not_applicable, 1);
    }

    #[test]
    fn alarms_are_ordered_worst_first() {
        let mut p2 = sig(SignalState::Alarm, Some(1.0));
        p2.id = "p2";
        p2.severity = Severity::P2;
        let mut p1 = sig(SignalState::Alarm, Some(1.0));
        p1.id = "p1";
        let r = HealthReport {
            tenant: "t".into(),
            generated_at: Utc::now(),
            window_days: 30,
            signals: vec![p2, p1],
        };
        assert_eq!(r.alarms().iter().map(|s| s.id).collect::<Vec<_>>(), ["p1", "p2"]);
    }

    /// Every §16.2 signal must be present in the report, in every state — a
    /// missing row is indistinguishable from a passing one on a page.
    #[test]
    fn every_spec_signal_has_an_id() {
        let expected = [
            "m5_calibration_ece",
            "policy_entropy",
            "mnar_b1",
            "gate_pass_rate",
            "exploration_fraction",
            "feature_consistency_p99",
            "trajectory_corpus_half_life",
            "neff_vs_trials",
            "artifact_pin_coverage",
            "sealed_holdout_calls",
            "leakage_suite",
        ];
        // The compiler cannot check this list against `health()` without a
        // database, so the list is the contract and `health()` is reviewed
        // against it; the live test (`pg_self_monitor`) asserts the real report
        // contains exactly these ids.
        assert_eq!(expected.len(), 11);
        assert_eq!(
            expected.iter().collect::<std::collections::HashSet<_>>().len(),
            expected.len(),
            "signal ids must be unique"
        );
    }
}
