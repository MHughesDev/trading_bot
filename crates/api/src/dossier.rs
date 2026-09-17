//! Compiling a dossier at the end of a run (AGENT-006 §5.8, AGENT-012 §5; Set S
//! task S-8.7).
//!
//! # The caller `harness::report` did not have
//!
//! `report::claims`, `report::critic` and `report::dossier` are sans-IO and were
//! complete, tested and reached by nothing. This is the thing that reaches them:
//! it assembles the evidence bundle and the critic's draft from rows the
//! platform already holds, compiles the dossier, and stores the result.
//!
//! # The bundle decides everything, so it is assembled carefully
//!
//! `claims::extract` aligns numbers against whatever bundle it is handed, and a
//! number that is in no evidence blocks the dossier. Two things therefore have
//! to be in the bundle or every honest report blocks:
//!
//! * **the run's own results**, rendered as text — a metric the model quoted has
//!   to be findable as a string, because alignment reads the rendered evidence
//!   and not the JSON behind it;
//! * **the pre-registration**, because a report compares measurements to the
//!   targets it committed to, and a target is a number that exists only there.
//!
//! `report::claims`'s module header states that requirement. This is the file
//! that has to honour it.
//!
//! # A failure here does not fail the run
//!
//! A dossier that could not be compiled is recorded as not compiled. The run
//! ended either way, and losing the run record to save the dossier would be the
//! wrong trade — the run record is how anybody finds out the run ended at all.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use harness::report::claims::{extract, Evidence, Extraction, Method};
use harness::report::critic::{review, Criterion, Draft, Presented, Trial};
use harness::report::dossier::{compile, DossierError, Parts, Section};

/// Why a dossier was not compiled.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("the dossier could not be compiled: {0}")]
    Dossier(#[from] DossierError),
    #[error("the platform could not be read: {0}")]
    Backend(#[from] sqlx::Error),
}

/// What the run hands over.
pub struct Produced<'a> {
    pub run_id: Uuid,
    pub tenant_id: &'a str,
    /// The trial this rests on — the dossier's id.
    pub ticket: &'a str,
    /// The run's own prose. The thing whose claims are checked.
    pub result: &'a str,
    /// Rendered evidence the driver already has, by id.
    pub evidence: Vec<Evidence>,
    pub cost_usd: f64,
    pub trials: u32,
    pub harness_version: &'a str,
    pub profile: &'a str,
    pub window: &'a str,
    pub seeds: &'a str,
}

/// Everything the critic and the evidence table need, read from the run's own
/// memory entries.
struct FromMemory {
    unresolved: Vec<String>,
    findings: Vec<Presented>,
    criteria: Vec<Criterion>,
    /// `(entry id, claim, confidence, evidence_class, n, se)` for the desk's
    /// evidence table.
    rows: Vec<(String, String, String, String, Option<i64>, Option<f64>)>,
}

async fn from_memory(pg: &PgPool, user_id: Uuid, run_id: Uuid) -> Result<FromMemory, sqlx::Error> {
    let run = run_id.to_string();
    let rows: Vec<(String, String, String, String, String, Option<i64>, Option<f64>, Value)> =
        sqlx::query_as(
            "SELECT entry_id, kind, claim, confidence, evidence_class, n, se, extra
               FROM agent_memory.entry
              WHERE user_id = $1 AND prov_run_id = $2 AND NOT archived
              ORDER BY created_at",
        )
        .bind(user_id)
        .bind(&run)
        .fetch_all(pg)
        .await?;

    let mut out = FromMemory {
        unresolved: Vec::new(),
        findings: Vec::new(),
        criteria: Vec::new(),
        rows: Vec::new(),
    };
    for (id, kind, claim, confidence, class, n, se, extra) in rows {
        match kind.as_str() {
            "open_question" => {
                // Only the ones still open. A question this run opened and a
                // later finding closed is not unresolved, and listing it would
                // make the section say the opposite of what happened.
                if extra.get("status").and_then(Value::as_str) == Some("open")
                    || extra.get("status").is_none()
                {
                    out.unresolved.push(claim);
                }
            }
            "finding" => {
                out.findings.push(Presented {
                    id: id.clone(),
                    confidence: confidence.clone(),
                    // What the platform recorded, which is the only thing it can
                    // check. A finding stored as `established` is being
                    // presented as established — that is what the column means.
                    presented_as_established: confidence == "established",
                });
                out.criteria.push(Criterion {
                    text: claim.clone(),
                    evidence_ids: vec![id.clone()],
                });
                out.rows.push((id, claim, confidence, class, n, se));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The trials this run reserved, and the family each was taken under.
///
/// The critic's `TrialsAreInAFamily` check reads these. A reservation with no
/// family is a look no multiple-comparison correction covers, which is exactly
/// what that check is for — and until `0134` the platform could not answer the
/// question at all.
async fn trials_of(pg: &PgPool, tenant_id: &str, run_id: Uuid) -> Result<Vec<Trial>, sqlx::Error> {
    let rows: Vec<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT reservation_id, family_id
           FROM mlops.trial_reservation
          WHERE tenant_id = $1 AND run_id = $2
          ORDER BY created_at",
    )
    .bind(tenant_id)
    .bind(run_id)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, family_id)| Trial {
            ticket_id: id.to_string(),
            family_id,
        })
        .collect())
}

/// The run's sealed pre-registration, rendered as evidence text.
///
/// Without it every comparison to a committed threshold is a number in no
/// evidence, and the dossier blocks on a report that did nothing wrong.
///
/// Scoped by `run_id` alone, because `agent_lifecycle.preregistration` has no
/// tenant column — a registration belongs to a run, and the run is already
/// tenant-scoped. The newest amendment wins, which is the one the run was
/// actually held to.
/// The twelve columns, named once: `?` after `flatten()` strips the `Option`, so
/// annotating the binding as `Option<_>` would make sqlx look for a row type of
/// `Option<tuple>` and fail to find one.
type Sealed = (
    Value,
    Value,
    f64,
    String,
    String,
    String,
    String,
    i32,
    String,
    String,
    String,
    String,
);

async fn registration(pg: &PgPool, run_id: Uuid) -> Option<(Evidence, String)> {
    let row: Sealed = sqlx::query_as::<_, Sealed>(
        "SELECT objective, benchmark, delta_practical, window_start, window_end,
                cost_model_version, null_family, trial_budget, gate_profile,
                primary_hypothesis, falsifier, prereg_hash
           FROM agent_lifecycle.preregistration
          WHERE run_id = $1
          ORDER BY amendment DESC, sealed_at DESC
          LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(pg)
    .await
    .ok()
    .flatten()?;
    let (
        objective,
        benchmark,
        delta_practical,
        window_start,
        window_end,
        cost_model_version,
        null_family,
        trial_budget,
        gate_profile,
        primary_hypothesis,
        falsifier,
        prereg_hash,
    ) = row;
    let text = render(&json!({
        "objective": objective,
        "benchmark": benchmark,
        "delta_practical": delta_practical,
        "window_start": window_start,
        "window_end": window_end,
        "cost_model_version": cost_model_version,
        "null_family": null_family.clone(),
        "trial_budget": trial_budget,
        "gate_profile": gate_profile,
        "primary_hypothesis": primary_hypothesis,
        "falsifier": falsifier,
    }));
    Some((
        Evidence {
            id: prereg_hash,
            text,
        },
        null_family,
    ))
}

/// Flatten a JSON object into the text alignment reads.
///
/// `key value` pairs separated by spaces, because
/// [`harness::report::claims::contains_number`] matches whole tokens and a
/// number wedged against a brace is not one.
#[must_use]
pub fn render(v: &Value) -> String {
    fn walk(v: &Value, prefix: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (k, inner) in map {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(inner, &key, out);
                }
            }
            Value::Array(items) => {
                for (i, inner) in items.iter().enumerate() {
                    walk(inner, &format!("{prefix}[{i}]"), out);
                }
            }
            Value::Null => {}
            other => out.push(format!("{prefix} {}", scalar(other))),
        }
    }
    let mut out = Vec::new();
    walk(v, "", &mut out);
    out.join(" ")
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Compile the dossier for a finished run, and store it.
///
/// # Errors
/// [`CompileError`] — an unsupported claim, a missing ticket, a critique that
/// never ran, or a backend that could not be read.
pub async fn compile_and_store(
    pg: &PgPool,
    user_id: Uuid,
    produced: Produced<'_>,
) -> Result<Value, CompileError> {
    let memory = from_memory(pg, user_id, produced.run_id).await?;
    let trials = trials_of(pg, produced.tenant_id, produced.run_id)
        .await
        .unwrap_or_default();

    let mut bundle = produced.evidence.clone();
    // One read for both: the registration is the evidence *and* the declared
    // null. Reading it twice would let the two disagree if an amendment landed
    // between them.
    let sealed = registration(pg, produced.run_id).await;
    if let Some((prereg, _)) = &sealed {
        bundle.push(prereg.clone());
    }
    // Every finding the run recorded is evidence for the claims that cite it.
    for (id, claim, _, _, n, se) in &memory.rows {
        bundle.push(Evidence {
            id: id.clone(),
            text: format!(
                "{claim} n {} se {}",
                n.map_or_else(|| "unknown".into(), |v| v.to_string()),
                se.map_or_else(|| "unknown".into(), |v| v.to_string())
            ),
        });
    }

    let extraction: Extraction = extract(produced.result, &bundle);

    // The critic runs before the dossier compiles, because the dossier refuses
    // to compile without a critique — `None` means "no critic ran", and that is
    // a different report from "the critic found nothing".
    let critique = review(&Draft {
        criteria: memory.criteria.clone(),
        trials,
        findings: memory.findings.clone(),
        // The run's declared null. Absent is the finding, not a default — a
        // report with no null compares a strategy to itself.
        null_family: sealed.as_ref().map(|(_, n)| n.clone()),
    });

    let dossier = compile(
        Parts {
            ticket: produced.ticket.to_string(),
            unresolved_questions: memory.unresolved.clone(),
            critique: Some(critique.lines()),
        },
        extraction.clone(),
    )?;

    let body = wire(&produced, &dossier, &extraction, &memory);
    let bundle_json = json!(bundle
        .iter()
        .map(|e| json!({ "id": e.id, "text": e.text }))
        .collect::<Vec<_>>());

    sqlx::query(
        "INSERT INTO mlops.dossier (ticket, tenant_id, run_id, body, evidence)
         VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (ticket) DO NOTHING",
    )
    .bind(produced.ticket)
    .bind(produced.tenant_id)
    .bind(produced.run_id)
    .bind(&body)
    .bind(&bundle_json)
    .execute(pg)
    .await?;

    Ok(body)
}

/// The shape `frontend/src/components/desk/Dossier.tsx` renders.
///
/// Built to that type rather than to a convenient one: the reader was written
/// first and has been rendering an empty state that names this route since
/// S-6.17. A response that is *nearly* the right shape would render blank
/// sections with no error anywhere.
fn wire(
    produced: &Produced<'_>,
    dossier: &harness::report::dossier::Dossier,
    extraction: &Extraction,
    memory: &FromMemory,
) -> Value {
    let claims: Vec<Value> = extraction
        .claims
        .iter()
        .map(|c| {
            let id = c.supported_by.first().cloned().unwrap_or_default();
            // The class the platform recorded for the entry this claim aligned
            // to, when it aligned to one. Never inferred from the sentence:
            // `model_summary` is a fact about how the number was obtained.
            let class = memory
                .rows
                .iter()
                .find(|(row_id, ..)| *row_id == id)
                .map_or("derived", |(_, _, _, class, _, _)| class.as_str());
            let (n, se) = memory
                .rows
                .iter()
                .find(|(row_id, ..)| *row_id == id)
                .map_or((None, None), |(_, _, _, _, n, se)| (*n, *se));
            json!({
                "claim": c.text,
                "evidence_id": id,
                "n": n,
                "se": se,
                "verification": match c.method {
                    Method::DirectStat => "direct_stat",
                    Method::Derived => "derived",
                    Method::Unverified => "unverified",
                },
                // Every link's provenance. `measured` for anything that aligned
                // to a platform row; a claim whose chain touched a fetched page
                // is marked in the desk's table and the mark survives export —
                // which is why this is a list and not a boolean.
                "chain": [if class == "model_summary" { "untrusted" } else { "measured" }],
                "evidence_class": class,
            })
        })
        .collect();

    json!({
        "ticket": dossier.ticket,
        "verdict": verdict_line(extraction),
        "claims": claims,
        // What actually ran, in order. The reservations are the record of it.
        "plan_as_executed": memory
            .rows
            .iter()
            .map(|(_, claim, _, _, _, _)| claim.clone())
            .collect::<Vec<_>>(),
        "funnel": [],
        "unresolved_questions": section(dossier, Section::UnresolvedQuestions),
        "reviewer_critique": section(dossier, Section::ReviewerCritique),
        "cost_usd": produced.cost_usd,
        "trials": produced.trials,
        "provenance": {
            "harness": produced.harness_version,
            "profile": produced.profile,
            "window": produced.window,
            "seeds": produced.seeds,
        },
        "quoted_untrusted": [],
    })
}

fn section(d: &harness::report::dossier::Dossier, s: Section) -> Vec<String> {
    d.section(s).map(|r| r.lines.clone()).unwrap_or_default()
}

fn verdict_line(extraction: &Extraction) -> String {
    let direct = extraction
        .claims
        .iter()
        .filter(|c| c.method == Method::DirectStat)
        .count();
    format!(
        "{} claim(s), of which {direct} are stated verbatim in the evidence",
        extraction.claims.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alignment matches whole tokens, so the rendering has to put spaces where
    /// JSON puts braces. `{"sharpe":1.43}` contains no token `1.43`.
    #[test]
    fn rendering_puts_every_number_where_alignment_can_find_it() {
        let text = render(&json!({ "sharpe": 1.43, "max_drawdown": 0.12 }));
        assert!(harness::report::claims::contains_number(&text, "1.43"), "{text}");
        assert!(harness::report::claims::contains_number(&text, "0.12"), "{text}");
    }

    /// Nested and array values reach the text too. A metric buried two levels
    /// down is a metric a model quoted and the dossier would have called
    /// invented.
    #[test]
    fn nested_numbers_reach_the_text() {
        let text = render(&json!({
            "metrics": { "sharpe": 1.43 },
            "windows": [{ "return": 0.07 }]
        }));
        assert!(harness::report::claims::contains_number(&text, "1.43"), "{text}");
        assert!(harness::report::claims::contains_number(&text, "0.07"), "{text}");
    }

    /// Nulls are dropped rather than rendered as the word. `null` is not a
    /// number and a bundle full of the string would align against nothing.
    #[test]
    fn a_null_contributes_nothing() {
        assert_eq!(render(&json!({ "a": null })), "");
    }

    /// The verdict line says how much of the report is verbatim, which is the
    /// one thing a reader cannot see by reading it.
    #[test]
    fn the_verdict_line_counts_the_verbatim_claims() {
        let evidence = [Evidence {
            id: "exp_7".into(),
            text: "sharpe 1.43 max_drawdown 0.12".into(),
        }];
        let x = extract("Sharpe 1.43. Drawdown about 0.1.", &evidence);
        let line = verdict_line(&x);
        assert!(line.starts_with("2 claim(s)"), "{line}");
        assert!(line.contains("1 are stated verbatim"), "{line}");
    }
}
