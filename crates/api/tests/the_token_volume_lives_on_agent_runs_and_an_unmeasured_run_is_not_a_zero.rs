//! Two claims, one about where the numbers are and one about what a zero means.
//!
//! # The claim this defends
//!
//! **1. The token volume this platform actually recorded lives on `agent_runs`, not
//! on `llm_usage`.** `llm_usage` is written in exactly one place — the API's LLM
//! proxy — and the agent driver does not call through it. On the deployment this was
//! found on, `llm_usage` held **0 rows** while `agent_runs` held **55**, carrying
//! 2,526,695 `tokens_in` and 27,512 `tokens_out`. The usage endpoint summed the empty
//! table and reported five confident zeros, which read as "this desk spent nothing"
//! and meant "this endpoint reads the wrong table".
//!
//! **2. A run that recorded no token count is an absence, not a zero.** 28 of those
//! 55 runs hold `tokens_in = 0 AND tokens_out = 0`. Folding them into a sum as zeros
//! is how a coverage of 27/55 turns into a total that looks complete — and
//! `COALESCE(SUM(…),0)` over *no* rows is the same bug with the denominator hidden
//! entirely.
//!
//! The two are one file because fixing the first without the second just moves the
//! lie to a bigger table.
//!
//! # Why the shaping is tested and not the SQL
//!
//! Everything worth defending here is about which fields are **absent**, and an
//! absence is the one thing a test against a populated database cannot show you: a
//! database with rows in it produces numbers whichever way the code is wrong. So the
//! pure tests below run in the default suite and carry the contract; the live-database
//! test at the bottom is `#[ignore]`d in the usual way and checks that the query
//! behind it agrees with the database's own arithmetic.
//!
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -p api --test the_token_volume_lives_on_agent_runs_and_an_unmeasured_run_is_not_a_zero \
//!   -- --ignored --test-threads=1
//! ```

use api::routes::agent_sessions::{
    run_token_volume, usage_body, PricedUsage, RunTokenVolume, COST_NOT_RECORDED,
};
use serde_json::Value;

/// `llm_usage` as this deployment has it: nothing at all.
fn empty_llm_usage() -> PricedUsage {
    PricedUsage::default()
}

/// `agent_runs` as this deployment has it, for the user who holds most of it.
fn the_runs_that_were_measured() -> RunTokenVolume {
    RunTokenVolume {
        runs: 55,
        runs_with_tokens: 27,
        input_tokens: Some(2_526_695),
        output_tokens: Some(27_512),
    }
}

#[test]
fn a_run_that_recorded_tokens_is_reported_even_when_llm_usage_is_empty() {
    let body = usage_body(empty_llm_usage(), Some(the_runs_that_were_measured()), 0);

    // The defect in one assertion: 2.5M prompt tokens exist, and the response used to
    // be five zeros because it read the table nobody writes.
    assert_eq!(body["run_input_tokens"], Value::from(2_526_695_i64));
    assert_eq!(body["run_output_tokens"], Value::from(27_512_i64));
}

#[test]
fn the_two_sources_stay_separable() {
    let body = usage_body(empty_llm_usage(), Some(the_runs_that_were_measured()), 0);

    // `llm_usage`'s own counters are untouched by the run totals. A reader comparing
    // `input_tokens` against `run_input_tokens` is comparing two measurements, and
    // the response has to let them.
    assert_eq!(body["input_tokens"], Value::from(0_i64));
    assert_eq!(body["llm_usage_calls"], Value::from(0_i64));
    assert_ne!(body["input_tokens"], body["run_input_tokens"]);

    // And each names the table it came from, so the comparison is interpretable.
    for key in ["llm_usage_source", "run_tokens_source"] {
        let s = body[key].as_str().unwrap_or_default();
        assert!(!s.is_empty(), "{key} must say which table it summed");
    }
    assert!(body["llm_usage_source"]
        .as_str()
        .unwrap()
        .contains("llm_usage"));
    assert!(body["run_tokens_source"]
        .as_str()
        .unwrap()
        .contains("agent_runs"));
}

#[test]
fn the_runs_that_recorded_nothing_are_counted_not_summed() {
    let body = usage_body(empty_llm_usage(), Some(the_runs_that_were_measured()), 0);

    // The coverage is stated, both ways round, so neither number can be read as the
    // whole population by accident.
    assert_eq!(body["runs_total"], Value::from(55_i64));
    assert_eq!(body["runs_with_tokens"], Value::from(27_i64));
    assert_eq!(body["runs_without_tokens"], Value::from(28_i64));

    // 27 runs, not 55, is the denominator of the sums above.
    assert_ne!(body["runs_with_tokens"], body["runs_total"]);
}

#[test]
fn a_scope_where_no_run_recorded_a_count_reports_null_not_zero() {
    // Four runs, none of which recorded anything. The sum over an empty set of
    // measurements is not zero tokens; it is no measurement.
    let nothing_measured = RunTokenVolume {
        runs: 4,
        runs_with_tokens: 0,
        input_tokens: None,
        output_tokens: None,
    };
    let body = usage_body(empty_llm_usage(), Some(nothing_measured), 0);

    assert_eq!(body["run_input_tokens"], Value::Null);
    assert_eq!(body["run_output_tokens"], Value::Null);
    // Still says how many runs it looked at, so `null` reads as "4 runs, none
    // measured" rather than "the field broke".
    assert_eq!(body["runs_total"], Value::from(4_i64));
    assert_eq!(body["runs_without_tokens"], Value::from(4_i64));
}

#[test]
fn no_dollar_figure_is_invented_and_the_response_says_why() {
    let body = usage_body(empty_llm_usage(), Some(the_runs_that_were_measured()), 0);

    // Was `COALESCE(SUM(cost_usd),0)` — an empty table priced at exactly $0.00 beside
    // two and a half million tokens.
    assert_eq!(body["cost_usd"], Value::Null);
    assert_eq!(body["run_cost_usd"], Value::Null);
    assert_eq!(body["dollars_per_verdict"], Value::Null);

    // And the reason travels with the response. A note in the source is not available
    // to the person reading the number.
    let note = body["cost_usd_note"].as_str().expect("a stated reason");
    assert_eq!(note, COST_NOT_RECORDED);
    assert!(note.contains("llm_usage"));
    assert!(note.contains("not estimated"));
}

#[test]
fn a_rate_nobody_can_measure_is_absent_rather_than_zero() {
    // Nothing billable has been sent, so there is no hit rate. `0.0` here is a claim
    // that the cache never hit, which is a different statement from silence.
    let body = usage_body(empty_llm_usage(), Some(the_runs_that_were_measured()), 0);
    assert_eq!(body["cache_hit_rate"], Value::Null);

    // With something billable it is a real fraction again.
    let billed = PricedUsage {
        calls: 3,
        input_tokens: 1_000,
        cache_read: 3_000,
        ..PricedUsage::default()
    };
    let body = usage_body(billed, Some(the_runs_that_were_measured()), 0);
    assert_eq!(body["cache_hit_rate"], Value::from(0.75_f64));
}

#[test]
fn four_measured_zeros_are_distinguishable_from_an_empty_table() {
    // The ambiguity that hid this defect for as long as it hid: four zero counters
    // with no call count behind them. The call count is what tells the two apart.
    let empty = usage_body(empty_llm_usage(), None, 0);
    let really_zero = usage_body(
        PricedUsage {
            calls: 12,
            ..PricedUsage::default()
        },
        None,
        0,
    );

    assert_eq!(empty["input_tokens"], really_zero["input_tokens"]);
    assert_eq!(empty["llm_usage_calls"], Value::from(0_i64));
    assert_eq!(really_zero["llm_usage_calls"], Value::from(12_i64));
}

#[test]
fn a_project_scoped_request_withholds_the_run_totals_rather_than_widening_them() {
    // `agent_runs` has no `project_id`. Answering a project-scoped request with the
    // unscoped totals would hand one project every run on the deployment.
    let body = usage_body(empty_llm_usage(), None, 0);

    assert!(body.get("run_input_tokens").is_none());
    assert!(body.get("runs_total").is_none());
    let note = body["run_tokens_scope_note"]
        .as_str()
        .expect("withholding is explained, not silent");
    assert!(note.contains("project_id"));
}

// ── Against the live database ────────────────────────────────────────────────

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

/// The read agrees with the database's own arithmetic, for every user that has runs.
///
/// Deliberately computes the expectation with an independent query rather than
/// hard-coding today's totals: a test that pins 2,526,695 fails the next time somebody
/// runs an agent, and a test that fails for the right reason only once is a test
/// somebody deletes.
#[tokio::test]
#[ignore = "needs the dev database"]
async fn the_read_matches_what_agent_runs_holds() {
    let pg = sqlx::PgPool::connect(&url())
        .await
        .expect("the dev database is up");

    #[allow(clippy::type_complexity)]
    let users: Vec<(uuid::Uuid, i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT user_id, \
                count(*)::bigint, \
                (count(*) FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint, \
                (SUM(tokens_in)  FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint, \
                (SUM(tokens_out) FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint \
           FROM agent_runs GROUP BY user_id",
    )
    .fetch_all(&pg)
    .await
    .expect("agent_runs is readable");

    assert!(
        !users.is_empty(),
        "no agent_runs rows at all; this test has nothing to check"
    );

    let mut anyone_measured = false;
    for (user_id, runs, with_tokens, t_in, t_out) in users {
        let got = run_token_volume(&pg, user_id, None)
            .await
            .expect("the run token read");
        assert_eq!(got.runs, runs, "run count for {user_id}");
        assert_eq!(got.runs_with_tokens, with_tokens, "coverage for {user_id}");
        assert_eq!(got.input_tokens, t_in, "tokens_in for {user_id}");
        assert_eq!(got.output_tokens, t_out, "tokens_out for {user_id}");

        // The absence rule, checked against real rows rather than a fixture: a user
        // whose every run recorded nothing gets `None`, never `Some(0)`.
        if with_tokens == 0 {
            assert_eq!(got.input_tokens, None, "unmeasured runs for {user_id}");
        } else {
            anyone_measured = true;
            let body = usage_body(PricedUsage::default(), Some(got), 0);
            assert_ne!(
                body["run_input_tokens"],
                Value::Null,
                "a user with measured runs must not be reported as unmeasured"
            );
        }
    }
    assert!(
        anyone_measured,
        "every run on this deployment recorded nothing, which is itself worth knowing"
    );
}
