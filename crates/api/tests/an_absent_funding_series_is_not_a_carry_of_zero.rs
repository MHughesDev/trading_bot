//! `GET /api/data/funding` must never render an unmeasured carry as zero
//! (Set S S-3.14, S-3.15; DATA-005 §11; D-P3).
//!
//! **Why this series and not another.** Funding is a *cost*. Every other missing
//! measurement in this platform reads as neutral or pessimistic when flattened to
//! zero; a missing funding interval read as zero makes a perpetual free to hold,
//! which biases every carry-paying strategy upward. It is the one series where the
//! absent measurement looks like good news, so it is the one where `[]` and `null`
//! have to be different answers.
//!
//! `storage::clickhouse::forward` already draws the line with types —
//! `SeriesRead::NoSuchSeries` against `Present { rows: [] }`, and `FundingRate` is
//! sealed so that "the only way to obtain one is to decode a row that exists".
//! This file is about the wire, which is where such a distinction is usually lost:
//! a handler that serialises both cases through the same `Vec` turns a sealed type
//! into `[]` at the boundary and nobody notices, because `[]` is what an empty
//! window looks like too.
//!
//! The route also existed for a second reason worth recording: `read_funding`,
//! `read_open_interest` and `funding_at` landed on 2026-09-22 as passengers on a
//! commit about agent pause/resume and had **zero callers anywhere** — no tool, no
//! route, no execution-side consumer. This is the first one.

use api::routes::data::{funding_body, FundingRowOut, FundingShape};
use chrono::{TimeZone, Utc};
use serde_json::json;

fn manifest() -> serde_json::Value {
    json!({
        "type": "funding",
        "venue": "kraken_futures",
        "instrument": "PF_XBTUSD",
        "kind": "realized",
    })
}

fn at(h: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, h, 0, 0).unwrap()
}

fn row(h: u32, forward_evidence: bool) -> FundingRowOut {
    FundingRowOut {
        event_time: at(h),
        knowledge_time: at(h),
        // A string, because the venue's own digits are what INV-05 keeps. `0.0001`
        // through an f64 and back is not `0.0001`.
        rate: "0.0000125".into(),
        relative_rate: None,
        quality_flags: if forward_evidence { 0 } else { 0x8000 },
        source_id: 17,
        is_forward_evidence: forward_evidence,
    }
}

// ── the distinction ──────────────────────────────────────────────────────────

#[test]
fn an_absent_series_answers_null_and_not_an_empty_array() {
    let body = funding_body(
        manifest(),
        FundingShape::Absent {
            table: "funding_rates".into(),
            database: "trading".into(),
        },
    );
    assert!(
        body["funding"].is_null(),
        "an absent series must answer null: `data.funding.length` has to throw \
         rather than read 0, so that `?? []` is a decision somebody made"
    );
    assert!(
        !body["funding"].is_array(),
        "null and [] must not be the same answer"
    );
    assert_eq!(body["manifest"]["measured"], json!(false));
}

#[test]
fn an_empty_window_answers_an_empty_array_and_says_it_was_measured() {
    let body = funding_body(manifest(), FundingShape::Present(Vec::new()));
    assert_eq!(
        body["funding"],
        json!([]),
        "the table exists and the window is empty: that is a measurement, and \
         nothing settled in it"
    );
    assert_eq!(body["manifest"]["measured"], json!(true));
    assert_eq!(body["manifest"]["rows"], json!(0));
}

/// The two cases side by side, because the defect is that they become equal —
/// each assertion above passes on its own against a handler that answers `[]` for
/// both, as long as `measured` is also wrong.
#[test]
fn absence_and_emptiness_are_never_the_same_body() {
    let absent = funding_body(
        manifest(),
        FundingShape::Absent {
            table: "funding_rates".into(),
            database: "trading".into(),
        },
    );
    let empty = funding_body(manifest(), FundingShape::Present(Vec::new()));
    assert_ne!(absent["funding"], empty["funding"]);
    assert_ne!(
        absent["manifest"]["measured"],
        empty["manifest"]["measured"]
    );
}

#[test]
fn the_absence_names_the_table_and_the_database_it_looked_in() {
    let body = funding_body(
        manifest(),
        FundingShape::Absent {
            table: "funding_rates".into(),
            database: "trading".into(),
        },
    );
    let u = &body["manifest"]["unmeasured"];
    assert_eq!(u["reason"], json!("no_such_series"));
    assert_eq!(u["table"], json!("funding_rates"));
    assert_eq!(
        u["database"],
        json!("trading"),
        "the database is part of the answer: the last time somebody asked this \
         question they were connected to the wrong one"
    );
    let msg = u["message"].as_str().expect("a message");
    assert!(
        msg.contains("not a funding rate of zero"),
        "the refusal has to say what it is not, because a client that reads only \
         `funding` will otherwise supply the zero itself: {msg}"
    );
}

// ── forward-evidence admissibility (D-P3) ────────────────────────────────────

/// A backfilled knowledge time is not a receipt, so Gate 15 may not take it. The
/// count is stated on every response rather than left to be derived, because it is
/// invisible from `rows` alone — two windows with four rows each can carry four
/// pieces of forward evidence or none.
#[test]
fn the_manifest_counts_forward_evidence_rows_apart_from_rows() {
    let body = funding_body(
        manifest(),
        FundingShape::Present(vec![row(0, true), row(8, false), row(16, true)]),
    );
    assert_eq!(body["manifest"]["rows"], json!(3));
    assert_eq!(body["manifest"]["forward_evidence_rows"], json!(2));
}

#[test]
fn a_wholly_backfilled_window_reports_no_forward_evidence_rather_than_no_rows() {
    let body = funding_body(
        manifest(),
        FundingShape::Present(vec![row(0, false), row(8, false)]),
    );
    assert_eq!(
        body["manifest"]["rows"],
        json!(2),
        "the rows exist and are readable; they are simply not receipts"
    );
    assert_eq!(body["manifest"]["forward_evidence_rows"], json!(0));
    assert_eq!(body["funding"].as_array().expect("rows").len(), 2);
}

#[test]
fn every_row_carries_its_own_admissibility_and_its_knowledge_time() {
    let body = funding_body(manifest(), FundingShape::Present(vec![row(0, false)]));
    let r = &body["funding"][0];
    assert_eq!(r["is_forward_evidence"], json!(false));
    assert_eq!(r["quality_flags"], json!(0x8000));
    assert!(
        r["knowledge_time"].is_string(),
        "a funding row without its knowledge time cannot be admitted or refused \
         by anything downstream"
    );
}

/// The rate crosses the wire as the venue's digits. A funding rate is on the order
/// of 1e-5, so this is not a rounding nicety: `f64` serialisation is where a
/// per-8-hour rate acquires a seventeenth significant digit that no venue
/// published.
#[test]
fn the_rate_is_a_string_and_not_a_float() {
    let body = funding_body(manifest(), FundingShape::Present(vec![row(0, true)]));
    assert_eq!(body["funding"][0]["rate"], json!("0.0000125"));
    assert!(!body["funding"][0]["rate"].is_number());
}

#[test]
fn an_absent_relative_rate_is_null_rather_than_zero() {
    let body = funding_body(manifest(), FundingShape::Present(vec![row(0, true)]));
    assert!(body["funding"][0]["relative_rate"].is_null());
}

// ── the manifest is not rewritten ────────────────────────────────────────────

#[test]
fn the_shaper_keeps_the_manifest_it_was_given() {
    let body = funding_body(manifest(), FundingShape::Present(Vec::new()));
    assert_eq!(body["manifest"]["venue"], json!("kraken_futures"));
    assert_eq!(body["manifest"]["instrument"], json!("PF_XBTUSD"));
    assert_eq!(
        body["manifest"]["kind"],
        json!("realized"),
        "the kind is the caller's, and a response that reported the other series \
         under the requested name would be worse than an error"
    );
}
