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
}
