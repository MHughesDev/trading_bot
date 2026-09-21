//! The three halves of S-5.10, S-5.11 and S-5.15 have to agree with each other
//! or none of them is worth reading.
//!
//! Each module is tested on its own beside its source. What only an integration
//! test can check is the **join**: the preview decomposes a condition into
//! clauses, the profile names one of them as the blocker, the suggester proposes
//! an edit to *that* clause, and — when someone makes the edit — the differ
//! reports exactly the move that was proposed, at the same address.
//!
//! If any pair of those four drifted, a model would be told to edit
//! `entry.long[0].clause[1]` and then shown a diff about `entry.long[0].when`,
//! and it would have no way to know the two were the same clause. The addressing
//! is the contract, and this file is what holds it.

use strategy_lang::block::{blocking_profile, entry_clauses, ClauseCounts, EntryCounts, Verdict};
use strategy_lang::diff::{diff, lineage, MoveKind, Version};
use strategy_lang::suggest::{suggest_moves, Direction, Evidence, Findings, Proposal};
use strategy_lang::{compile, parse};

use domain::strategy_def::StrategyDefinition;

/// A trend rule with an oversold filter bolted on: the filter is the clause
/// built to be the blocker.
const BASE: &str = "\
strategy joined on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 30
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

fn def(src: &str) -> StrategyDefinition {
    let ast = parse(src).unwrap_or_else(|e| panic!("parse failed for:\n{src}\n{e:#?}"));
    compile(&ast).unwrap_or_else(|e| panic!("compile failed for:\n{src}\n{e:#?}"))
}

/// A window on which the trend clause is usually true, the oversold clause
/// almost never is, and the strategy therefore never fires.
fn never_fired(def: &StrategyDefinition) -> Findings {
    let clauses = entry_clauses(def).expect("clauses");
    let counts = EntryCounts::new(
        clauses[0].path.clone(),
        2_000,
        0,
        vec![
            ClauseCounts {
                path: clauses[0].clauses[0].path.clone(),
                bars_true: 1_300,
                bars_sole_blocker: 690,
            },
            ClauseCounts {
                path: clauses[0].clauses[1].path.clone(),
                bars_true: 12,
                bars_sole_blocker: 1_300,
            },
        ],
    )
    .expect("a possible tally");
    Findings {
        blocking: Some(blocking_profile(def, &[counts]).expect("profile")),
        ..Findings::default()
    }
}

/// The whole chain, in the order a research turn would walk it.
#[test]
fn a_suggestion_acted_on_produces_the_move_it_said_it_would() {
    let base = def(BASE);

    // 1. The preview's decomposition, and the profile over its counts.
    let findings = never_fired(&base);
    let profile = findings.blocking.as_ref().expect("a profile");
    let entry = &profile.entries[0];
    let Verdict::Blocked { clause, .. } = entry.verdict else {
        panic!("expected a sole blocker, got {:?}", entry.verdict);
    };
    assert_eq!(
        entry.clauses[clause].text, "rsi(close, 14) < 30",
        "the fixture was built so that this is the blocker"
    );

    // 2. The suggestion, which must name the clause the profile named.
    let suggestions = suggest_moves(&base, &findings).expect("suggestions");
    let m = suggestions
        .moves
        .iter()
        .find(|m| matches!(m.why, Evidence::SoleBlocker { .. }))
        .expect("a move motivated by the blocking profile");
    assert_eq!(m.kind, MoveKind::ChangeThreshold);
    assert_eq!(
        m.path, entry.clauses[clause].path,
        "the suggestion and the profile must address the same clause"
    );
    let (literal, direction) = match &m.proposal {
        Proposal::MoveThreshold {
            literal, direction, ..
        } => (literal.as_str().to_string(), *direction),
        other => panic!("expected a threshold move, got {other:?}"),
    };
    assert_eq!(literal, "30");
    assert_eq!(direction, Direction::Raise);

    // 3. Somebody acts on it — this crate does not, which is the point.
    let acted = BASE.replace("rsi(close, 14) < 30", "rsi(close, 14) < 45");
    let after = def(&acted);

    // 4. The diff reports exactly that move, at exactly that address.
    let d = diff(&base, &after).expect("diffs");
    assert!(d.is_single_change(), "{:#?}", d.changes());
    let c = &d.changes()[0];
    assert_eq!(c.kind, m.kind);
    assert_eq!(
        c.path,
        m.path.rsplit_once(".clause[").expect("a clause path").0.to_string() + ".when",
        "a condition-level move addresses the condition; the clause it touched is in `before`"
    );
    assert_eq!(c.before.as_deref(), Some("rsi(close, 14) < 30"));
    assert_eq!(c.after.as_deref(), Some("rsi(close, 14) < 45"));
    assert!(c.words.contains("`30` became `45`"), "{}", c.words);
}

/// The same chain when the honest answer is "there is nothing to move": the
/// suggester must not invent a threshold for a clause that compares two
/// features, and the differ must not call the removal something it is not.
#[test]
fn a_clause_with_nothing_to_move_is_dropped_rather_than_tuned() {
    let src = "\
strategy joined on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30 and close > donchian_upper(close, 20)
size fixed 0.01
exit time 48 bars
";
    let base = def(src);
    let clauses = entry_clauses(&base).expect("clauses");
    let counts = EntryCounts::new(
        clauses[0].path.clone(),
        1_000,
        0,
        vec![
            ClauseCounts {
                path: clauses[0].clauses[0].path.clone(),
                bars_true: 300,
                bars_sole_blocker: 80,
            },
            ClauseCounts {
                path: clauses[0].clauses[1].path.clone(),
                bars_true: 5,
                bars_sole_blocker: 900,
            },
        ],
    )
    .expect("a possible tally");
    let findings = Findings {
        blocking: Some(blocking_profile(&base, &[counts]).expect("profile")),
        ..Findings::default()
    };

    let suggestions = suggest_moves(&base, &findings).expect("suggestions");
    let m = &suggestions.moves[0];
    assert_eq!(m.kind, MoveKind::RemoveFilter);
    let dropped = match &m.proposal {
        Proposal::DropClause { clause } => clause.as_str().to_string(),
        other => panic!("expected a dropped clause, got {other:?}"),
    };
    assert_eq!(dropped, "close > donchian_upper(close, 20)");

    let acted = src.replace(" and close > donchian_upper(close, 20)", "");
    let d = diff(&base, &def(&acted)).expect("diffs");
    assert!(d.is_single_change(), "{:#?}", d.changes());
    assert_eq!(d.changes()[0].kind, MoveKind::RemoveFilter);
    assert_eq!(d.changes()[0].before.as_deref(), Some(dropped.as_str()));
}

/// Lineage is the reason the diff exists: a reader must see what was tried, in
/// order, in words, without opening a single JSON document.
#[test]
fn a_lineage_reads_as_the_decisions_that_were_taken() {
    let v2 = BASE.replace("rsi(close, 14) < 30", "rsi(close, 14) < 45");
    let v3 = v2.replace("exit time 48 bars", "exit time 96 bars");
    let v4 = v3.replace("size fixed 0.01", "size risk_per_trade 0.01");

    let l = lineage(&[
        Version {
            label: "v1".into(),
            definition: def(BASE),
        },
        Version {
            label: "v2".into(),
            definition: def(&v2),
        },
        Version {
            label: "v3".into(),
            definition: def(&v3),
        },
        Version {
            label: "v4".into(),
            definition: def(&v4),
        },
    ])
    .expect("lineage");

    assert_eq!(l.steps().len(), 3);
    for step in l.steps() {
        assert!(
            step.diff.is_single_change(),
            "{} → {}: {:#?}",
            step.from,
            step.to,
            step.diff.changes()
        );
    }
    assert!(
        l.multi_change_steps().is_empty(),
        "one change per version, so nothing to flag"
    );

    let rendered = l.render();
    for want in [
        "v1 → v2",
        "`30` became `45`",
        "v2 → v3",
        "v3 → v4",
        "risk_per_trade",
    ] {
        assert!(rendered.contains(want), "missing `{want}` in:\n{rendered}");
    }
    // The whole point: nothing a reader sees is JSON.
    for bad in ["feature('", "\"nodes\"", "/nodes/", "definition_version"] {
        assert!(
            !rendered.contains(bad),
            "the lineage leaked the stored form (`{bad}`):\n{rendered}"
        );
    }
}

/// Every clause address a profile or a suggestion can produce must be one
/// `entry_clauses` produced, and every one of those must sit under the entry
/// path the differ uses. This is the addressing contract stated once.
#[test]
fn every_address_the_three_modules_use_is_the_same_address() {
    let two_sided = "\
strategy joined on $bound_at_init timeframe 1h
input r = rsi(close, 14)
entry long when r < 30 and ema(close, 7) > ema(close, 21)
entry short when r > 70
size fixed 0.01
exit time 48 bars
";
    let d = def(two_sided);
    let clauses = entry_clauses(&d).expect("clauses");
    assert_eq!(clauses.len(), 2, "one entry per side");
    assert_eq!(clauses[0].path, "entry.long[0]");
    assert_eq!(clauses[1].path, "entry.short[0]");

    let mut seen = 0;
    for entry in &clauses {
        for (i, c) in entry.clauses.iter().enumerate() {
            assert_eq!(c.path, format!("{}.clause[{i}]", entry.path));
            seen += 1;
        }
    }
    assert_eq!(seen, 3, "two long clauses and one short one");

    // And the differ addresses the condition the clauses belong to.
    let edited = two_sided.replace("r > 70", "r > 80");
    let diffed = diff(&d, &def(&edited)).expect("diffs");
    assert_eq!(diffed.changes()[0].path, "entry.short[0].when");
    assert!(diffed.changes()[0]
        .path
        .starts_with(&clauses[1].path));
}
