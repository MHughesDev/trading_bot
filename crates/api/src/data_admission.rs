//! Data admission: DA-07 grade gating and the revised-data check.
//!
//! `data_qc` computes a grade (DATA-005 §7); this is what the grade *does*. Without
//! it, grading is advisory, and an advisory grade is a note in a log that nobody
//! reads until after the result has been written up.
//!
//! Two rules, both about the same failure: a beautiful backtest on data that could
//! not have supported it.
//!
//! - **Grade D refuses an Experiment outright, grade C needs a waiver.** A waiver is
//!   a recorded decision by a human with a stated reason, not a flag; that is the
//!   whole difference between an exception and a loophole.
//! - **A confirmatory run must pin the data revision it read.** Bars are revised.
//!   A run that read "whatever the table says now" cannot be reproduced, and a
//!   result that cannot be reproduced is not evidence — and the *specific* danger is
//!   that a later revision improves the past, so re-reading unpinned silently
//!   upgrades a result nobody re-ran.

use serde::Serialize;
use serde_json::Value;

/// A QC grade (DATA-005 §7).
///
/// `D` refuses an Experiment outright and `C` needs an approved waiver, so the
/// thresholds are a policy decision, not a cosmetic one.
#[must_use]
pub fn grade(coverage_pct: f64, gap_pct: f64, flat_pct: f64) -> &'static str {
    if coverage_pct < 50.0 || gap_pct > 25.0 {
        "D"
    } else if coverage_pct < 80.0 || gap_pct > 10.0 || flat_pct > 20.0 {
        "C"
    } else if coverage_pct < 95.0 || gap_pct > 2.0 || flat_pct > 5.0 {
        "B"
    } else {
        "A"
    }
}

/// Why an Experiment was refused, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub fix: &'static str,
}

/// A recorded human decision to proceed on grade-C data.
#[derive(Debug, Clone)]
pub struct Waiver {
    /// The grade the waiver was granted against. A waiver written for a `C` does not
    /// cover a `D`, and re-grading later must not silently widen it.
    pub granted_for: String,
    /// Who approved it, and why. An empty reason is not a waiver.
    pub approved_by: String,
    pub reason: String,
}

impl Waiver {
    #[must_use]
    pub fn is_valid_for(&self, assessed: &str) -> bool {
        !self.approved_by.trim().is_empty()
            && !self.reason.trim().is_empty()
            && self.granted_for == assessed
    }
}

/// DA-07: may an Experiment run on data of this grade?
///
/// `A` and `B` pass. `C` passes only with a valid waiver. `D` never passes, waiver
/// or not — a waiver system that can wave through anything is a rubber stamp, and
/// grade D means coverage below half or a quarter of the window missing, which is
/// not a judgement call.
pub fn admit_experiment(assessed: &str, waiver: Option<&Waiver>) -> Result<(), Refusal> {
    match assessed {
        "A" | "B" => Ok(()),
        "C" => match waiver {
            Some(w) if w.is_valid_for("C") => Ok(()),
            Some(_) => Err(Refusal {
                code: "data_waiver_invalid",
                message: "the waiver does not name an approver and a reason for this grade".into(),
                fix: "a waiver is a recorded decision: it needs who approved it and why, \
                      and it must have been granted for the grade actually assessed",
            }),
            None => Err(Refusal {
                code: "data_grade_c_needs_waiver",
                message: "grade C data may back an Experiment only with an approved waiver".into(),
                fix: "improve the coverage, pick a cleaner window, or request a waiver \
                      stating who approved it and why",
            }),
        },
        "D" => Err(Refusal {
            code: "data_grade_d_refused",
            message: "grade D data cannot back an Experiment".into(),
            fix: "grade D means under half the expected bars are present or a quarter \
                  of the window is missing; backfill the gaps or choose another window \
                  — this refusal cannot be waived",
        }),
        other => Err(Refusal {
            code: "data_grade_unknown",
            message: format!("unknown data grade {other:?}"),
            fix: "run a data_qc job for this instrument and timeframe first",
        }),
    }
}

/// Whether a job manifest pins the data it read.
///
/// A confirmatory run must carry `data_snapshot_id` and an `as_of`. Exploratory work
/// need not: the point of exploration is to look around, and demanding a pin for
/// every glance would make the honest path the expensive one.
pub fn check_revision_pinning(manifest: &Value, confirmatory: bool) -> Result<(), Refusal> {
    if !confirmatory {
        return Ok(());
    }
    let snapshot = manifest
        .get("data_snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if snapshot.is_empty() {
        return Err(Refusal {
            code: "data_not_pinned",
            message: "a confirmatory run must pin the data revision it read".into(),
            fix: "include the data_snapshot_id from the read's response manifest, so the \
                  run can be reproduced against the same revisions",
        });
    }
    if manifest.get("as_of").and_then(Value::as_str).is_none()
        && manifest.get("as_of").and_then(Value::as_i64).is_none()
    {
        return Err(Refusal {
            code: "data_not_pinned",
            message: "a confirmatory run must state the as-of instant it read at".into(),
            fix: "include as_of; without it a later revision silently changes the past \
                  this result was measured against",
        });
    }
    // A run that asks for revisions later than the window it claims to test is
    // reading corrections it could not have had. This is the shape of the leak:
    // not a future *bar*, a future *edit* to a past bar.
    if let (Some(as_of), Some(end)) = (
        manifest.get("as_of").and_then(Value::as_str),
        manifest.get("window_end").and_then(Value::as_str),
    ) {
        if as_of < end {
            return Err(Refusal {
                code: "data_as_of_before_window_end",
                message: format!(
                    "as_of {as_of} is before the window end {end}, so part of the window \
                     was unreadable at that instant"
                ),
                fix: "set as_of at or after the window end",
            });
        }
    }
    Ok(())
}

// ── Experiment creation gate (Set P P-2.3) ───────────────────────────────────
//
// `admit_experiment` above was complete and tested and its only caller was the
// auditor's fixture runner: the create handler never asked it anything, so a grade-D
// series backed Experiments freely. This is the part that asks.
//
// The grade it reads is the **persisted battery grade** (`data_qc_reports`, P-2.2),
// never a recomputation with `grade()`'s four thresholds, which disagree with the
// battery and would let the admission gate and the manifest print different letters.

/// Whether a series with **no persisted report** refuses an Experiment.
///
/// `true` since 2026-10-04 (ADR-P2-3a). It was `false` while nothing wrote
/// `data_qc_reports` (the P-2.6 sequencing rule: refusing everything would be an
/// outage). The nightly census job now grades every series that holds bars
/// (`apps/platform/src/census_jobs.rs`) and the 11 live series have reports, so
/// "ungraded" now means a series with no bars or one that appeared since the last
/// pass, and it is refused with the way to get it graded. Rejected: leaving it
/// `false` (an Experiment on data nobody graded is the case DA-07 exists to stop).
pub const REFUSE_UNGRADED: bool = true;

/// One series an Experiment declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeriesRef {
    pub instrument: String,
    pub timeframe: String,
}

/// The series a `universe_ref` names, and the parts that cannot be graded here.
///
/// A `universe_ref` is free text to the suite. This reads the form a human or agent
/// writes for explicit data: comma-separated symbols. `selection:<ref>` universes
/// are resolved per decision instant at run time, so they cannot be graded here and
/// are returned as `skipped`; they are not silently treated as clean.
#[must_use]
pub fn series_of(
    universe_ref: &str,
    eval_resolution: Option<&str>,
) -> (Vec<SeriesRef>, Vec<String>) {
    let timeframe = eval_resolution.unwrap_or("1d").to_string();
    let mut series = Vec::new();
    let mut skipped = Vec::new();
    for part in universe_ref
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        if part.starts_with("selection:") {
            skipped.push(part.to_string());
        } else {
            series.push(SeriesRef {
                instrument: part.to_string(),
                timeframe: timeframe.clone(),
            });
        }
    }
    (series, skipped)
}

/// What the gate found for one series.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesVerdict {
    pub instrument: String,
    pub timeframe: String,
    /// `None`: no persisted report.
    pub grade: Option<String>,
    pub report_id: Option<uuid::Uuid>,
    pub reasons: Value,
    /// The refusal's code when the series did not pass.
    pub refusal: Option<&'static str>,
    pub fix: Option<&'static str>,
    /// The waiver that admitted a grade-C series.
    pub waiver_id: Option<uuid::Uuid>,
}

impl SeriesVerdict {
    #[must_use]
    pub fn admitted(&self) -> bool {
        self.refusal.is_none()
    }
}

/// A persisted grade as the gate reads it: letter, report id, reasons.
pub type GradedSeries = (String, uuid::Uuid, Value);

/// Decide one series from its grade and (for C) the waiver the caller offered.
///
/// Pure. There is no token, scope or role among its inputs: no credential can
/// bypass it because none reaches it, which is what "no scope can bypass the
/// check" means when the check is a function of the data alone.
#[must_use]
pub fn decide_series(
    series: &SeriesRef,
    graded: Option<GradedSeries>,
    waiver: Option<(uuid::Uuid, Waiver)>,
) -> SeriesVerdict {
    let mut v = SeriesVerdict {
        instrument: series.instrument.clone(),
        timeframe: series.timeframe.clone(),
        grade: None,
        report_id: None,
        reasons: Value::Null,
        refusal: None,
        fix: None,
        waiver_id: None,
    };
    let Some((letter, report_id, reasons)) = graded else {
        if REFUSE_UNGRADED {
            v.refusal = Some("data_ungraded");
            v.fix = Some(
                "this series has no persisted data_qc report: the nightly census grades every                  series that holds bars, so either it holds none (collect or backfill it) or                  the grading pass has not run since it appeared; submit a data_qc job for                  this instrument and timeframe to grade it now",
            );
        }
        return v;
    };
    v.grade = Some(letter.clone());
    v.report_id = Some(report_id);
    v.reasons = reasons;
    let (waiver_id, waiver) = match waiver {
        Some((id, w)) => (Some(id), Some(w)),
        None => (None, None),
    };
    match admit_experiment(&letter, waiver.as_ref()) {
        Ok(()) => {
            if letter == "C" {
                v.waiver_id = waiver_id;
            }
        }
        Err(r) => {
            v.refusal = Some(r.code);
            v.fix = Some(r.fix);
        }
    }
    v
}

/// Look up an approved `qc_waiver` for one series.
///
/// Valid only if the approval is `kind = 'qc_waiver'`, answered by a human
/// (`answered_by` set), the answer is `approve` with a non-empty note (the reason),
/// and its payload names this exact `(instrument, timeframe)` and the grade `C`.
/// A waiver for one series does not cover another, and one granted for `C` does not
/// cover a later `D` ([`Waiver::is_valid_for`]).
pub async fn find_waiver(
    pg: &sqlx::PgPool,
    approval_id: uuid::Uuid,
    series: &SeriesRef,
) -> Option<(uuid::Uuid, Waiver)> {
    type Row = (Value, Option<Value>, Option<uuid::Uuid>);
    let (payload, answer, answered_by): Row = sqlx::query_as(
        "SELECT payload, answer, answered_by FROM approval_requests
          WHERE approval_id = $1 AND kind = 'qc_waiver' AND state = 'answered'",
    )
    .bind(approval_id)
    .fetch_optional(pg)
    .await
    .ok()
    .flatten()?;
    let answered_by = answered_by?;
    let answer = answer?;
    if answer.get("option").and_then(Value::as_str) != Some("approve") {
        return None;
    }
    let names = |k: &str| payload.get(k).and_then(Value::as_str);
    if names("instrument") != Some(series.instrument.as_str())
        || names("timeframe") != Some(series.timeframe.as_str())
    {
        return None;
    }
    Some((
        approval_id,
        Waiver {
            granted_for: names("grade").unwrap_or("").to_string(),
            approved_by: answered_by.to_string(),
            reason: answer
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
    ))
}

/// Check every series an Experiment declares. Returns all verdicts; the caller
/// refuses if any is not admitted, and the 422 lists **every** failing series with
/// its reasons: an error that names only the first is one that gets fixed one
/// retry at a time.
pub async fn check_experiment_data(
    pg: &sqlx::PgPool,
    universe_ref: &str,
    eval_resolution: Option<&str>,
    waiver_id: Option<uuid::Uuid>,
) -> (Vec<SeriesVerdict>, Vec<String>) {
    let (series, skipped) = series_of(universe_ref, eval_resolution);
    let mut out = Vec::with_capacity(series.len());
    for s in &series {
        let graded = storage::clickhouse::qc_reports::latest_grade(pg, &s.instrument, &s.timeframe)
            .await
            .ok()
            .flatten()
            .map(|g| (g.grade, g.report_id, g.reasons));
        let waiver = match (&graded, waiver_id) {
            (Some((letter, _, _)), Some(id)) if letter == "C" => find_waiver(pg, id, s).await,
            _ => None,
        };
        out.push(decide_series(s, graded, waiver));
    }
    (out, skipped)
}

/// The 422 body: `data_quality_insufficient`, every failing series, with reasons.
#[must_use]
pub fn refusal_body(verdicts: &[SeriesVerdict]) -> Value {
    let failing: Vec<&SeriesVerdict> = verdicts.iter().filter(|v| !v.admitted()).collect();
    let names = failing
        .iter()
        .map(|v| {
            format!(
                "{} {} grade {}",
                v.instrument,
                v.timeframe,
                v.grade.as_deref().unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    serde_json::json!({
        "error": "data_quality_insufficient",
        "message": format!(
            "{} of {} series cannot back an Experiment: {names}",
            failing.len(),
            verdicts.len()
        ),
        "series": failing,
    })
}

/// Record the admission on the Experiment's lineage, one row per series.
///
/// # Errors
/// The database; the caller reports `lineage_recorded: false` rather than
/// pretending, since the Experiment already exists.
pub async fn record_admission(
    pg: &sqlx::PgPool,
    experiment_id: &str,
    user_id: uuid::Uuid,
    verdicts: &[SeriesVerdict],
) -> Result<(), sqlx::Error> {
    for v in verdicts.iter().filter(|v| v.admitted()) {
        sqlx::query(
            "INSERT INTO experiment_data_admission
               (experiment_id, user_id, instrument_id, timeframe, report_id, grade, waiver_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(experiment_id)
        .bind(user_id)
        .bind(&v.instrument)
        .bind(&v.timeframe)
        .bind(v.report_id)
        .bind(v.grade.as_deref())
        .bind(v.waiver_id)
        .execute(pg)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good_waiver() -> Waiver {
        Waiver {
            granted_for: "C".into(),
            approved_by: "mason".into(),
            reason: "the gap is a venue outage, not a collection failure".into(),
        }
    }

    #[test]
    fn grades_run_from_a_to_d() {
        assert_eq!(grade(99.9, 0.1, 0.0), "A");
        assert_eq!(grade(97.0, 3.0, 0.0), "B");
        assert_eq!(grade(85.0, 15.0, 0.0), "C");
        assert_eq!(grade(40.0, 60.0, 0.0), "D");
    }

    #[test]
    fn a_and_b_pass_without_ceremony() {
        assert!(admit_experiment("A", None).is_ok());
        assert!(admit_experiment("B", None).is_ok());
    }

    #[test]
    fn c_needs_a_waiver_and_the_refusal_says_so() {
        let err = admit_experiment("C", None).unwrap_err();
        assert_eq!(err.code, "data_grade_c_needs_waiver");
        assert!(err.fix.contains("waiver"));
        assert!(admit_experiment("C", Some(&good_waiver())).is_ok());
    }

    #[test]
    fn a_waiver_without_a_reason_is_not_a_waiver() {
        let mut w = good_waiver();
        w.reason = "   ".into();
        assert_eq!(
            admit_experiment("C", Some(&w)).unwrap_err().code,
            "data_waiver_invalid"
        );
        let mut w = good_waiver();
        w.approved_by = String::new();
        assert_eq!(
            admit_experiment("C", Some(&w)).unwrap_err().code,
            "data_waiver_invalid"
        );
    }

    /// The one that matters: a waiver granted for C must not silently cover D.
    #[test]
    fn grade_d_cannot_be_waived() {
        assert_eq!(
            admit_experiment("D", None).unwrap_err().code,
            "data_grade_d_refused"
        );
        assert_eq!(
            admit_experiment("D", Some(&good_waiver()))
                .unwrap_err()
                .code,
            "data_grade_d_refused"
        );
        let d_waiver = Waiver {
            granted_for: "D".into(),
            approved_by: "mason".into(),
            reason: "I would like this to work".into(),
        };
        assert_eq!(
            admit_experiment("D", Some(&d_waiver)).unwrap_err().code,
            "data_grade_d_refused",
            "a waiver system that can wave through grade D is a rubber stamp"
        );
    }

    #[test]
    fn an_ungraded_instrument_is_refused_rather_than_assumed_good() {
        let err = admit_experiment("", None).unwrap_err();
        assert_eq!(err.code, "data_grade_unknown");
    }

    #[test]
    fn exploration_does_not_have_to_pin() {
        assert!(check_revision_pinning(&json!({}), false).is_ok());
    }

    #[test]
    fn a_confirmatory_run_must_pin_the_snapshot_and_the_as_of() {
        assert_eq!(
            check_revision_pinning(&json!({}), true).unwrap_err().code,
            "data_not_pinned"
        );
        assert_eq!(
            check_revision_pinning(&json!({"data_snapshot_id": "snap_1"}), true)
                .unwrap_err()
                .code,
            "data_not_pinned"
        );
        assert!(check_revision_pinning(
            &json!({"data_snapshot_id": "snap_1", "as_of": "2026-01-01T00:00:00Z"}),
            true
        )
        .is_ok());
    }

    #[test]
    fn reading_at_an_instant_before_the_window_closed_is_refused() {
        let err = check_revision_pinning(
            &json!({
                "data_snapshot_id": "snap_1",
                "as_of": "2026-01-01T00:00:00Z",
                "window_end": "2026-06-01T00:00:00Z",
            }),
            true,
        )
        .unwrap_err();
        assert_eq!(err.code, "data_as_of_before_window_end");
    }
    // ── the creation gate (P-2.3) ────────────────────────────────────────────

    fn series(i: &str) -> SeriesRef {
        SeriesRef {
            instrument: i.into(),
            timeframe: "1m".into(),
        }
    }
    fn graded(l: &str) -> Option<GradedSeries> {
        Some((l.into(), uuid::Uuid::nil(), json!(["why"])))
    }

    #[test]
    fn a_grade_d_series_is_refused_with_its_reasons_listed() {
        let v = decide_series(&series("BTC-USD"), graded("D"), None);
        assert_eq!(v.refusal, Some("data_grade_d_refused"));
        let body = refusal_body(&[v]);
        assert_eq!(body["error"], "data_quality_insufficient");
        assert_eq!(body["series"][0]["reasons"], json!(["why"]));
    }

    #[test]
    fn grade_c_fails_without_a_waiver_and_passes_with_one() {
        assert_eq!(
            decide_series(&series("X"), graded("C"), None).refusal,
            Some("data_grade_c_needs_waiver")
        );
        let id = uuid::Uuid::from_u128(7);
        let v = decide_series(&series("X"), graded("C"), Some((id, good_waiver())));
        assert!(v.admitted());
        assert_eq!(
            v.waiver_id,
            Some(id),
            "the waiver id travels to the lineage"
        );
    }

    /// A waiver is never recorded against a grade that did not need it, and never
    /// rescues a D.
    #[test]
    fn a_waiver_neither_decorates_a_b_nor_rescues_a_d() {
        let id = uuid::Uuid::from_u128(7);
        let b = decide_series(&series("X"), graded("B"), Some((id, good_waiver())));
        assert_eq!(b.waiver_id, None);
        assert!(!decide_series(&series("X"), graded("D"), Some((id, good_waiver()))).admitted());
    }

    #[test]
    fn every_failing_series_is_listed_not_just_the_first() {
        let vs = vec![
            decide_series(&series("A"), graded("D"), None),
            decide_series(&series("B"), graded("B"), None),
            decide_series(&series("C"), graded("C"), None),
        ];
        assert_eq!(refusal_body(&vs)["series"].as_array().unwrap().len(), 2);
    }

    /// ADR-P2-3a: ungraded is refused, with the way to get it graded, and never
    /// reads as a letter.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn an_ungraded_series_is_refused_and_told_how_to_get_graded() {
        assert!(REFUSE_UNGRADED);
        let v = decide_series(&series("NEW"), None, None);
        assert!(!v.admitted());
        assert_eq!(v.refusal, Some("data_ungraded"));
        assert!(v.fix.unwrap().contains("data_qc job"));
        assert!(
            v.grade.is_none(),
            "it must read as ungraded, never as a letter"
        );
    }

    #[test]
    fn a_universe_ref_names_its_series_and_selections_are_skipped_not_cleaned() {
        let (named, skipped) = series_of("BTC-USD, ETH-USD,selection:top5", Some("1h"));
        assert_eq!(named.len(), 2);
        assert_eq!(named[0].timeframe, "1h");
        assert_eq!(skipped, vec!["selection:top5".to_string()]);
        assert_eq!(series_of("BTC-USD", None).0[0].timeframe, "1d");
    }

    /// P-2.3 clause 4: no token scope bypasses it. The decision has no scope input,
    /// and the handler calls the gate before it creates anything with no scope
    /// branch in between, so each scope is checked by name against both facts.
    #[test]
    fn no_token_scope_can_bypass_the_creation_gate() {
        let handler = include_str!("routes/experiments.rs");
        let at = handler.find("pub async fn create_experiment(").unwrap();
        let body = &handler[at..at + handler[at..].find("\n}").unwrap()];
        let gate = body
            .find("check_experiment_data(")
            .expect("the handler calls the gate");
        let create = body.find("state.suite.create_experiment(").unwrap();
        assert!(gate < create, "the gate must precede creation");
        let between = &body[gate..create];
        assert!(
            !between.contains("permits("),
            "no scope branch may sit between"
        );
        for scope in crate::auth::scopes::RESEARCH_SCOPES.iter().copied().chain([
            crate::auth::scopes::WEB_FULL,
            crate::auth::scopes::TEXT_READER,
        ]) {
            assert!(
                !between.contains(scope),
                "scope {scope} must not appear in the gate path"
            );
            assert_eq!(
                decide_series(&series("X"), graded("D"), None).refusal,
                Some("data_grade_d_refused"),
                "scope {scope}"
            );
        }
    }
}
