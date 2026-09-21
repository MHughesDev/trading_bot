//! Two desk fields that were reporting numbers nobody measured (AGENT-012 §3.5,
//! §6.2; ADR-D-25, ADR-D-08).
//!
//! # What these tests are about
//!
//! §3.5 asks the Research Desk for "acceptance criteria **with ticks**". §6.2
//! asks for four budget bars, "with the cap, the 90% wrap-up mark, and the
//! projected end at current burn". The harness emitted neither status nor caps,
//! and the desk filled both gaps from whatever was in reach:
//!
//! * `acceptance` was `Vec<String>`, so a tick would have been a measurement the
//!   desk invented. It rendered a plain list, which was the honest thing to do
//!   with what it was given.
//! * `tokensCap = caps?.tokens ?? budget.tokens_remaining` made `used = cap -
//!   remaining` identically zero, and a run that had burned half a million
//!   tokens rendered `0 / 40,000`. The trials bar did it backwards and printed
//!   `4 / 4 · 90%+` for a run nothing had capped.
//!
//! Both are the same defect: **an absent measurement rendered as a number**. The
//! fix is not on the desk. It is that a status and a cap are facts the harness
//! either holds or does not, and either way it has to say which.
//!
//! # None of these tests could have been written before
//!
//! Not "would have failed" — could not have compiled. `WorkingState.acceptance`
//! had no per-criterion status to assert on, and `BudgetView` had no cap field
//! for tokens, dollars or trials. The two that *could* have been written are
//! [`an_uncounted_axis_is_null_and_not_zero`] and
//! [`the_old_acceptance_wire_shape_still_loads`], and the first of them would
//! have failed: `tokens_remaining` was a `u32` that nothing in the loop ever
//! assigned, so it serialised as `0` — a full budget and a spent one, spelled
//! the same way.

use harness::drive::{Effect, Input, Loop, Task};
use harness::hardware::{Admission, TaskDemand};
use harness::profile::{Budgets, Profile, ProfileSet};
use harness::registry::ToolRegistry;
use harness::state::{AcceptanceCriterion, Detail, GoalKind, Met, WorkingState};
use serde_json::{json, Value};

fn profile() -> Profile {
    ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
        .expect("the shipped profiles load")
        .get("qwen2.5:7b-instruct")
        .expect("the degraded profile ships")
        .clone()
}

fn task() -> Task {
    Task {
        goal: "find whether BTC-USD has momentum at 1h".into(),
        charter: "You are a research agent.".into(),
        demand: TaskDemand::MultiStep,
        namespaces: vec!["data".into()],
    }
}

fn started(p: Profile) -> (Loop, Vec<Effect>) {
    let mut l = Loop::new(p, ToolRegistry::with_core(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    (l, fx)
}

fn wire(state: &WorkingState) -> Value {
    serde_json::to_value(state).expect("the working state serialises")
}

// ── ADR-D-25: the tick is a measurement, so it has three states ─────────────

/// Every criterion the harness derives comes back **unevaluated**, for every kind
/// of goal.
///
/// This is the finding, not a placeholder: nothing in this harness evaluates an
/// acceptance criterion. `on_finish` checks that `finish_task`'s evidence cites
/// ids the session produced — attribution, not satisfaction — and
/// `report::critic` checks that criteria *the model's own draft claims to have
/// met* name evidence, which is a claim read out of model prose. Neither decides
/// whether a criterion holds.
///
/// So the shape ships with the honest value in it. A test that asserted `Met`
/// somewhere would be asserting an evaluator that does not exist.
#[test]
fn every_criterion_the_harness_derives_is_unevaluated() {
    for kind in [
        GoalKind::Lookup,
        GoalKind::Research,
        GoalKind::Diagnose,
        GoalKind::Compare,
    ] {
        let criteria = kind.acceptance_for("find whether BTC-USD has momentum at 1h");
        assert!(!criteria.is_empty(), "{kind:?} derives criteria");
        for c in &criteria {
            assert_eq!(
                c.met,
                Met::Unevaluated,
                "{kind:?} criterion {:?} claims a status nothing measured",
                c.text
            );
        }
    }
}

/// And the loop puts them on the state before the first model call, with that
/// status intact.
#[test]
fn the_loop_starts_with_criteria_that_carry_a_status() {
    let (l, _) = started(profile());
    let acceptance = &l.working().acceptance;
    assert!(!acceptance.is_empty(), "acceptance is derived at step 0");
    for c in acceptance {
        assert_eq!(c.met, Met::Unevaluated);
    }
}

/// `null`, not `false`.
///
/// The whole point of the third state: a consumer that receives `false` renders a
/// red cross against work nobody has looked at, and "unstarted" becomes "failing"
/// in one JSON field. `met` is **present** and null rather than omitted, because
/// an absent key is what a `??` in the consumer reaches past.
#[test]
fn an_unevaluated_criterion_is_null_on_the_wire_and_never_false() {
    let (l, _) = started(profile());
    let v = wire(l.working());
    let criteria = v["acceptance"].as_array().expect("an array of objects");
    assert!(!criteria.is_empty());
    for c in criteria {
        assert!(
            c.get("text").and_then(Value::as_str).is_some(),
            "each criterion carries its text: {c}"
        );
        let met = c.get("met").unwrap_or_else(|| {
            panic!("`met` is present, not omitted — an omitted key is what `??` reaches past: {c}")
        });
        assert_eq!(
            met,
            &Value::Null,
            "an unevaluated criterion is null; `false` would report unstarted work as failed"
        );
    }
}

/// The other two states are ordinary booleans, so a consumer that *does* get a
/// measurement gets something it can render directly.
#[test]
fn a_measured_criterion_round_trips_as_true_or_false() {
    for (met, expected) in [
        (Met::Met, json!(true)),
        (Met::NotMet, json!(false)),
        (Met::Unevaluated, Value::Null),
    ] {
        let c = AcceptanceCriterion {
            text: "the null comparison ran".into(),
            met,
        };
        let v = serde_json::to_value(&c).expect("serialises");
        assert_eq!(v["met"], expected, "{met:?} is {expected} on the wire");
        let back: AcceptanceCriterion = serde_json::from_value(v).expect("round trips");
        assert_eq!(back, c);
    }
}

/// A state persisted before `met` existed still loads, as what it is: text with
/// no evaluation.
///
/// The alternative is a deploy that cannot read yesterday's rows. Note what it
/// must *not* do — invent `false` for the rows that predate the field.
#[test]
fn the_old_acceptance_wire_shape_still_loads() {
    // Built from a current state so the row differs from today's in exactly one
    // way: `acceptance` as it was written before `met` existed.
    let mut v = wire(&WorkingState::new("find whether BTC-USD has momentum at 1h"));
    v["acceptance"] = json!([
        "the claim survives the null it was measured against",
        {"text": "every number reported is in a tool result this run cited", "met": true},
        {"text": "a reproduction is given", "met": null},
    ]);
    let s: WorkingState = serde_json::from_value(v).expect("an old row loads");
    assert_eq!(s.acceptance.len(), 3);
    assert_eq!(
        s.acceptance[0].met,
        Met::Unevaluated,
        "a bare string carries no evaluation, and does not acquire one on the way in"
    );
    assert_eq!(s.acceptance[0].text, "the claim survives the null it was measured against");
    assert_eq!(s.acceptance[1].met, Met::Met);
    assert_eq!(s.acceptance[2].met, Met::Unevaluated);
}

/// The card the model reads says "not yet evaluated" in words.
///
/// `[x]` beside `[ ]` is the shape that makes the third state disappear: an empty
/// box reads as a failed check, and two of these three states are not that.
#[test]
fn the_card_never_renders_an_unevaluated_criterion_as_a_failed_one() {
    let (l, _) = started(profile());
    let card = l.working().render(Detail::Rich);
    assert!(card.contains("DONE MEANS:"), "the section is there: {card}");
    assert!(
        card.contains("[not yet evaluated]"),
        "and it says so in words: {card}"
    );
    assert!(
        !card.contains("[ ]") && !card.contains("[x]"),
        "no checkbox, because an empty one is read as a cross: {card}"
    );
}

/// `Met` must never grow a `bool` escape hatch.
///
/// The type is the enforcement: with no `bool` anywhere on it, `if c.met {…}` and
/// `c.met.unwrap_or(false)` are compile errors, and [`Met::fold`] makes a caller
/// type the word `unevaluated` and choose its rendering on purpose. That property
/// survives only as long as nobody adds the convenience method, so this is the
/// test that says no — a comment saying "don't add `is_met`" is not enforcement.
#[test]
fn met_never_grows_a_bool_escape_hatch() {
    let src = include_str!("../src/state.rs");
    for forbidden in [
        "impl From<Met> for bool",
        "fn is_met",
        "fn met_or",
        "fn as_bool",
        "PartialEq<bool> for Met",
    ] {
        assert!(
            !src.contains(forbidden),
            "`{forbidden}` gives a caller a two-state reading of a three-state fact; \
             use `Met::fold` and name all three"
        );
    }
    // The subtler one: any accessor handing back `Option<bool>` restores
    // `unwrap_or(false)`, which is the exact collapse this type exists to prevent.
    let met_impl = src
        .split("impl Met {")
        .nth(1)
        .expect("`impl Met` is still in state.rs")
        .split("\n}\n")
        .next()
        .expect("with a closing brace");
    assert!(
        !met_impl.contains("Option<bool>"),
        "an `Option<bool>` accessor on `Met` is `unwrap_or(false)` with extra steps"
    );
}

// ── ADR-D-08: a cap and a burn are two facts, and most of them are absent ───

/// The token cap exists in the profile and now reaches the wire.
///
/// This is the missing field ADR-D-08 names. `budgets.max_tokens_per_task` is a
/// real, configured ceiling — 2,000,000 on the degraded profile — and the desk
/// was never sent it, so it substituted the remainder and computed a burn of
/// zero.
#[test]
fn the_token_cap_reaches_the_wire() {
    let p = profile();
    let declared = p.budgets.max_tokens_per_task;
    assert!(declared > 0, "the shipped profile declares a token ceiling");

    let (l, _) = started(p);
    assert_eq!(
        l.working().budget.tokens_cap,
        Some(declared),
        "the cap the profile declares is the cap the desk is told"
    );
    assert_eq!(wire(l.working())["budget"]["tokens_cap"], json!(declared));
}

/// A profile that declares no ceiling sends none — `0` is not a budget of zero.
#[test]
fn a_profile_with_no_token_ceiling_sends_no_cap() {
    let mut p = profile();
    p.budgets.max_tokens_per_task = 0;
    let (l, _) = started(p);
    assert_eq!(
        l.working().budget.tokens_cap,
        None,
        "a declared 0 is a profile that did not set a ceiling, not a ceiling of zero"
    );
    assert_eq!(wire(l.working())["budget"]["tokens_cap"], Value::Null);
}

/// Every axis this loop does not count is `null`, and never `0`.
///
/// The one that would have failed before this change: `tokens_remaining` was a
/// `u32` that no code path in `drive.rs` assigns, so it left the harness as `0`.
/// Sending the cap beside it would have made `used = cap - 0 = cap` — the same
/// invented number as before, pointing the other way, and this time reading as a
/// budget fully spent.
#[test]
fn an_uncounted_axis_is_null_and_not_zero() {
    let (l, _) = started(profile());
    let b = wire(l.working());
    let b = &b["budget"];
    for field in [
        "tokens_remaining",
        "usd_tenth_cents_cap",
        "usd_tenth_cents_remaining",
        "trials_cap",
    ] {
        assert_eq!(
            b[field],
            Value::Null,
            "`{field}` is not measured by this loop, and a 0 there is a measurement"
        );
    }
}

/// The dollar cap is **refused**, even though every profile declares one.
///
/// `budgets.max_usd_per_task` is required and validated positive, and it is still
/// not a number this platform can put on a budget bar: nothing here converts
/// tokens or seconds into money (OQ-16). ADR-D-03 settles that with a versioned
/// price table in which an unverified or stale row is treated as **absent**
/// wherever a cap is reported — so until a fresh row exists for the model in
/// question, the honest wire value is `null`.
///
/// Two assertions, deliberately: the declaration is real, and it still does not
/// travel. A future change that starts sending it has to come past this test with
/// a price table in hand.
#[test]
fn the_dollar_cap_is_refused_although_the_profile_declares_one() {
    let p = profile();
    assert!(
        p.budgets.max_usd_per_task > 0.0,
        "the profile really does declare a per-task dollar ceiling"
    );
    let (l, _) = started(p);
    assert_eq!(
        l.working().budget.usd_tenth_cents_cap,
        None,
        "a declared dollar ceiling is not a priced one: converting it needs a price \
         table (ADR-D-03, OQ-16), and a number sent without one is read as a measurement"
    );
}

/// There is no trials cap in `profile::Budgets` to send.
///
/// ADR-D-08 assumed three caps lived there. Two do — and `max_steps` is in
/// `orchestration`, not `Budgets`, at that. The trial ceilings on this platform
/// are session- and campaign-scoped and live in the API; a `Loop` built from a
/// profile has not been told one, so `trials_cap` is `None` and the bar renders
/// uncapped instead of taking `trials_spent` for its own denominator.
///
/// This test is a tripwire pointing forward: the day somebody adds a trials
/// ceiling to `Budgets`, it fails, and wiring it into `BudgetView` is how it goes
/// green again.
#[test]
fn no_trials_cap_exists_in_the_profile_to_send() {
    let b: Budgets = serde_json::from_value(json!({
        "max_usd_per_task": 5.0,
        "max_tokens_per_task": 2_000_000u64,
    }))
    .expect("Budgets loads from its own fields");
    let keys: Vec<String> = serde_json::to_value(&b)
        .expect("serialises")
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    assert!(
        !keys.iter().any(|k| k.contains("trial")),
        "`profile::Budgets` grew a trials ceiling ({keys:?}) — wire it into \
         `BudgetView::trials_cap`, because a cap that exists and is not sent is the \
         gap the desk filled by inventing one"
    );
}

/// Steps are the one axis that is both capped and counted, and it stays that way.
#[test]
fn the_step_axis_is_capped_and_counted() {
    let mut p = profile();
    p.orchestration.max_steps = 12;
    let (l, _) = started(p);
    let b = l.working().budget;
    assert_eq!(b.steps_total, 12, "the cap is the profile's step ceiling");
    assert!(b.steps_remaining <= b.steps_total);
    let v = wire(l.working());
    assert_eq!(v["budget"]["steps_total"], json!(12));
    assert!(
        v["budget"]["steps_remaining"].is_u64(),
        "steps are genuinely counted, so this one is a number rather than a null"
    );
}

/// No projection is sent, because a projection needs a series.
///
/// §6.2 asks for "the projected end at current burn". There is no burn *series*
/// on this wire — `BudgetView` is one snapshot, rewritten in place each step —
/// and a slope drawn through a single remainder is a forecast with one data
/// point. Refused in both readings, and the absence is asserted so that nothing
/// arrives later without the series it would need.
#[test]
fn no_projection_field_is_sent() {
    let (l, _) = started(profile());
    let b = wire(l.working());
    let keys: Vec<String> = b["budget"]
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    for k in &keys {
        assert!(
            !(k.contains("project") || k.contains("eta") || k.contains("burn_rate")),
            "`{k}` is a forecast, and this struct carries one snapshot, not a series"
        );
    }
}
