//! A battery's verdict reads fields a Study actually answers with (Set S tasks
//! S-8.2, S-8.3).
//!
//! # The defect this exists because of
//!
//! The first version of `precapital_battery` and `leakage_audit` read a
//! `p_value` from each `run_study` step. **A Study does not produce one.** It
//! produces a `Distribution` and a `StudyVerdict` of three booleans, and a
//! reader of an absent field gets `None` — which every verdict function here
//! treats as the conservative branch. So every battery would have rejected every
//! strategy, forever, and the leakage audit's one decisive check would never have
//! fired. Nothing would have failed: the code compiled, the tests passed against
//! fixtures the same author had written, and the verdicts were *plausible*.
//!
//! It is this repository's dominant defect shape — code that is complete, tested
//! and joined to nothing — in the form it takes across a crate boundary, where
//! no compiler can see it. `mcp-server` is a thin HTTP client and does not depend
//! on `backtest`; the JSON in between is the only contract, and nothing checked
//! it.
//!
//! So: build the answer from `backtest`'s own types, serialise it the way the
//! route does, and read it with the accessors the batteries use.

use serde_json::{json, Value};

use backtest::run::MetricKind;
use backtest::study::{Distribution, StudyKind, StudyVerdict};
use mcp_server_lib::tools::workflows::{Ran, Status};

/// One `run_study` answer, built from `backtest`'s own types and serialised the
/// way `api::routes::experiments::run_study` serialises `StudyView`.
///
/// Not the whole `StudyView` — that needs a `SelectionRule` and member ids this
/// test has no opinion about — but every field a battery reads, produced by the
/// types that produce it in production rather than typed out here.
fn study_answer(verdict: &StudyVerdict) -> Value {
    let distribution = Distribution::from_values(MetricKind::DetrendedSharpe, vec![0.4, 0.5, 0.6]);
    json!({
        "study_id": "fam-1",
        "kind": StudyKind::Neighborhood,
        "metric": MetricKind::DetrendedSharpe,
        "question": "is this a region or a point?",
        "trial_delta": 3,
        "sealed": true,
        "distribution": distribution,
        "verdict": verdict,
        "members": [],
    })
}

fn ran(detail: Value) -> Ran {
    Ran {
        tool: "run_study",
        label: "ablation",
        status: Status::Ran,
        ids: std::collections::BTreeMap::new(),
        detail,
    }
}

#[test]
fn the_fields_a_battery_reads_are_the_fields_a_study_answers_with() {
    let verdict = StudyVerdict {
        summary: "v".into(),
        positive_median: true,
        survivable_worst5: true,
        plateau: Some(true),
    };
    let step = ran(study_answer(&verdict));

    assert_eq!(
        step.study_flag("positive_median"),
        Some(true),
        "a battery cannot read `positive_median` off a real Study answer"
    );
    assert_eq!(step.study_flag("survivable_worst5"), Some(true));
    assert_eq!(step.study_flag("plateau"), Some(true));
    assert_eq!(step.study_survived(), Some(true));
}

/// Each of the three booleans travels independently. A reader that returned the
/// same answer for all of them would pass the test above.
#[test]
fn each_of_the_three_booleans_is_read_separately() {
    for (positive, survivable, plateau) in [
        (true, true, Some(true)),
        (false, true, Some(true)),
        (true, false, Some(false)),
        (false, false, None),
    ] {
        let verdict = StudyVerdict {
            summary: "v".into(),
            positive_median: positive,
            survivable_worst5: survivable,
            plateau,
        };
        let step = ran(study_answer(&verdict));
        assert_eq!(step.study_flag("positive_median"), Some(positive));
        assert_eq!(step.study_flag("survivable_worst5"), Some(survivable));
        assert_eq!(step.study_flag("plateau"), plateau);
        assert_eq!(step.study_survived(), Some(positive && survivable));
    }
}

/// The defect, stated as a test: a Study answers with no p-value, and a battery
/// asking for one gets nothing rather than a number.
///
/// Kept so that adding a `p_value` to `StudyVerdict` one day is a decision
/// somebody makes rather than a silent change in what the batteries mean.
#[test]
fn a_study_answers_with_no_p_value() {
    let verdict = StudyVerdict {
        summary: "v".into(),
        positive_median: true,
        survivable_worst5: true,
        plateau: None,
    };
    let answer = study_answer(&verdict);
    let serialised = serde_json::to_string(&answer).expect("serialises");
    assert!(
        !serialised.contains("p_value"),
        "a Study now answers with a p-value; the batteries were written on the assumption that \
         it does not, and their verdict functions should be revisited rather than left to read \
         it by accident"
    );
    assert_eq!(ran(answer).number("p_value"), None);
}

/// `plateau` is a three-valued answer and stays one. `None` means the notion
/// does not apply to this Study kind — a cost sweep has no neighbourhood — and
/// collapsing it into `false` would reject every strategy on a question its
/// Study never asked.
#[test]
fn plateau_keeps_its_third_value() {
    let verdict = StudyVerdict {
        summary: "v".into(),
        positive_median: true,
        survivable_worst5: true,
        plateau: None,
    };
    let step = ran(study_answer(&verdict));
    assert_eq!(step.study_flag("plateau"), None);
    // And the Study still survives: an inapplicable question is not a failure.
    assert_eq!(step.study_survived(), Some(true));
}

/// An answer with no `verdict` object at all — what a route or type change would
/// look like from the battery's side — reads as "did not answer", never as
/// "passed".
#[test]
fn a_shape_change_reads_as_unanswered_and_not_as_a_pass() {
    let step = ran(json!({ "study_id": "fam-1", "sealed": true }));
    assert_eq!(step.study_flag("positive_median"), None);
    assert_eq!(step.study_survived(), None);
}

/// The oracle test's flag, read the same way. `precapital_battery`'s red-team
/// step rejects on `cost_model_is_sane: false`, and that key is at the top level
/// of `POST /api/research/oracle-test`'s answer.
#[test]
fn the_oracle_tests_flag_is_where_the_battery_looks_for_it() {
    let answer = json!({
        "instrument_id": "BTC-USD",
        "round_trip_bps": 10.0,
        "cost_model_is_sane": false,
        "verdict": "FAILURE.",
    });
    let step = Ran {
        tool: "run_oracle_test",
        label: "red-team falsify",
        status: Status::Ran,
        ids: std::collections::BTreeMap::new(),
        detail: answer,
    };
    assert_eq!(step.flag("cost_model_is_sane"), Some(false));
}
