//! Store discipline (AGENT-005 §26; Set S task S-9.17).
//!
//! Four properties, each of which is invisible until a run gets long — which is
//! exactly when this phase's machinery matters and exactly when nobody is
//! watching.

use std::collections::BTreeMap;
use std::time::Instant;

use harness::context::body::{Body, Column};
use harness::context::manifest;
use harness::context::policy::ContextPolicy;
use harness::context::select::{select, Query};
use harness::context::store::{Draft, RecordKind, RunStore, Subject};
use harness::state::GoalKind;

fn table(rows: usize) -> Body {
    Body::Table {
        schema: vec![Column::new("ts", 200), Column::new("close", 200)],
        rows: (0..rows)
            .map(|i| vec![i.to_string(), (40_000 + i).to_string()])
            .collect(),
        total_rows: rows,
    }
}

fn long_run(n: u32) -> RunStore {
    let mut s = RunStore::new();
    for i in 0..n {
        s.insert(
            Draft::new(i, RecordKind::ToolResult, "get_bars", table(20))
                .with_subject(Subject::instrument(format!("SYM-{}", i % 40)))
                .with_id("window", format!("w{i}")),
        );
    }
    s
}

fn query(step: u32) -> Query {
    Query {
        step,
        goal_kind: GoalKind::Research,
        ..Query::default()
    }
}

/// A superseded record is never selected and reads as collapsed (§26.1).
#[test]
fn a_superseded_record_leaves_selection_but_not_the_store() {
    let mut s = RunStore::new();
    let subject = Subject {
        instrument: Some("BTC-USD".into()),
        timeframe: Some("1h".into()),
        ..Subject::default()
    };
    let first = s.insert(
        Draft::new(1, RecordKind::ToolResult, "get_bars", table(20))
            .with_subject(subject.clone())
            .with_id("window", "2024-01..06"),
    );
    let second = s.insert(
        Draft::new(9, RecordKind::ToolResult, "get_bars", table(20))
            .with_subject(subject)
            .with_id("window", "2024-01..06"),
    );

    let sel = select(&s, &query(10), 100_000, &ContextPolicy::v2());
    assert!(
        !sel.ids().contains(&first),
        "the redundancy penalty should not be spending effort suppressing what the store \
         should never have offered twice"
    );
    assert!(sel.ids().contains(&second));

    let m = manifest::render(&s, &sel.ids().into_iter().collect(), 10_000);
    assert!(m.text.contains(&first.to_string()), "still listed:\n{}", m.text);
    assert!(
        m.text.contains(&format!("superseded by {second}")),
        "and shown as superseded:\n{}",
        m.text
    );
}

/// Spill moves bytes, never metadata (§26.3).
#[test]
fn a_spill_is_invisible_except_in_latency_and_expand() {
    let mut s = long_run(300);
    let before: BTreeMap<_, _> = s
        .all()
        .map(|r| (r.id, (r.subject.clone(), r.tokens_full, r.kind)))
        .collect();

    let spilled = s.spill(100, |id| format!("work/context/{id}.json"));
    assert_eq!(s.resident(), 100);
    assert_eq!(spilled.len(), 200);
    assert_eq!(s.len(), 300, "nothing left the store");

    for r in s.all() {
        let (subject, tokens, kind) = &before[&r.id];
        assert_eq!(&r.subject, subject, "{} lost its subject", r.id);
        assert_eq!(&r.tokens_full, tokens, "{} lost its size", r.id);
        assert_eq!(&r.kind, kind);
    }

    // The manifest still describes the whole store, spilled records included.
    let m = manifest::render(&s, &Default::default(), 100_000);
    assert!(m.text.starts_with("CONTEXT (300 records"), "{}", m.text);

    // And `expand` reads one back.
    let (id, _, body) = spilled.into_iter().next().expect("something spilled");
    assert!(
        s.get(id).unwrap().body.render().contains("expand to read it"),
        "a spilled body says how to get it back"
    );
    s.restore(id, body);
    assert!(s.get(id).unwrap().body.render().contains("ts | close"));
}

/// The coldest bodies go first, and a cited or expanded record stays resident.
#[test]
fn what_the_run_is_using_does_not_spill() {
    let mut s = long_run(50);
    let ids: Vec<_> = s.all().map(|r| r.id).collect();
    // The oldest record, which would otherwise be first out.
    let oldest = ids[0];
    s.note_citation(oldest);
    s.note_expansion(oldest);

    s.spill(10, |id| format!("work/{id}"));
    assert!(
        !matches!(
            s.get(oldest).unwrap().body,
            Body::Elsewhere { .. }
        ),
        "a record the model has cited and asked for back is the last thing to spill"
    );
}

/// Assembly time per call stays flat as the store grows past a thousand (§26.4).
///
/// A four-hundred-step run must not spend more wall clock assembling prompts
/// than waiting for models. The assertion is a *ratio* rather than a wall-clock
/// budget, because an absolute threshold measures the machine the test runs on.
#[test]
fn assembly_time_does_not_grow_with_the_store() {
    let policy = ContextPolicy::v2();
    let small = long_run(100);
    let large = long_run(2_000);

    // One warm pass each, so the comparison is not measuring first-touch page
    // faults on the larger store.
    let _ = select(&small, &query(100), 4_000, &policy);
    let _ = select(&large, &query(2_000), 4_000, &policy);

    let time = |s: &RunStore, step: u32| {
        let start = Instant::now();
        for _ in 0..5 {
            let _ = select(s, &query(step), 4_000, &policy);
        }
        start.elapsed().as_secs_f64() / 5.0
    };

    let small_t = time(&small, 100);
    let large_t = time(&large, 2_000);
    // Twenty times the records. Linear signal extraction is unavoidable and
    // fine; what must not happen is the quadratic marginal-relevance loop
    // growing with the store, which is what `MMR_CANDIDATES` bounds.
    assert!(
        large_t < small_t * 40.0 + 0.010,
        "assembly went from {small_t:.6}s at 100 records to {large_t:.6}s at 2,000 — \
         more than linear, so the candidate cap is not holding"
    );
}

/// The cap cannot hide a record the set is required to contain.
///
/// The property that makes bounded scoring safe rather than a silent quality
/// cliff: must-includes and the per-kind floors search the whole store, so a
/// record below the score cut is still reachable.
#[test]
fn a_required_record_survives_the_candidate_cap() {
    let mut s = long_run(2_000);
    // Ancient, off-subject, uncited — it loses on every signal and sits far
    // below the cap.
    let needed = s.insert(
        Draft::new(0, RecordKind::ToolResult, "run_backtest", Body::text("x"))
            .with_id("experiment_id", "exp_42"),
    );
    let correction = s.insert(
        Draft::new(0, RecordKind::Correction, "validate_strategy", Body::text("bad name"))
            .with_id("code", "when_names_string"),
    );

    let q = query(2_000).requiring("exp_42");
    let sel = select(&s, &q, 8_000, &ContextPolicy::v2());
    assert!(sel.ids().contains(&needed), "a must-include is not a candidate");
    assert!(
        sel.ids().contains(&correction),
        "and the per-kind floor searches the whole store, not the scored slice"
    );
}
