//! "Since you were away" (AGENT-012 §2.2a, §19; Set S task S-6.24).
//!
//! `AwayDigest.tsx` has been calling this route since it was written and nobody
//! served it.
//!
//! # The question it answers
//!
//! A research agent that runs for hours without you is only useful if coming
//! back to it takes a minute, and a reverse-chronological event list is not
//! that: it makes the human reconstruct a state the platform already knows.
//!
//! So every line is a **verdict or a state change**, never an event name. "Run
//! finished" is an event; "the EMA-cross family did not clear Gate 3" is a
//! verdict. The difference is whether reading the line saves you opening the
//! thing.
//!
//! # `kinds_not_covered` is measured, not asserted
//!
//! The response names the kinds it could not cover, because "no findings" and
//! "findings are unreadable here" look identical in an empty list and only one
//! of them means you can stop looking.
//!
//! That array used to be a **constant**, and two of its three rows were false.
//! It said findings were not persisted "because the memory engine exists and its
//! migrations do not" — while `0122_semantic_memory.sql`, `0124` and `0125` were
//! in the tree and applied, `agent_memory.entry` carried its full CHECK set, and
//! `memory.rs` had been serving all seven buckets over it. And it said the
//! monitor "records current state, not state changes" — while
//! `0075_kill_switch.sql` creates `monitor.events`, one row per state change,
//! carrying `from_state`, `to_state`, `actor` and `reason`. No diffing and no
//! guessing were ever needed.
//!
//! A hardcoded claim about the deployment is a claim that was true when it was
//! typed. This one outlived its truth by three migrations and shipped a false
//! sentence to the operator *as data*, in the one field whose entire job is
//! telling them when to stop looking — the exact failure it was written to
//! prevent, pointing the other way.
//!
//! So every row of the array is now derived from a read that just happened.
//! [`kinds_not_covered`] takes the outcome of each read and nothing else: a kind
//! is declared uncovered only when this request tried to read its source and
//! could not, and the reason quotes what the source said. If the store starts
//! answering, the row disappears without anyone editing a string.
//!
//! # What each kind reads
//!
//! * `concluded` — `agent_runs.summary`, the run's own conclusion.
//! * `finding` / `refuted` — `agent_memory.entry`, the semantic memory store.
//! * `approval_defaulted` — `approval_requests` that timed out, with what defaulted.
//! * `monitor` — `monitor.events`, which is the state-change log itself.
//! * `budget` — `llm_usage`, falling back to the runs' own token counters.
//! * `waiting` — approvals still pending, the only forward-looking line.
//!
//! # The memory read repeats a filter, and a test holds the two together
//!
//! `MemoryStore` carries the `NOT archived AND superseded_by IS NULL` filter,
//! and `memory.rs` says out loud why there should not be a second copy of it:
//! "two routes over one table is two places for the archived-and-superseded
//! filter to be forgotten, and that filter is what stops a retracted entry being
//! read as live."
//!
//! The store cannot serve this route, because `Entry` does not carry
//! `created_at` and a digest is a question about a window. So the filter is
//! written out again below — and
//! `crates/api/tests/a_kind_is_declared_uncovered_only_when_its_source_is_absent.rs`
//! scans every SELECT in this crate that touches `agent_memory.entry` and fails
//! if one of them drops either half. A second copy guarded by a test is a second
//! copy; a second copy guarded by a comment is a future defect.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::{auth::BearerToken, state::AppState};

/// Below this the digest is absent entirely.
///
/// Not a tidiness rule: the panel is dismissable and resets the marker, so
/// showing it after a five-minute gap trains the operator to dismiss it without
/// reading — and then it is not there when the gap was eight hours.
const MIN_GAP: Duration = Duration::hours(1);

/// At most this many lines from any one source.
///
/// Per-source rather than overall, so a noisy monitor cannot crowd out the
/// findings.
const PER_SOURCE: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct DigestQuery {
    /// When the operator last looked. The gap is exactly `[since, now]`.
    pub since: DateTime<Utc>,
}

fn line(id: String, at: DateTime<Utc>, kind: &str, text: String, href: Option<String>) -> Value {
    json!({ "id": id, "at": at, "kind": kind, "text": text, "href": href })
}

/// Attach a `ProvenanceMark`, which `AwayDigest.tsx` renders as a badge on the
/// line.
///
/// A `model_summary` claim and a `direct_stat` one are different evidence and
/// the badge is where the difference reaches the human, so a memory line that
/// arrived without it would put the two behind the same mark.
fn with_mark(mut v: Value, mark: Value) -> Value {
    v["mark"] = mark;
    v
}

// ── coverage: what this request could and could not read ────────────────────

/// Whether a source answered, and what it said when it did not.
///
/// `Err` carries the store's own message rather than a sentence about the
/// deployment, because the message is a fact about this request and the sentence
/// would be a claim with no expiry.
pub type Readable = Result<(), String>;

/// What the spend read found, which is four different things and not one number.
///
/// The distinction this type exists for: `llm_usage` is written only by the LLM
/// proxy, and a driver that does not route through the proxy leaves no row. A
/// `SUM` over no rows is `0`, and a `0` rendered beside "spent while you were
/// away" is the platform telling the operator their research was free. An absent
/// measurement is never a zero.
#[derive(Debug, Clone, PartialEq)]
pub enum Budget {
    /// `llm_usage` priced the window. This is the only branch with a dollar figure.
    Priced { usd: f64, calls: i64 },
    /// Nothing priced it, but runs that finished recorded token counters. Volume
    /// is measured; cost is not.
    TokensOnly {
        tokens_in: i64,
        tokens_out: i64,
        runs: i64,
        /// Finished runs whose counters are still at zero — themselves an
        /// absence rather than a run that used nothing.
        runs_without_counters: i64,
    },
    /// Runs were active and nothing recorded what any of them cost.
    Unmeasured { runs: i64 },
    /// No run was active in the window, so there is nothing to price.
    Idle,
    /// The read itself failed.
    Unreadable(String),
}

fn gap(kind: &str, why: String) -> Value {
    json!({ "kind": kind, "why": why })
}

/// The kinds this request could not cover, and why — derived entirely from the
/// three arguments.
///
/// Public because the guard test calls it: the property worth holding is "every
/// source answered ⇒ the array is empty", and that is a statement about this
/// function rather than about any one response.
#[must_use]
pub fn kinds_not_covered(memory: &Readable, monitor: &Readable, budget: &Budget) -> Vec<Value> {
    let mut out = Vec::new();

    if let Err(e) = memory {
        // Both kinds read the same table, so they fail together and the second
        // row says so rather than repeating the message.
        out.push(gap(
            "finding",
            format!(
                "the semantic memory store did not answer this request: {e}. The digest is \
                 therefore silent about findings, which is not the same as there being none."
            ),
        ));
        out.push(gap(
            "refuted",
            "same source as `finding`, same failure".into(),
        ));
    }

    if let Err(e) = monitor {
        out.push(gap(
            "monitor",
            format!(
                "the monitor's state-change log did not answer this request: {e}. Nothing here \
                 should be read as the platform having stayed nominal."
            ),
        ));
    }

    match budget {
        Budget::Unreadable(e) => out.push(gap(
            "budget",
            format!("the spend read did not answer this request: {e}"),
        )),
        Budget::Unmeasured { runs } => out.push(gap(
            "budget",
            format!(
                "{runs} run(s) were active while you were away and nothing recorded what they \
                 cost — no priced model call, and no token counter on the runs themselves. That \
                 is an absent measurement and not a spend of zero, so no figure is shown."
            ),
        )),
        Budget::Priced { .. } | Budget::TokensOnly { .. } | Budget::Idle => {}
    }

    out
}

/// GET /api/desk/digest?since=…
pub async fn digest(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<DigestQuery>,
) -> impl IntoResponse {
    let until = Utc::now();
    let user = token.user_id();

    if until - q.since < MIN_GAP {
        // Absent, not empty. The two render differently and mean different
        // things: "nothing happened" against "you have not been away".
        return Json(json!({
            "since": q.since,
            "until": until,
            "lines": [],
            "absent": true,
            "reason": "the gap is under an hour",
        }))
        .into_response();
    }

    let mut lines: Vec<Value> = Vec::new();

    // ── runs that concluded, and what they concluded ────────────────────────
    let runs = sqlx::query(
        "SELECT run_id, status, goal, summary, error, finished_at \
           FROM agent_runs \
          WHERE user_id = $1 AND finished_at > $2 AND finished_at <= $3 \
          ORDER BY finished_at DESC LIMIT 50",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .fetch_all(&state.pg)
    .await;

    match runs {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("run_id");
                let status: String = r.get("status");
                let goal: String = r.get("goal");
                let summary: Option<String> = r.try_get("summary").ok().flatten();
                let error: Option<String> = r.try_get("error").ok().flatten();
                let at: Option<DateTime<Utc>> = r.try_get("finished_at").ok().flatten();

                // The conclusion if there is one, and the failure if there is
                // not. The goal is the fallback and it is the weakest line here,
                // because a goal is what was asked rather than what was found.
                let text = match (&summary, &error) {
                    (Some(s), _) if !s.trim().is_empty() => s.trim().to_string(),
                    (_, Some(e)) if !e.trim().is_empty() => {
                        format!("{status}: {}", e.trim())
                    }
                    _ => format!("{status}, with no conclusion recorded — “{goal}”"),
                };
                lines.push(line(
                    format!("run:{id}"),
                    at.unwrap_or(until),
                    "concluded",
                    text,
                    Some(format!("/desk/runs/{id}")),
                ));
            }
        }
        Err(e) => return db_error("agent_runs", &e),
    }

    // ── what was learned, and what was refuted ──────────────────────────────
    //
    // `NOT archived AND superseded_by IS NULL` mirrors `MemoryStore`, and the
    // guard test holds the two together — see the module header. Dropping either
    // half would let a retracted entry read as a fresh finding, which is the one
    // thing this section must never do.
    //
    // A `refuted` confidence can sit on any kind, so the predicate is not just a
    // kind list: an instrument profile that was refuted is a refutation.
    let memory_rows = sqlx::query(
        "SELECT entry_id, kind, subject, claim, confidence, evidence_class, \
                quantity, direction, value, se, n, extra, stale, created_at \
           FROM agent_memory.entry \
          WHERE user_id = $1 AND NOT archived AND superseded_by IS NULL \
            AND created_at > $2 AND created_at <= $3 \
            AND (kind IN ('finding', 'negative_result') OR confidence = 'refuted') \
          ORDER BY created_at DESC LIMIT $4",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .bind(PER_SOURCE)
    .fetch_all(&state.pg)
    .await;

    // Not a 500. A digest whose memory read failed is still worth the four
    // sections that answered — as long as it says which one did not, which is
    // what the `Err` is carried here to do.
    let memory: Readable = match memory_rows {
        Ok(rows) => {
            for r in rows {
                lines.push(memory_line(&r));
            }
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    };

    // ── what the monitor did about it ───────────────────────────────────────
    //
    // `monitor.events` is one row per state change (`0075_kill_switch.sql`), so
    // this reads transitions rather than inferring them. It carries no
    // `user_id`: the monitor guards the platform, not an account, and every
    // operator sees the same halt. Said here rather than papered over with a
    // join that would silently drop every row.
    let monitor_rows = sqlx::query(
        "SELECT event_id, occurred_at, scope, family, kind, metric, \
                observed_value::text AS observed_value, \
                limit_value::text AS limit_value, \
                from_state, to_state, reason, \
                is_drill, suppressed_by_window \
           FROM monitor.events \
          WHERE occurred_at > $1 AND occurred_at <= $2 \
          ORDER BY occurred_at DESC LIMIT $3",
    )
    .bind(q.since)
    .bind(until)
    .bind(PER_SOURCE)
    .fetch_all(&state.pg)
    .await;

    let monitor: Readable = match monitor_rows {
        Ok(rows) => {
            for r in rows {
                lines.push(monitor_line(&r));
            }
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    };

    // ── approvals that timed out, and what defaulted ────────────────────────
    let defaulted = sqlx::query(
        "SELECT a.approval_id, a.kind, a.default_option, a.answered_at, a.created_at \
           FROM approval_requests a \
           JOIN research_projects p ON p.project_id = a.project_id \
          WHERE p.user_id = $1 AND a.state = 'defaulted' \
            AND COALESCE(a.answered_at, a.created_at) > $2 \
            AND COALESCE(a.answered_at, a.created_at) <= $3 \
          ORDER BY COALESCE(a.answered_at, a.created_at) DESC LIMIT 50",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .fetch_all(&state.pg)
    .await;

    match defaulted {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("approval_id");
                let kind: String = r.get("kind");
                let default_option: Option<String> = r.try_get("default_option").ok().flatten();
                let at: Option<DateTime<Utc>> = r.try_get("answered_at").ok().flatten();
                let created: DateTime<Utc> = r.get("created_at");
                // Naming what defaulted is the whole point: an approval that
                // timed out still decided something.
                let text = match default_option {
                    Some(opt) => format!("`{kind}` timed out and defaulted to **{opt}**"),
                    None => format!(
                        "`{kind}` timed out with no default recorded, so nothing was decided"
                    ),
                };
                lines.push(line(
                    format!("approval:{id}"),
                    at.unwrap_or(created),
                    "approval_defaulted",
                    text,
                    Some(format!("/desk/approvals/{id}")),
                ));
            }
        }
        Err(e) => return db_error("approval_requests", &e),
    }

    // ── what is waiting on you now ──────────────────────────────────────────
    let pending = sqlx::query(
        "SELECT a.approval_id, a.kind, a.created_at, a.timeout_at \
           FROM approval_requests a \
           JOIN research_projects p ON p.project_id = a.project_id \
          WHERE p.user_id = $1 AND a.state = 'pending' \
          ORDER BY a.created_at ASC LIMIT 50",
    )
    .bind(user)
    .fetch_all(&state.pg)
    .await;

    match pending {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("approval_id");
                let kind: String = r.get("kind");
                let created: DateTime<Utc> = r.get("created_at");
                let timeout: Option<DateTime<Utc>> = r.try_get("timeout_at").ok().flatten();
                let text = match timeout {
                    Some(t) if t > until => format!(
                        "`{kind}` is waiting on you, and defaults in {} minutes",
                        (t - until).num_minutes().max(0)
                    ),
                    Some(_) => format!("`{kind}` is waiting on you and is past its timeout"),
                    None => format!("`{kind}` is waiting on you, with no timeout"),
                };
                lines.push(line(
                    format!("waiting:{id}"),
                    created,
                    "waiting",
                    text,
                    Some(format!("/desk/approvals/{id}")),
                ));
            }
        }
        Err(e) => return db_error("approval_requests", &e),
    }

    // ── what it cost ────────────────────────────────────────────────────────
    let budget = read_budget(&state, user, q.since, until).await;
    if let Some(v) = budget_line(&budget, until) {
        lines.push(v);
    }

    // Newest first, which is how the panel reads.
    lines.sort_by(|a, b| b["at"].as_str().cmp(&a["at"].as_str()));

    Json(json!({
        "since": q.since,
        "until": until,
        "lines": lines,
        // Said out loud, and measured rather than asserted. "No findings" and
        // "findings are unreadable here" look identical in an empty list, and
        // only one of them means you can stop looking.
        "kinds_not_covered": kinds_not_covered(&memory, &monitor, &budget),
    }))
    .into_response()
}

// ── the memory line ─────────────────────────────────────────────────────────

/// A measurement, rendered as one, or nothing at all.
///
/// `quantity IS NULL` is the schema's way of saying this entry measured nothing,
/// and `0.0` is a value it measured. They are never printed the same.
fn measurement_clause(r: &sqlx::postgres::PgRow) -> String {
    let Some(quantity): Option<String> = r.try_get("quantity").ok().flatten() else {
        return String::new();
    };
    let value: Option<f64> = r.try_get("value").ok().flatten();
    let se: Option<f64> = r.try_get("se").ok().flatten();
    let n: Option<i64> = r.try_get("n").ok().flatten();
    let mut c = match value {
        Some(v) => format!(" ({quantity} {v:+.4}"),
        None => format!(" ({quantity}"),
    };
    if let Some(s) = se {
        c.push_str(&format!(" ± {s:.4}"));
    }
    match n {
        Some(n) => c.push_str(&format!(", n={n})")),
        None => c.push_str(", n not recorded)"),
    }
    c
}

fn memory_line(r: &sqlx::postgres::PgRow) -> Value {
    let id: String = r.get("entry_id");
    let kind: String = r.get("kind");
    let subject: String = r.get("subject");
    let claim: String = r.get("claim");
    let confidence: String = r.get("confidence");
    let evidence_class: String = r.get("evidence_class");
    let extra: Value = r.try_get("extra").unwrap_or(Value::Null);
    let stale: bool = r.try_get("stale").unwrap_or(false);
    let at: DateTime<Utc> = r.get("created_at");

    // A negative result is a refutation with a class attached, and a `refuted`
    // confidence is a refutation whatever kind carries it.
    let refuted = confidence == "refuted" || kind == "negative_result";

    let mut text = if refuted {
        format!("**{subject}** — refuted: {claim}")
    } else {
        format!("**{subject}** — {claim}")
    };
    if refuted {
        if let Some(c) = extra.get("failure_class").and_then(Value::as_str) {
            text.push_str(&format!(" (`{c}`)"));
        }
    } else if confidence == "suggestive" {
        // Said on the line rather than left to the badge: "suggestive" is the
        // difference between a result and a lead, and the operator is deciding
        // what to act on.
        text.push_str(" — suggestive");
    }
    text.push_str(&measurement_clause(r));
    if stale {
        text.push_str(" (now stale)");
    }

    with_mark(
        line(
            format!("memory:{id}"),
            at,
            if refuted { "refuted" } else { "finding" },
            text,
            Some("/desk/memory".into()),
        ),
        json!({
            // `human` is the one class whose source is the operator; everything
            // else this store holds was computed here.
            "provenance": if evidence_class == "human" { "user" } else { "platform" },
            "evidence_class": evidence_class,
        }),
    )
}

// ── the monitor line ────────────────────────────────────────────────────────

/// A `NUMERIC(38,18)` for display, without the zeros the column pads to.
///
/// Cast to `text` in the query rather than decoded as a number, for two
/// reasons that point the same way. `float8` would round, and a limit compared
/// against an observation that has been rounded for display is how a breach
/// comes to read as a miss by a hair. `rust_decimal` would *fail* above its
/// 96-bit mantissa, which `NUMERIC(38,18)` admits — and a decode error caught
/// with `.ok()` turns a recorded observation into a silent absence, which is
/// the one substitution this route exists to refuse.
///
/// Trailing zeros go; the digits do not.
fn number(r: &sqlx::postgres::PgRow, col: &str) -> Option<String> {
    let raw: Option<String> = r.try_get(col).ok().flatten();
    raw.map(|s| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    })
    .filter(|s| !s.is_empty())
}

fn monitor_line(r: &sqlx::postgres::PgRow) -> Value {
    let id: uuid::Uuid = r.get("event_id");
    let at: DateTime<Utc> = r.get("occurred_at");
    let scope: String = r.get("scope");
    let family: String = r.get("family");
    let kind: String = r.get("kind");
    let metric: Option<String> = r.try_get("metric").ok().flatten();
    let from_state: Option<String> = r.try_get("from_state").ok().flatten();
    let to_state: Option<String> = r.try_get("to_state").ok().flatten();
    let reason: Option<String> = r.try_get("reason").ok().flatten();
    let is_drill: bool = r.try_get("is_drill").unwrap_or(false);
    let suppressed: Option<uuid::Uuid> = r.try_get("suppressed_by_window").ok().flatten();

    let mut text = String::new();
    // First, and in capitals, because a rehearsal that reads as a halt is worse
    // than no line at all — the operator's next act is to go and look at a
    // position that was never at risk.
    if is_drill {
        text.push_str("DRILL — ");
    }
    text.push_str(&format!("`{scope}` ({family}) **{kind}**"));
    match (&from_state, &to_state) {
        (Some(f), Some(t)) => text.push_str(&format!(": {f} → {t}")),
        (None, Some(t)) => text.push_str(&format!(": → {t}")),
        _ => {}
    }
    if let Some(m) = &metric {
        text.push_str(&format!(" — {m}"));
        if let Some(o) = number(r, "observed_value") {
            text.push_str(&format!(" at {o}"));
        }
        if let Some(l) = number(r, "limit_value") {
            text.push_str(&format!(" against a limit of {l}"));
        }
    }
    if let Some(why) = &reason {
        if !why.trim().is_empty() {
            text.push_str(&format!(" — {}", why.trim()));
        }
    }
    if suppressed.is_some() {
        text.push_str(
            " (an open maintenance window suppressed the action; the event was recorded anyway)",
        );
    }

    line(
        format!("monitor:{id}"),
        at,
        "monitor",
        text,
        Some("/desk/monitor".into()),
    )
}

// ── the budget line ─────────────────────────────────────────────────────────

/// Price the window, and say which of the four things happened.
///
/// Two reads rather than one, because they answer different questions:
/// `llm_usage` is the priced record, and `agent_runs` is whether there was
/// anything to price. Without the second, "no rows" and "no work" are the same
/// answer — and they are the difference between a quiet night and a hole in the
/// accounting.
async fn read_budget(
    state: &AppState,
    user: uuid::Uuid,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Budget {
    let spend = sqlx::query(
        "SELECT COALESCE(SUM(cost_usd), 0)::float8 AS usd, COUNT(*)::bigint AS calls \
           FROM llm_usage WHERE user_id = $1 AND created_at > $2 AND created_at <= $3",
    )
    .bind(user)
    .bind(since)
    .bind(until)
    .fetch_one(&state.pg)
    .await;

    let (usd, calls) = match spend {
        Ok(r) => (
            r.try_get::<f64, _>("usd").unwrap_or(0.0),
            r.try_get::<i64, _>("calls").unwrap_or(0),
        ),
        Err(e) => return Budget::Unreadable(e.to_string()),
    };
    if calls > 0 {
        return Budget::Priced { usd, calls };
    }

    // Tokens are attributed to runs that *finished* in the gap, so the sum is
    // exactly "what the runs that concluded while you were away used" — a whole
    // number about a whole run, rather than a slice of one apportioned by guess.
    let counters = sqlx::query(
        "SELECT \
           COUNT(*) FILTER (WHERE finished_at > $2 AND finished_at <= $3)::bigint AS finished, \
           COALESCE(SUM(tokens_in)  FILTER (WHERE finished_at > $2 AND finished_at <= $3), 0)::bigint AS tin, \
           COALESCE(SUM(tokens_out) FILTER (WHERE finished_at > $2 AND finished_at <= $3), 0)::bigint AS tout, \
           COUNT(*) FILTER (WHERE finished_at > $2 AND finished_at <= $3 \
                              AND tokens_in = 0 AND tokens_out = 0)::bigint AS blank, \
           COUNT(*) FILTER (WHERE finished_at IS NULL AND created_at <= $3)::bigint AS active \
         FROM agent_runs WHERE user_id = $1",
    )
    .bind(user)
    .bind(since)
    .bind(until)
    .fetch_one(&state.pg)
    .await;

    let r = match counters {
        Ok(r) => r,
        Err(e) => return Budget::Unreadable(e.to_string()),
    };
    let finished: i64 = r.try_get("finished").unwrap_or(0);
    let active: i64 = r.try_get("active").unwrap_or(0);
    let tokens_in: i64 = r.try_get("tin").unwrap_or(0);
    let tokens_out: i64 = r.try_get("tout").unwrap_or(0);
    let blank: i64 = r.try_get("blank").unwrap_or(0);

    if tokens_in + tokens_out > 0 {
        Budget::TokensOnly {
            tokens_in,
            tokens_out,
            runs: finished,
            runs_without_counters: blank,
        }
    } else if finished + active > 0 {
        Budget::Unmeasured {
            runs: finished + active,
        }
    } else {
        Budget::Idle
    }
}

/// The line, when there is a measurement to put on one.
///
/// `Unmeasured`, `Unreadable` and `Idle` return `None` — the first two because
/// they belong in `kinds_not_covered` where the operator reads them as a gap,
/// and `Idle` because no run was active and silence is then the true report.
#[must_use]
pub fn budget_line(budget: &Budget, at: DateTime<Utc>) -> Option<Value> {
    let text = match budget {
        Budget::Priced { usd, calls } => {
            format!("${usd:.2} spent over {calls} model calls while you were away")
        }
        Budget::TokensOnly {
            tokens_in,
            tokens_out,
            runs,
            runs_without_counters,
        } => {
            let mut t = format!(
                "{tokens_in} tokens in and {tokens_out} out over {runs} run(s) that finished \
                 while you were away. No priced model call was recorded for this window, so what \
                 it cost in dollars is absent rather than zero"
            );
            if *runs_without_counters > 0 {
                t.push_str(&format!(
                    " — and {runs_without_counters} of those runs recorded no token count at \
                     all, which is also an absence and not a run that used nothing"
                ));
            }
            t
        }
        Budget::Unmeasured { .. } | Budget::Unreadable(_) | Budget::Idle => return None,
    };
    Some(line(
        "budget".into(),
        at,
        "budget",
        text,
        Some("/desk/usage".into()),
    ))
}

fn db_error(table: &str, e: &sqlx::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "digest_unavailable",
            "table": table,
            "message": e.to_string(),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gap rule is the acceptance's third clause, and it is arithmetic, so
    /// it is tested without a database.
    #[test]
    fn a_short_gap_is_below_the_threshold_and_a_long_one_is_not() {
        let now = Utc::now();
        assert!(now - (now - Duration::minutes(59)) < MIN_GAP);
        assert!(now - (now - Duration::minutes(61)) >= MIN_GAP);
    }

    /// When every source answers, nothing is declared uncovered.
    ///
    /// This is the property the old constant array could not have: it named
    /// three kinds as unreadable on every request, including the ones where all
    /// three sources answered perfectly.
    #[test]
    fn a_request_whose_sources_all_answered_declares_no_gaps() {
        let none = kinds_not_covered(&Ok(()), &Ok(()), &Budget::Idle);
        assert!(
            none.is_empty(),
            "every source answered and the response still claims a blind spot: {none:?}"
        );
        let priced = kinds_not_covered(
            &Ok(()),
            &Ok(()),
            &Budget::Priced {
                usd: 1.5,
                calls: 12,
            },
        );
        assert!(priced.is_empty(), "{priced:?}");
    }

    /// A gap's reason quotes what the source said.
    ///
    /// The failure this prevents is the one that shipped: a reason that is a
    /// standing claim about the deployment stays in the body long after it stops
    /// being true, and nothing about the request can contradict it.
    #[test]
    fn a_gaps_reason_quotes_the_failure_rather_than_asserting_one() {
        let gaps = kinds_not_covered(
            &Err("relation \"agent_memory.entry\" does not exist".into()),
            &Err("permission denied for schema monitor".into()),
            &Budget::Unreadable("connection closed".into()),
        );
        let kinds: Vec<&str> = gaps.iter().filter_map(|g| g["kind"].as_str()).collect();
        assert_eq!(kinds, ["finding", "refuted", "monitor", "budget"]);
        assert!(gaps[0]["why"].as_str().unwrap().contains("does not exist"));
        assert!(gaps[2]["why"]
            .as_str()
            .unwrap()
            .contains("permission denied"));
        assert!(gaps[3]["why"]
            .as_str()
            .unwrap()
            .contains("connection closed"));
    }

    /// An unmeasured spend is a gap, never a zero.
    #[test]
    fn an_unpriced_window_with_runs_in_it_is_a_gap_and_not_a_dollar_figure() {
        let b = Budget::Unmeasured { runs: 7 };
        assert!(
            budget_line(&b, Utc::now()).is_none(),
            "an unmeasured spend must not become a line, because every line with a `budget` kind \
             carries a figure and the only figure available would be 0"
        );
        let gaps = kinds_not_covered(&Ok(()), &Ok(()), &b);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0]["kind"], "budget");
        let why = gaps[0]["why"].as_str().unwrap();
        assert!(why.contains('7'), "the reason does not say how many runs");
        assert!(
            why.contains("not a spend of zero"),
            "the reason must rule the zero out in words: {why}"
        );
    }

    /// Token volume is a measurement even when dollars are not, and the line
    /// says which of the two it is showing.
    #[test]
    fn a_token_only_window_reports_volume_and_names_the_missing_dollars() {
        let v = budget_line(
            &Budget::TokensOnly {
                tokens_in: 2_526_695,
                tokens_out: 27_512,
                runs: 27,
                runs_without_counters: 3,
            },
            Utc::now(),
        )
        .expect("a measured token count is a line");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("2526695"));
        assert!(
            text.contains("absent rather than zero"),
            "a volume figure beside a silent dollar column reads as free: {text}"
        );
        assert!(
            !text.contains("$"),
            "no dollar figure exists for this window: {text}"
        );
    }

    /// Every kind the frontend's `DigestLine` union admits has a source this
    /// route reads. A kind that is neither emitted nor capable of being declared
    /// uncovered is a silent hole.
    #[test]
    fn every_line_kind_is_either_served_or_can_declare_itself_uncovered() {
        // `DigestLine['kind']` in frontend/src/api/desk.ts.
        const KINDS: [&str; 7] = [
            "concluded",
            "finding",
            "refuted",
            "approval_defaulted",
            "monitor",
            "budget",
            "waiting",
        ];
        // Kinds that can appear in `kinds_not_covered`, which is exactly the
        // kinds whose source is read with a degrade path rather than a 500.
        let declarable: Vec<String> = kinds_not_covered(
            &Err("x".into()),
            &Err("x".into()),
            &Budget::Unreadable("x".into()),
        )
        .iter()
        .map(|g| g["kind"].as_str().unwrap().to_string())
        .collect();
        // The rest 500 the whole digest when their read fails, so they are never
        // silently empty either.
        const HARD: [&str; 3] = ["concluded", "approval_defaulted", "waiting"];
        for k in KINDS {
            assert!(
                declarable.iter().any(|d| d == k) != HARD.contains(&k),
                "`{k}` is in neither list, or in both — a kind that can be silently empty reads \
                 as \"this did not happen\" when it means \"I cannot see it\""
            );
        }
    }
}
