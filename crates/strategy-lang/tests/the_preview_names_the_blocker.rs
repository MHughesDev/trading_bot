//! The preview says which clause blocked an entry (S-5.15; AGENT-011 §4).
//!
//! The acceptance, in its own terms: *"the blocking profile identifies the sole
//! blocker on a fixture built to have one."*
//!
//! # What is real here
//!
//! The evaluator. Every number below is `bars_true` from
//! `backtest::preview::preview_signals` — the function the API's preview route
//! calls — run over [`strategy_lang::probes`]' definitions. Nothing in
//! `strategy-lang` walks a bar; it only builds the questions (each a preview of
//! the same definition with one expression swapped in) and reads the answers
//! back as a tally. That is the point of the design: no second evaluator, so no
//! second set of answers about when a clause is true.
//!
//! # What is not
//!
//! The bars are the fixture in `support`, not a market.

mod support;

use backtest::preview::SignalPreview;
use strategy_lang::{blocking_profile, probes, Verdict};
use support::{compiled, hourly_bars, preview};

/// Run every probe of every entry and fold the answers into a profile, the way
/// the MCP tool does over HTTP.
fn profile_of(text: &str) -> (strategy_lang::BlockingProfile, SignalPreview) {
    let def = compiled(text);
    let bars = hourly_bars(1_500, 7);
    let whole = preview(&def, &bars);
    let mut tallies = Vec::new();
    for set in probes(&def).expect("probes") {
        let mut answers = Vec::new();
        let mut evaluated = None;
        for probe in set.probes() {
            let p = preview(&probe.definition, &bars);
            // Same warm-up for every probe: the whole reason the subtraction
            // inside `counts` is legitimate.
            assert_eq!(
                *evaluated.get_or_insert(p.bars_evaluated),
                p.bars_evaluated,
                "{:?} evaluated a different window from the others",
                probe.measures
            );
            answers.push(
                p.signals
                    .iter()
                    .find(|s| s.signal == probe.signal)
                    .map_or(0, |s| s.bars_true) as u64,
            );
        }
        tallies.push(
            set.counts(evaluated.unwrap_or(0) as u64, &answers)
                .unwrap_or_else(|d| panic!("{d:#?}")),
        );
    }
    let profile = blocking_profile(&def, &tallies).expect("profile");
    (profile, whole)
}

fn long_true(p: &SignalPreview) -> usize {
    p.signals
        .iter()
        .find(|s| s.signal == "long")
        .map_or(0, |s| s.bars_true)
}

/// `rsi(close, 14) < 0` cannot be true — an RSI is never below zero — so it is a
/// threshold with a sign error in it, which is the most common way a strategy
/// never fires. The other clause is true on almost every bar, so the blame has
/// exactly one address.
const SIGN_ERROR: &str = "\
strategy sign_error on $bound_at_init timeframe 1h
entry long when ema(close, 5) > 0 and rsi(close, 14) < 0
size fixed 0.01
exit time 24 bars
";

#[test]
fn the_profile_names_the_one_clause_that_blocks_the_entry() {
    let (profile, whole) = profile_of(SIGN_ERROR);
    assert_eq!(long_true(&whole), 0, "the fixture was built never to fire");
    assert_eq!(whole.entries, 0);

    let entry = &profile.entries[0];
    assert_eq!(entry.path, "entry.long[0]");
    let Verdict::Blocked {
        clause,
        sole_blocker_share,
    } = &entry.verdict
    else {
        panic!(
            "expected one blocker, got {:?}\n{}",
            entry.verdict,
            profile.render()
        );
    };
    assert_eq!(*clause, 1, "{}", profile.render());
    assert_eq!(entry.clauses[*clause].text, "rsi(close, 14) < 0");
    assert!(*sole_blocker_share > 0.95, "{sole_blocker_share}");
    // The other clause was fine and the profile says so: it is true nearly
    // everywhere and blocks nothing.
    assert!(entry.clauses[0].true_share > 0.95);
    assert!(entry.clauses[0].sole_blocker_share < 0.01);
    assert_eq!(entry.clauses[1].bars_true, 0);
    // And the finding is the sentence the spec asks for.
    let words = entry.words();
    assert!(
        words.contains("`rsi(close, 14) < 0` was true on 0.0% of bars and was the sole blocker"),
        "{words}"
    );
}

/// The same fixture with the clauses written the other way round. A profile that
/// named clause 1 because it was last would pass the test above.
#[test]
fn it_is_the_clause_and_not_its_position() {
    let swapped = SIGN_ERROR.replace(
        "ema(close, 5) > 0 and rsi(close, 14) < 0",
        "rsi(close, 14) < 0 and ema(close, 5) > 0",
    );
    let (profile, _) = profile_of(&swapped);
    let Verdict::Blocked { clause, .. } = &profile.entries[0].verdict else {
        panic!("{}", profile.render());
    };
    assert_eq!(*clause, 0);
    assert_eq!(profile.entries[0].clauses[0].text, "rsi(close, 14) < 0");
}

/// With three clauses the sole-blocker count comes from a different probe (the
/// conjunction of the *other two*), so it is checked on its own.
#[test]
fn three_clauses_name_the_right_one() {
    let text = "\
strategy three on $bound_at_init timeframe 1h
entry long when ema(close, 3) > 0 and ema(close, 5) > 0 and rsi(close, 14) < 0
size fixed 0.01
exit time 24 bars
";
    let (profile, _) = profile_of(text);
    let entry = &profile.entries[0];
    assert_eq!(entry.clauses.len(), 3);
    let Verdict::Blocked { clause, .. } = &entry.verdict else {
        panic!("{}", profile.render());
    };
    assert_eq!(*clause, 2, "{}", profile.render());
    assert!(entry.clauses[0].sole_blocker_share < 0.01);
    assert!(entry.clauses[1].sole_blocker_share < 0.01);
}

/// Two clauses that cannot be true together blame nothing alone: moving either
/// threshold on its own will not make the strategy fire, and the profile says
/// that instead of picking one.
#[test]
fn two_dead_clauses_have_no_single_blocker() {
    let text = "\
strategy two_dead on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 0 and ema(close, 5) < 0
size fixed 0.01
exit time 24 bars
";
    let (profile, _) = profile_of(text);
    assert_eq!(
        profile.entries[0].verdict,
        Verdict::NoSingleBlocker,
        "{}",
        profile.render()
    );
}

/// A strategy that does fire says how often, and the numbers are the preview's
/// own: the conjunction's count in the profile is `bars_true` of the unmodified
/// definition's signal.
#[test]
fn a_firing_entry_reports_the_previews_own_count() {
    let text = "\
strategy fires on $bound_at_init timeframe 1h
entry long when ema(close, 5) > ema(close, 20) and rsi(close, 14) < 70
size fixed 0.01
exit time 24 bars
";
    let (profile, whole) = profile_of(text);
    let entry = &profile.entries[0];
    assert_eq!(entry.bars_all_true as usize, long_true(&whole));
    assert!(entry.bars_all_true > 0);
    assert!(matches!(entry.verdict, Verdict::Fires { .. }));
    assert_eq!(entry.bars_evaluated as usize, whole.bars_evaluated);
}

/// Two entries on one side share a signal name, and the preview reports one
/// count per name. A probe has to silence the sibling or it measures the union.
#[test]
fn an_entry_is_measured_alone_when_a_sibling_feeds_the_same_signal() {
    let two = "\
strategy siblings on $bound_at_init timeframe 1h
entry long when ema(close, 5) > ema(close, 20) and rsi(close, 14) < 70
entry long when rsi(close, 7) < 0 and ema(close, 3) > 0
size fixed 0.01
exit time 24 bars
";
    let (profile, whole) = profile_of(two);
    assert_eq!(profile.entries.len(), 2);
    // The second entry of the pair is the dead one: it must read as dead, not
    // as the first entry's firing.
    let dead = &profile.entries[1];
    assert_eq!(dead.path, "entry.long[1]");
    assert_eq!(dead.bars_all_true, 0, "{}", profile.render());
    // While the pair together does fire, through the first.
    assert!(long_true(&whole) > 0);
    assert!(profile.entries[0].bars_all_true > 0);
}

/// An impossible tally is refused rather than read: a conjunction that was true
/// on more bars than a subset of its own clauses cannot be a count.
#[test]
fn answers_no_window_could_have_produced_are_refused() {
    let def = compiled(SIGN_ERROR);
    let set = &probes(&def).expect("probes")[0];
    // [all, only(0), only(1)] — all-true 9 but the clauses true on 5 and 5.
    assert!(set.counts(100, &[9, 5, 5]).is_err());
    // The wrong number of answers.
    assert!(set.counts(100, &[1]).is_err());
}
