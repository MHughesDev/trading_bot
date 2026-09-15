//! Static invariants added by the external-review sprint
//! (EXTERNAL-REVIEW-2026-09; ADR-P6-01 … ADR-P6-10).
//!
//! Each of these pins a decision that is cheap to reverse by accident and
//! expensive to notice afterwards. They are source scans rather than unit tests
//! because the thing being protected is the *absence* of something: a setting
//! nobody added, a shortcut nobody took. A unit test can only check what exists.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn files(dirs: &[&str], ext: &[&str]) -> Vec<(String, String)> {
    fn walk(dir: &Path, ext: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matches!(name, "target" | "node_modules" | ".git") {
                    walk(&p, ext, out);
                }
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| ext.contains(&x))
            {
                out.push(p);
            }
        }
    }

    let r = root();
    let mut paths = Vec::new();
    for d in dirs {
        walk(&r.join(d), ext, &mut paths);
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(&r)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            std::fs::read_to_string(&p).ok().map(|s| (rel, s))
        })
        .collect()
}

fn rust_files() -> Vec<(String, String)> {
    files(&["crates", "apps"], &["rs"])
}

fn sql_files() -> Vec<(String, String)> {
    files(&["migrations"], &["sql"])
}

/// Line comments blanked, contents preserved.
///
/// Without this every scan here fails on its own explanation: a test that bans
/// `array_length(x, 1) >= 1` has to say so, and saying so writes the banned
/// string into the tree. Blanking rather than deleting keeps line numbers
/// intact, which matters when a failure quotes one.
fn strip_line_comments(src: &str, marker: &str) -> String {
    src.lines()
        .map(|l| match l.find(marker) {
            Some(i) => l[..i].to_string(),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("
")
}

/// Rust sources with comments blanked, and this suite itself excluded — a scan
/// that reads its own source finds every string it bans.
fn rust_code() -> Vec<(String, String)> {
    rust_files()
        .into_iter()
        .filter(|(p, _)| !p.contains("crates/invariants/tests/"))
        .map(|(p, s)| {
            let code = strip_line_comments(&s, "//");
            (p, code)
        })
        .collect()
}

/// Migration DDL with `--` comments blanked.
fn sql_code() -> Vec<(String, String)> {
    sql_files()
        .into_iter()
        .map(|(p, s)| {
            let code = strip_line_comments(&s, "--");
            (p, code)
        })
        .collect()
}

// ── ADR-P6-09: no optimistic intrabar resolution ─────────────────────────────

/// An agent scored on backtests will find any setting that resolves intrabar
/// ambiguity in its favour. The defence is that the setting does not exist.
///
/// `IntrabarPolicy` has exactly two variants — `WorstFirst` and `CloseOnly` —
/// and this fails the build if a third appears, whatever it is called.
#[test]
fn intrabar_policy_has_no_optimistic_variant() {
    let src = rust_code();
    let (path, body) = src
        .iter()
        .find(|(p, _)| p.ends_with("strategy-runtime/src/exits.rs"))
        .expect("the exit tracker must exist");

    let enum_start = body
        .find("pub enum IntrabarPolicy")
        .unwrap_or_else(|| panic!("{path}: IntrabarPolicy is gone"));
    let enum_body = &body[enum_start..enum_start
        + body[enum_start..]
            .find('}')
            .expect("IntrabarPolicy must be a braced enum")];

    for banned in ["Optimistic", "BestFirst", "Favourable", "Favorable", "Configurable"] {
        assert!(
            !enum_body.contains(banned),
            "{path}: `IntrabarPolicy::{banned}` resolves a bar that breached both the stop and \
             the target in the strategy's favour. OHLCV cannot say which came first, so that is \
             not a choice — it is a way to report a win on every such bar. ADR-P6-09."
        );
    }

    let variants = enum_body.matches("    ").count();
    assert!(variants > 0, "{path}: no variants found; the scan is looking at the wrong text");
    assert!(
        enum_body.contains("WorstFirst") && enum_body.contains("CloseOnly"),
        "{path}: both documented variants must still be present"
    );
}

/// `WorstFirst` must remain the default. A default of `CloseOnly` would
/// silently understate every stop hit on the platform.
#[test]
fn the_default_intrabar_policy_is_the_pessimistic_one() {
    let src = rust_files();
    let (_, body) = src
        .iter()
        .find(|(p, _)| p.ends_with("strategy-runtime/src/exits.rs"))
        .expect("the exit tracker must exist");
    // Inside the enum body: the name also appears in the module docs above it,
    // and the doc comment is not where a `#[default]` attribute would be.
    let start = body.find("pub enum IntrabarPolicy").expect("IntrabarPolicy must exist");
    let enum_body = &body[start..start + body[start..].find("
}").expect("braced enum")];
    let idx = enum_body.find("WorstFirst").expect("WorstFirst must be a variant");
    let before = &enum_body[..idx];
    assert!(
        before.contains("#[default]"),
        "WorstFirst must carry #[default]: a default of CloseOnly understates every stop hit \
         on the platform. ADR-P6-09."
    );
}

// ── ADR-P6-07: dollars and dollar tokens are different things ────────────────

/// The literal that started it: three assets, one of them an unsecured claim on
/// a private issuer, summed into a figure labelled USD with no record.
#[test]
fn nothing_matches_usd_against_stablecoins_in_a_literal() {
    for (path, body) in rust_code() {
        if path.ends_with("domain/src/numeraire.rs") {
            continue;
        }
        let flat = body.replace([' ', '\n', '\t'], "");
        for banned in [
            r#""USD"|"USDC"|"USDT""#,
            r#""USD"|"USDT"|"USDC""#,
            r#""USD"|"USDC""#,
            r#""USDC"|"USDT""#,
        ] {
            assert!(
                !flat.contains(banned),
                "{path}: classifying currencies with a literal alternation treats a dollar token \
                 as a dollar. Use `domain::numeraire::Numeraire::of`, which records the peg \
                 assumption instead of hiding it. ADR-P6-07."
            );
        }
    }
}

/// A prefix test would classify `USDe`, `USDD` and a token called `USDScam` as
/// dollar-pegged. Being wrong there is silent, which is the whole problem.
#[test]
fn stablecoins_are_never_identified_by_a_string_prefix() {
    for (path, body) in rust_code() {
        let flat = body.replace([' ', '\n', '\t'], "");
        for banned in [r#"starts_with("USD")"#, r#"starts_with("usd")"#] {
            assert!(
                !flat.contains(banned),
                "{path}: a `USD` prefix is not a peg. Classify through \
                 `domain::numeraire::Numeraire::of`, whose stablecoin list is explicit. ADR-P6-07."
            );
        }
    }
}

// ── ADR-P6-05/06: a kill is a defect, never a threshold ──────────────────────

/// There is no `Underperforms` class and there must never be one. A class named
/// for a performance threshold re-admits "the Sharpe was low" as a kill, which
/// is the degenerate policy the whole mechanism exists to block.
#[test]
fn no_kill_class_names_a_performance_threshold() {
    let src = rust_code();
    let (path, body) = src
        .iter()
        .find(|(p, _)| p.ends_with("ledger/src/falsification.rs"))
        .expect("the falsification types must exist");

    let start = body.find("pub enum KillClass").expect("KillClass must exist");
    let enum_body = &body[start..start + body[start..].find("\n}").expect("braced enum")];

    for banned in [
        "Underperform",
        "LowSharpe",
        "PoorReturn",
        "BelowBenchmark",
        "Unprofitable",
    ] {
        assert!(
            !enum_body.contains(banned),
            "{path}: `KillClass::{banned}` is a result, not a defect. A kill needs a specific \
             reproducible defect a human can check in ten minutes; a threshold re-admits \
             'the Sharpe was low' through the side door. ADR-P6-05."
        );
    }
}

/// The database's scoring function and the Rust type must agree that a verified
/// evaluator defect outranks a promotion. If either drifts, an agent that finds
/// a simulator bug is better off exploiting it.
#[test]
fn the_database_scores_an_evaluator_defect_above_a_promotion() {
    let sql = sql_files();
    let (path, body) = sql
        .iter()
        .find(|(p, _)| p.ends_with("0059_falsification_ledger.sql"))
        .expect("migration 0059 must exist");

    let start = body
        .find("CREATE OR REPLACE FUNCTION mlops.falsification_score")
        .unwrap_or_else(|| panic!("{path}: the scoring function is gone"));
    let fn_body = &body[start..start + body[start..].find("$$ LANGUAGE sql").unwrap_or(600)];

    let defect_score = extract_score(fn_body, "evaluator_defect");
    let kill_score = extract_score(fn_body, "kill");
    assert!(
        defect_score > 2 && kill_score < 2,
        "{path}: falsification_score must put a verified evaluator defect above a promotion (2) \
         and a kill below it; got defect={defect_score}, kill={kill_score}. An agent scored on \
         promotions that discovers a simulator bug will exploit it silently unless reporting it \
         is worth more. ADR-P6-06."
    );
}

fn extract_score(fn_body: &str, kind: &str) -> i32 {
    let marker = format!("p_report_kind = '{kind}' THEN ");
    fn_body
        .find(&marker)
        .and_then(|i| {
            fn_body[i + marker.len()..]
                .split_whitespace()
                .next()
                .and_then(|s| s.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
        })
        .unwrap_or_else(|| panic!("no score found for {kind}"))
}

/// The agent may file a report and may not verify one. A reporter that can mark
/// its own report verified scores whatever it likes.
#[test]
fn the_agent_cannot_verify_its_own_falsification_reports() {
    let sql = sql_files();
    let (path, body) = sql
        .iter()
        .find(|(p, _)| p.ends_with("0059_falsification_ledger.sql"))
        .expect("migration 0059 must exist");

    let grant = body
        .lines()
        .find(|l| l.contains("ON mlops.falsification TO agent_role"))
        .unwrap_or_else(|| panic!("{path}: agent_role has no grant on mlops.falsification"));
    assert!(
        !grant.contains("UPDATE"),
        "{path}: agent_role must not hold UPDATE on mlops.falsification — verification is how a \
         report earns its score, and a reporter that can verify itself makes the ordering in \
         falsification_score decorative. Got: {grant}. ADR-P6-06."
    );
    assert!(grant.contains("INSERT"), "{path}: the agent must still be able to file a report");
}

// ── ADR-P6-04: the authoring store is no longer destructive ──────────────────

/// The write that made `strategy_version` meaningless.
#[test]
fn saving_a_strategy_appends_a_revision_before_it_moves_the_pointer() {
    let src = rust_files();
    let (path, body) = src
        .iter()
        .find(|(p, _)| p.ends_with("api/src/routes/strategies.rs"))
        .expect("the strategy route must exist");

    let rev = body
        .find("INSERT INTO strategy_revision")
        .unwrap_or_else(|| panic!(
            "{path}: saving a strategy must append an immutable revision. Without it the \
             definition a live instance runs is reconstructable from nothing, and two runs \
             sharing a `strategy_version` are only comparable if nobody saved in between. \
             ADR-P6-04."
        ));
    let ptr = body
        .find("INSERT INTO strategy_definitions")
        .unwrap_or_else(|| panic!("{path}: the pointer write is gone"));
    assert!(
        rev < ptr,
        "{path}: the revision must be appended before the pointer moves, or a rolled-back \
         revision leaves a dangling head. ADR-P6-04."
    );
    assert!(
        body.contains("head_revision"),
        "{path}: the pointer write must set head_revision"
    );
}

/// Canonicalisation may sort object keys and must never sort arrays:
/// `exits.rules` and `actions` are evaluated in order, so sorting them would
/// make two behaviourally different strategies hash alike.
#[test]
fn the_canonicaliser_never_sorts_arrays() {
    let src = rust_files();
    let (path, body) = src
        .iter()
        .find(|(p, _)| p.ends_with("domain/src/strategy_def/revision.rs"))
        .expect("the canonicaliser must exist");

    let start = body.find("fn write_canonical").expect("write_canonical must exist");
    let array_arm = body[start..]
        .find("Value::Array(items)")
        .map(|i| &body[start + i..start + i + 400])
        .expect("the array arm must exist");

    assert!(
        !array_arm.contains("sort"),
        "{path}: the array arm of the canonicaliser sorts. `exits.rules` and `actions` are \
         order-dependent, so two strategies that behave differently would hash alike and a \
         `strategy_version` would stop identifying a program. ADR-P6-04."
    );
}

// ── ADR-P6-03: the gate stack still has exactly sixteen gates ────────────────

/// The tradeable frontier is an admission check and not a seventeenth gate. A
/// stack that changes size makes its own pass rate incomparable across
/// versions, which is why the frontier lives before the trial instead.
#[test]
fn the_frontier_check_did_not_become_a_gate() {
    for (path, body) in rust_code() {
        if !path.contains("backtest/src/gates/") {
            continue;
        }
        assert!(
            !body.contains("gate_17") && !body.contains("Gate17"),
            "{path}: a seventeenth gate appeared. The stack is sixteen and does not grow — the \
             tradeable-frontier check is a pre-trial admission rule precisely so the stack size \
             stays fixed. ADR-P6-03."
        );
    }
}

// ── the trap that cost a live database to find ───────────────────────────────

/// `array_length(x, 1)` returns NULL for an empty array and a CHECK evaluating
/// to NULL passes. Already enforced for the original four; re-asserted here so
/// migrations added by this sprint are covered by a test that names the reason.
#[test]
fn no_new_migration_requires_a_non_empty_array_with_array_length() {
    for (path, body) in sql_code() {
        let flat = body.replace([' ', '\n', '\t'], "");
        // 0 for an empty JSON array, never NULL. Only bare Postgres
        // `array_length` has the hole, so a match that is the tail of the jsonb
        // one is skipped rather than reported.
        for (i, _) in flat.to_lowercase().match_indices("array_length(") {
            if flat[..i].to_lowercase().ends_with("jsonb_") {
                continue;
            }
            let window = &flat[i..(i + 60).min(flat.len())];
            assert!(
                !window.contains(">=1") && !window.contains(">0"),
                "{path}: `array_length(x, 1) >= 1` does not require a non-empty array — it                  returns NULL for one, and a CHECK evaluating to NULL *passes*. Use                  `cardinality(x)`. Found: {window}"
            );
        }
    }
}
