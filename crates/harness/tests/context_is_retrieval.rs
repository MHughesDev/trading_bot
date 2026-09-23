//! The case Part II exists for (AGENT-005 §30.3; Set S task S-9.5).
//!
//! # The fixture
//!
//! A forty-step run where the observation that decides step 30 arrives at step
//! 3. Under recency ordering it is long gone by the time it matters — not
//! because anything went wrong, but because that is what recency ordering *is*.
//! Under selection it is rendered, because the step names the id it carries.
//!
//! This is the phase's headline regression test. Both policy versions are named
//! in the assertions, so the difference between them is the thing being
//! asserted rather than a side effect of some other change.
//!
//! # Why the two halves are checked differently
//!
//! `v1` is Part I, so the question "does the deciding record survive" is
//! answered by Part I's own assembler: build the blocks in arrival order, give
//! it the budget, and read what came out. `v2` is retrieval, so the question is
//! answered by `select`. Running both through `select` would prove nothing —
//! `v1`'s selection is a no-op by construction, and the trimming that loses the
//! record happens downstream of it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use harness::context::body::{Body, Column};
use harness::context::policy::ContextPolicy;
use harness::context::select::{select, Query};
use harness::context::store::{Draft, RecordKind, RunStore, Subject};
use harness::context::{assemble, Block, Section};
use harness::profile::{Profile, ProfileSet};
use harness::provenance::Tagged;
use harness::state::GoalKind;

/// The id the deciding record carries and step 30 needs.
const DECIDING_ID: &str = "exp_the_one_that_matters";
/// A phrase that appears only in the deciding record.
const DECIDING_PHRASE: &str = "funding flipped negative on the third of April";

fn profile() -> Profile {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("config")
        .join("profiles");
    ProfileSet::load_dir(&dir)
        .expect("the shipped profiles load")
        .get("qwen2.5:7b-instruct")
        .expect("the smallest window, which is where this matters most")
        .clone()
}

/// Forty steps of ordinary work, with the deciding observation at step 3.
fn forty_step_run() -> RunStore {
    let mut store = RunStore::new();
    for step in 0..40u32 {
        if step == 3 {
            store.insert(
                Draft::new(
                    step,
                    RecordKind::ToolResult,
                    "get_funding",
                    Body::Table {
                        schema: vec![Column::new("date", 200), Column::new("rate", 200)],
                        rows: vec![vec!["2024-04-03".into(), "-0.0004".into()]],
                        total_rows: 1,
                    },
                )
                .with_subject(Subject {
                    instrument: Some("ETH-USD".into()),
                    ..Subject::default()
                })
                .with_id("experiment_id", DECIDING_ID)
                .with_id("note", DECIDING_PHRASE),
            );
            continue;
        }
        // Filler: real work about something else, each large enough that forty
        // of them cannot fit a 7B's window.
        store.insert(
            Draft::new(
                step,
                RecordKind::ToolResult,
                "get_bars",
                Body::text(format!("bars for step {step}: {}", "x".repeat(1200))),
            )
            .with_subject(Subject::instrument(format!("SYM-{step}")))
            .with_id("window", format!("w{step}")),
        );
    }
    store
}

/// Part I's prompt, built the way Part I builds it: arrival order, equal
/// priority, and the assembler trims.
fn part_one_prompt(store: &RunStore, profile: &Profile) -> String {
    let blocks: Vec<Block> = store
        .live()
        .map(|r| Block {
            section: Section::WorkingState,
            tagged: Tagged::system(r.source.clone(), r.body.render()),
            priority: 100,
            reference: Some(format!("workspace://{}", r.id)),
        })
        .collect();
    assemble(profile, blocks).text()
}

#[test]
fn the_record_that_decides_step_thirty_arrives_at_step_three() {
    let store = forty_step_run();
    let profile = profile();

    // ---- v1: recency ordering ----------------------------------------------
    let v1 = ContextPolicy::v1();
    assert!(!v1.is_retrieval(), "v1 is the control");
    let prompt = part_one_prompt(&store, &profile);
    assert!(
        !prompt.contains(DECIDING_PHRASE),
        "under policy {} the deciding observation is gone by step 30 — that is not a \
         defect in Part I, it is what recency ordering is",
        v1.version
    );

    // ---- v2: retrieval ------------------------------------------------------
    let v2 = ContextPolicy::v2();
    let query = Query {
        step: 30,
        goal_kind: GoalKind::Research,
        ..Query::default()
    }
    .with_subject(Subject::instrument("ETH-USD"))
    .requiring(DECIDING_ID);

    let budget = profile.input_budget_tokens() / 2;
    let selection = select(&store, &query, budget, &v2);

    let rendered: BTreeSet<_> = selection.ids().into_iter().collect();
    let deciding = store
        .live()
        .find(|r| r.ids.get("experiment_id").map(String::as_str) == Some(DECIDING_ID))
        .expect("the fixture inserted it");
    assert!(
        rendered.contains(&deciding.id),
        "under policy {} the deciding record must be rendered; selected {:?}",
        v2.version,
        selection.report.selected
    );

    // And it is rendered whole: a record pinned by a requirement is not one the
    // fitter may degrade to make room.
    let kept = selection
        .fit
        .kept
        .iter()
        .find(|k| k.id == deciding.id)
        .expect("kept");
    assert!(
        kept.degraded.is_none(),
        "a must-include is pinned, so the step gets the record rather than its schema"
    );
    assert!(
        kept.body.render().contains("-0.0004"),
        "and the number the step needs survived"
    );
}

/// Selection is a no-op when everything fits.
///
/// The property that makes this phase safe to ship: a short run behaves exactly
/// as it does today, and the machinery only engages under the pressure it exists
/// for.
#[test]
fn a_run_that_fits_is_unchanged() {
    let mut store = RunStore::new();
    for step in 0..5u32 {
        store.insert(
            Draft::new(
                step,
                RecordKind::ToolResult,
                "get_bars",
                Body::text("small"),
            )
            .with_subject(Subject::instrument(format!("SYM-{step}"))),
        );
    }
    let query = Query {
        step: 5,
        goal_kind: GoalKind::Research,
        ..Query::default()
    };
    let sel = select(&store, &query, 100_000, &ContextPolicy::v2());
    assert_eq!(
        sel.fit.kept.len(),
        5,
        "everything fits, so everything renders"
    );
    assert!(sel.fit.dropped.is_empty());
    assert!(sel.fit.kept.iter().all(|k| k.degraded.is_none()));
}

/// No record is ever byte-truncated on the record path.
///
/// Part I's `truncate_middle` still exists — the transcript and the tool-result
/// cap use it, and they are not records. This asserts it is not reachable from
/// the store path, by checking the property rather than the call graph: every
/// rendered body still parses as the shape it claims to be.
#[test]
fn no_record_is_ever_byte_truncated() {
    let store = forty_step_run();
    let query = Query {
        step: 39,
        goal_kind: GoalKind::Research,
        ..Query::default()
    };
    // A budget far under what the set needs, so the fitter has to work hard.
    let sel = select(&store, &query, 300, &ContextPolicy::v2());
    for kept in &sel.fit.kept {
        let text = kept.body.render();
        if let Body::Table { schema, .. } = &kept.body {
            let header = text.lines().next().unwrap_or_default();
            for c in schema {
                assert!(
                    header.contains(&c.name),
                    "a degraded table still names its schema: {text}"
                );
            }
        }
        assert!(
            !text.contains("… "),
            "Part I's elision marker must not appear on a record: {text}"
        );
    }
    // Everything the budget could not hold is still in the store and still
    // addressable, which is what makes the loss recoverable.
    for id in &sel.fit.dropped {
        assert!(
            store.get(*id).is_some(),
            "{id} left the prompt, not the store"
        );
    }
}
