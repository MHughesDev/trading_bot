//! The caller `harness::lifecycle::evaluate` never had (AGENT-013 §6.2, §6.9;
//! Set S task S-10.4).
//!
//! # What was missing
//!
//! `evaluate` is a pure function of the tickets and was built, tested and
//! complete. Nothing called it, so every hypothesis a run sealed stayed `open`
//! forever — the desk rendered `open`, the scorecard counted `open`, and the
//! field the whole pre-registration exists to settle was never settled.
//!
//! Two things had to be true before a caller could exist, and only one of them
//! was. The verdict logic was there; the **evidence** was not, because the
//! falsifier vocabulary and `mlops.trial_statistic`'s CHECK were disjoint lists
//! (migration 0130). A caller written before that would have found no
//! measurement for any falsifier ever validated and written
//! `inconclusive: insufficient_data` on every hypothesis on the platform —
//! which looks exactly like a finding about the research and is a fact about the
//! schema.
//!
//! # Why this is not a tool
//!
//! §6.2: a run may state a hypothesis and its falsifier, and may not decide
//! whether the hypothesis was supported. A verdict the model writes is a verdict
//! the model can rewrite, and a run that has been working on something for
//! thirty steps genuinely believes it has shown it. So this runs on the
//! platform's side, from the ledger's own rows, and the request type for
//! `hypothesise` has no status field to ignore.
//!
//! # What `supported` needs, and what this can supply
//!
//! §6.9's battery — an out-of-sample ticket, a cost-model ticket, a benchmark
//! ticket and a reproducibility receipt. **Only the first is derivable today.**
//! `mlops.trial` does not label a trial's role, so "which ticket was the
//! benchmark" has no answer in the schema, and the funnel records no receipt.
//!
//! Absent evidence is left absent, exactly as `gate_worker` leaves it: a
//! would-be `supported` becomes `inconclusive: evidence_incomplete` carrying the
//! list of what was missing. That means **`supported` is currently unreachable
//! from real data**, and saying so is the point — the alternative is filling the
//! battery with tickets that happen to exist, which would make the expensive
//! verdict the cheap one.
//!
//! Refutations are unaffected, and that asymmetry is deliberate: requiring the
//! battery before admitting a failure would make failing expensive and passing
//! cheap, which is backwards for a platform whose main output is meant to be
//! negative results.

use std::collections::BTreeMap;

use harness::lifecycle::evaluate::{self, Evidence, Falsifier, Measurement, Role, Verdict, Why};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// One hypothesis, decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Decided {
    pub hypothesis_id: Uuid,
    pub verdict: Verdict,
    /// What `supported` was missing, when that is why it is not `supported`.
    pub missing: Vec<&'static str>,
    /// The metric the falsifier named, so a reader can see what was looked for.
    pub metric: String,
}

impl Decided {
    /// The status and reason as the columns hold them.
    #[must_use]
    pub fn columns(&self) -> (&'static str, Option<&'static str>) {
        match &self.verdict {
            Verdict::Supported => ("supported", None),
            Verdict::Refuted => ("refuted", None),
            Verdict::Inconclusive { why } => ("inconclusive", Some(why.as_str())),
        }
    }
}

/// Why a hypothesis could not be decided at all.
///
/// Distinct from `inconclusive`, and the distinction matters: `inconclusive` is
/// a verdict the platform reached, and this is the platform failing to reach
/// one. A run whose falsifier does not parse has a registration that was
/// accepted and cannot be evaluated, which is a defect rather than a result.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Undecidable {
    #[error("the falsifier does not parse: {0}")]
    Unparseable(String),
    #[error("reading the ledger: {0}")]
    Db(String),
}

/// Assemble what the ledger holds about one hypothesis.
///
/// Every field this cannot prove is left `None`. The rule is `gate_worker`'s and
/// the reason is the same: a default here would make the platform's measured
/// confidence go *up* as its evidence pipeline degrades.
///
/// # Errors
/// The database failed. An empty result is not an error — it is
/// `insufficient_data`, which is a verdict.
pub async fn evidence_for(
    pg: &PgPool,
    tenant: &str,
    hypothesis_id: Uuid,
    metric: &str,
    protocol_unstable: bool,
    budget_exhausted: bool,
) -> Result<Evidence, Undecidable> {
    let rows: Vec<(Uuid, f64)> = sqlx::query_as(
        "SELECT s.trial_id, s.value
           FROM mlops.trial_statistic s
           JOIN mlops.trial t ON t.trial_id = s.trial_id
          WHERE t.hypothesis_id = $1
            AND t.tenant_id = $2
            AND s.name = $3
          ORDER BY s.recorded_at",
    )
    .bind(hypothesis_id)
    .bind(tenant)
    .bind(metric)
    .fetch_all(pg)
    .await
    .map_err(|e| Undecidable::Db(e.to_string()))?;

    let mut measurements: BTreeMap<String, Vec<Measurement>> = BTreeMap::new();
    if !rows.is_empty() {
        measurements.insert(
            metric.to_string(),
            rows.iter()
                .map(|(trial_id, value)| Measurement {
                    value: *value,
                    ticket_id: trial_id.to_string(),
                    // `mlops.trial_statistic` stores a value and no standard
                    // error. A missing `se` is "cannot tell" and never zero —
                    // a silent zero would call every pair of unequal numbers a
                    // conflict and turn every repeated measurement into an
                    // `inconclusive`.
                    se: None,
                })
                .collect(),
        );
    }

    Ok(Evidence {
        // The observed run in the funnel is out-of-sample by construction: the
        // gates it feeds are CPCV and walk-forward. It is the one piece of
        // §6.9's battery the schema can actually answer for.
        out_of_sample_ticket: rows.first().map(|(t, _)| t.to_string()),
        // Not derivable. `mlops.trial` carries no role for a trial, so "which
        // ticket was the benchmark" and "which pinned the cost model" have no
        // answer here — and a ticket that merely exists is not evidence that the
        // comparison was made.
        cost_model_ticket: None,
        benchmark_ticket: None,
        // The funnel records no reproducibility receipt. `backtest::receipt`
        // builds one and nothing calls it (S-10.8).
        repro_spec: None,
        n: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        measurements,
        protocol_unstable,
        budget_exhausted,
    })
}

/// Decide every open hypothesis a run sealed, and write the verdicts.
///
/// Returns what it decided, so a caller can report it without re-reading.
///
/// A hypothesis whose falsifier does not parse is **left open** and reported as
/// [`Undecidable`]: the registration was accepted with it, so writing a verdict
/// now would be the platform grading a commitment it cannot read.
///
/// # Errors
/// The database failed while listing. A per-hypothesis failure is returned
/// beside the decisions rather than aborting the rest — a run with four
/// hypotheses should not lose three verdicts to one bad falsifier.
pub async fn decide_run(
    pg: &PgPool,
    tenant: &str,
    run_id: Uuid,
) -> Result<(Vec<Decided>, Vec<(Uuid, Undecidable)>), Undecidable> {
    let open: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT hypothesis_id, falsifier, role
           FROM agent_lifecycle.hypothesis
          WHERE run_id = $1 AND status = 'open'
          ORDER BY created_at",
    )
    .bind(run_id)
    .fetch_all(pg)
    .await
    .map_err(|e| Undecidable::Db(e.to_string()))?;

    // Two run-level facts §6.2 reads: a protocol rewritten to the cap, and a
    // budget spent. Both are about the run rather than the hypothesis, so they
    // are read once.
    let amendments: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(amendment), 0) FROM agent_lifecycle.preregistration WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pg)
    .await
    .unwrap_or(0);
    let protocol_unstable = amendments >= i32::try_from(ledger::prereg::MAX_PREREG_AMENDMENTS)
        .unwrap_or(i32::MAX)
        - 1;

    let mut decided = Vec::new();
    let mut undecidable = Vec::new();

    for (hypothesis_id, falsifier_text, role) in open {
        let falsifier: Falsifier =
            match evaluate::parse(&falsifier_text, crate::routes::lifecycle::METRIC_VOCABULARY) {
                Ok(f) => f,
                Err(e) => {
                    undecidable.push((hypothesis_id, Undecidable::Unparseable(e.to_string())));
                    continue;
                }
            };
        let evidence = match evidence_for(
            pg,
            tenant,
            hypothesis_id,
            &falsifier.metric,
            protocol_unstable,
            // Not read from the ledger yet: the trial budget is on the
            // registration and the count is on the campaign, and a run that
            // spent neither has no row saying so. `false` is the conservative
            // value here — it makes an unmeasured hypothesis read
            // `insufficient_data` rather than `budget_exhausted`, and the second
            // is a stronger claim about why.
            false,
        )
        .await
        {
            Ok(e) => e,
            Err(e) => {
                undecidable.push((hypothesis_id, e));
                continue;
            }
        };

        let verdict = evaluate::evaluate(&falsifier, Role::from_column(&role), &evidence);
        let missing = if matches!(
            verdict,
            Verdict::Inconclusive {
                why: Why::EvidenceIncomplete
            }
        ) {
            evaluate::missing_evidence(&evidence)
        } else {
            Vec::new()
        };
        let d = Decided {
            hypothesis_id,
            verdict,
            missing,
            metric: falsifier.metric.clone(),
        };
        if let Err(e) = write(pg, &d).await {
            undecidable.push((hypothesis_id, e));
            continue;
        }
        decided.push(d);
    }

    Ok((decided, undecidable))
}

/// Write one verdict.
///
/// `status = 'open'` in the WHERE clause is the second half of §6.2's rule: the
/// first verdict stands. A re-evaluation that overwrote one would make the
/// answer depend on when somebody last looked, and a hypothesis whose verdict
/// changed without new evidence is not a verdict.
async fn write(pg: &PgPool, d: &Decided) -> Result<(), Undecidable> {
    let (status, why) = d.columns();
    sqlx::query(
        "UPDATE agent_lifecycle.hypothesis
            SET status = $2, inconclusive_why = $3, decided_at = now()
          WHERE hypothesis_id = $1 AND status = 'open'",
    )
    .bind(d.hypothesis_id)
    .bind(status)
    .bind(why)
    .execute(pg)
    .await
    .map_err(|e| Undecidable::Db(e.to_string()))?;
    Ok(())
}

/// The decisions, as the run record holds them.
#[must_use]
pub fn as_json(decided: &[Decided], undecidable: &[(Uuid, Undecidable)]) -> Value {
    json!({
        "decided": decided
            .iter()
            .map(|d| {
                let (status, why) = d.columns();
                json!({
                    "hypothesis_id": d.hypothesis_id,
                    "metric": d.metric,
                    "status": status,
                    "why": why,
                    "missing": d.missing,
                })
            })
            .collect::<Vec<_>>(),
        // Named, not counted. A falsifier the platform accepted and cannot read
        // is a defect in the acceptance, and a number would not say which one.
        "undecidable": undecidable
            .iter()
            .map(|(id, e)| json!({ "hypothesis_id": id, "reason": e.to_string() }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn falsifier() -> Falsifier {
        evaluate::parse("sharpe <= 0", ledger::OBJECTIVE_STATISTICS)
            .expect("the vocabulary contains it")
    }

    /// The vocabulary a falsifier is parsed against is the one the ledger can
    /// store. Before migration 0130 this parse failed for every metric a
    /// falsifier could name.
    #[test]
    fn a_falsifier_parses_against_the_storable_vocabulary() {
        let f = falsifier();
        assert_eq!(f.metric, "sharpe");
        assert!(ledger::TRIAL_STATISTICS.contains(&f.metric.as_str()));
    }

    /// A measurement that fires the falsifier refutes, with none of §6.9's
    /// battery — the asymmetry is the design.
    #[test]
    fn a_refutation_needs_no_battery() {
        let mut measurements = BTreeMap::new();
        measurements.insert(
            "sharpe".to_string(),
            vec![
                Measurement { value: -0.2, ticket_id: "t1".into(), se: None },
                Measurement { value: 0.1, ticket_id: "t2".into(), se: None },
                Measurement { value: 0.3, ticket_id: "t3".into(), se: None },
            ],
        );
        let evidence = Evidence {
            measurements,
            n: 3,
            out_of_sample_ticket: None,
            cost_model_ticket: None,
            benchmark_ticket: None,
            repro_spec: None,
            protocol_unstable: false,
            budget_exhausted: false,
        };
        let v = evaluate::evaluate(&falsifier(), Role::Primary, &evidence);
        assert_eq!(v, Verdict::Refuted);
        assert_eq!(
            Decided {
                hypothesis_id: Uuid::nil(),
                verdict: v,
                missing: vec![],
                metric: "sharpe".into()
            }
            .columns(),
            ("refuted", None)
        );
    }

    /// And the honest half: with the platform's current evidence, a hypothesis
    /// that survives its falsifier is **not** `supported` — it is
    /// `evidence_incomplete`, naming the three pieces the schema cannot answer
    /// for.
    ///
    /// This test is the claim in the module header, kept true: if the battery
    /// ever becomes derivable this fails, which is the right time to notice.
    #[test]
    fn a_surviving_hypothesis_is_incomplete_not_supported() {
        let mut measurements = BTreeMap::new();
        measurements.insert(
            "sharpe".to_string(),
            vec![
                Measurement { value: 1.1, ticket_id: "t1".into(), se: None },
                Measurement { value: 1.4, ticket_id: "t2".into(), se: None },
                Measurement { value: 0.9, ticket_id: "t3".into(), se: None },
            ],
        );
        let evidence = Evidence {
            measurements,
            n: 3,
            // What `evidence_for` can prove today, and only that.
            out_of_sample_ticket: Some("t1".into()),
            cost_model_ticket: None,
            benchmark_ticket: None,
            repro_spec: None,
            protocol_unstable: false,
            budget_exhausted: false,
        };
        let v = evaluate::evaluate(&falsifier(), Role::Primary, &evidence);
        assert_eq!(
            v,
            Verdict::Inconclusive {
                why: Why::EvidenceIncomplete
            }
        );
        let missing = evaluate::missing_evidence(&evidence);
        assert_eq!(missing.len(), 3, "{missing:?}");
        assert!(missing.iter().all(|m| !m.is_empty()));
    }

    /// The JSON a run record carries names what was missing rather than
    /// counting it: "evidence incomplete" is not something anyone can act on
    /// and "no benchmark ticket" is.
    #[test]
    fn the_record_names_what_was_missing() {
        let d = Decided {
            hypothesis_id: Uuid::nil(),
            verdict: Verdict::Inconclusive {
                why: Why::EvidenceIncomplete,
            },
            missing: vec!["a benchmark ticket"],
            metric: "sharpe".into(),
        };
        let j = as_json(&[d], &[(Uuid::nil(), Undecidable::Unparseable("no comparator".into()))]);
        assert_eq!(j["decided"][0]["status"], "inconclusive");
        assert_eq!(j["decided"][0]["why"], "evidence_incomplete");
        assert_eq!(j["decided"][0]["missing"][0], "a benchmark ticket");
        assert!(j["undecidable"][0]["reason"]
            .as_str()
            .expect("a reason")
            .contains("comparator"));
    }
}
