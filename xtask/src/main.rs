//! Dev automation tasks.
//! Usage: cargo xtask <command>
//! Commands:
//!   check-money-f64       — scan workspace for f64 usage on price/size (CI enforced)
//!   lint-no-json-hotpath  — verify serde_json absent from market-lane hot paths (CI enforced)
//!   check-bars-v1-frozen  — verify nothing writes the retired market_bars table (CI enforced)
//!   check-bars-v2-frozen  — verify nothing new writes the frozen market_bars_v2 (CI enforced)

use std::process::Command;

fn main() {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-money-f64") => check_money_f64(),
        Some("lint-no-json-hotpath") => lint_no_json_hotpath(),
        Some("check-bars-v1-frozen") => check_bars_v1_frozen(),
        Some("check-bars-v2-frozen") => check_bars_v2_frozen(),
        Some(t) => {
            eprintln!("Unknown xtask: {t}");
            std::process::exit(1);
        }
        None => {
            println!(
                "Usage: cargo xtask <check-money-f64|lint-no-json-hotpath|check-bars-v1-frozen|check-bars-v2-frozen>"
            );
        }
    }
}

/// Scan the workspace for f64/f32 usage near price/size/quantity/amount field
/// names or type annotations.  Any match is a CI failure — monetary values must
/// use rust_decimal::Decimal.
///
/// Skips:
/// - Lines inside `#[cfg(test)]` modules (depth-tracked brace counting)
/// - Files under `*/tests/*` directories
fn check_money_f64() {
    let money_keywords = [
        "price",
        "size",
        "quantity",
        "amount",
        "pnl",
        "commission",
        "fee",
    ];

    let output = Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .output()
        .expect("git ls-files failed");

    if !output.status.success() {
        eprintln!("git ls-files failed");
        std::process::exit(2);
    }

    let file_list = String::from_utf8(output.stdout).expect("invalid utf8 from git ls-files");
    let files: Vec<&str> = file_list.lines().collect();

    let mut violations: Vec<String> = vec![];

    for file in &files {
        if file.contains("target/") {
            continue;
        }
        // Skip dedicated test directories (e.g. crates/*/tests/).
        if file.contains("/tests/") {
            continue;
        }

        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Track whether we are inside a #[cfg(test)] block by counting braces.
        let mut in_test_depth: i32 = 0;
        let mut brace_depth: i32 = 0;

        for (lineno, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            // Detect entry into a cfg(test) block.
            if trimmed.contains("#[cfg(test)]") {
                in_test_depth = brace_depth + 1; // set sentinel one level deeper
            }

            // Count braces so we know when we exit the test module.
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        // Once brace depth falls below the test sentinel, we're out.
                        if in_test_depth > 0 && brace_depth < in_test_depth {
                            in_test_depth = 0;
                        }
                    }
                    _ => {}
                }
            }

            // Skip lines inside test modules.
            if in_test_depth > 0 {
                continue;
            }

            // Skip comment-only lines.
            if trimmed.starts_with("//") {
                continue;
            }

            let lower = line.to_lowercase();

            let has_float = lower.contains(": f64")
                || lower.contains(": f32")
                || lower.contains("-> f64")
                || lower.contains("-> f32")
                || lower.contains(" f64 ")
                || lower.contains(" f32 ")
                || lower.contains("(f64)")
                || lower.contains("(f32)")
                || lower.contains("<f64>")
                || lower.contains("<f32>")
                || lower.contains(" as f64")
                || lower.contains(" as f32");

            if !has_float {
                continue;
            }

            // Match lowercase keywords only — this avoids false positives from
            // Decimal wrapper types like `Price`, `Size` that appear in return
            // type annotations (`-> Result<Price, ...>`).
            //
            // Special-case "size": exclude occurrences that only appear as part
            // of "usize" or "isize" (e.g. `w: usize`) to avoid false positives
            // in statistical helper functions.
            let has_money_keyword = money_keywords.iter().any(|kw| {
                if *kw == "size" {
                    // Strip "usize" and "isize" from the line before checking.
                    let stripped = line.replace("usize", "____").replace("isize", "____");
                    stripped.contains("size")
                } else {
                    line.contains(kw)
                }
            });
            // Named exemptions, each with the reason it is not money.
            //
            // A list, not a widened keyword: dropping "size" or "quantity" from
            // the keywords would stop the scan catching an actual `size: f64` on
            // an order, which is the thing it exists for. Every entry here is a
            // line the heuristic reads as monetary and is not, and each has to be
            // written down to be exempt — so a future real violation cannot hide
            // among them, and a stale entry is visible as a line nobody can
            // justify any more.
            const NOT_MONEY: &[(&str, u32, &str)] = &[
                (
                    "crates/harness/src/eval/holdout.rs",
                    138,
                    "`corpus_size` is a count of eval cases; the scan matches `size`",
                ),
                (
                    "crates/harness/src/memory/mod.rs",
                    260,
                    "`quantity` here is a String naming what was measured (a Sharpe,                      a count); `value` is that measurement, not an amount of money",
                ),
            ];
            let exempt = NOT_MONEY
                .iter()
                .any(|(f, l, _)| file.replace('\\', "/") == *f && lineno + 1 == *l as usize);

            if has_money_keyword && !exempt {
                violations.push(format!(
                    "{}:{}: f64/f32 on monetary field — use Decimal\n  {}",
                    file,
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }

    if violations.is_empty() {
        println!("check-money-f64: OK — no f64/f32 on monetary fields found");
    } else {
        eprintln!(
            "check-money-f64: FAILED — {} violation(s):",
            violations.len()
        );
        for v in &violations {
            eprintln!("  {v}");
        }
        std::process::exit(1);
    }
}

/// Scan market-lane hot-path crates for `serde_json` imports.
///
/// `serde_json` is only allowed in:
/// - `crates/api/` (REST API boundary)
/// - `crates/event-bus/src/quarantine.rs` (quarantine lane, intentional)
/// - `crates/storage/` (Parquet archive writer)
/// - `crates/domain/` (serde test helpers, not on hot path)
/// - `*/tests/*` directories
/// - `#[cfg(test)]` blocks
///
/// Any other occurrence in the hot-path crates is a CI failure.
fn lint_no_json_hotpath() {
    // Only crates that encode/decode EventEnvelope payloads on the market-data
    // hot path.  Execution and risk crates talk to REST APIs and legitimately
    // use serde_json to parse venue responses.
    let hot_path_crates = [
        "crates/collectors",
        "crates/event-bus",
        "crates/strategy-runtime",
        "crates/features",
    ];

    let output = Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .output()
        .expect("git ls-files failed");

    if !output.status.success() {
        eprintln!("git ls-files failed");
        std::process::exit(2);
    }

    let file_list = String::from_utf8(output.stdout).expect("invalid utf8 from git ls-files");
    let files: Vec<&str> = file_list.lines().collect();

    let mut violations: Vec<String> = vec![];

    for file in &files {
        if file.contains("target/") || file.contains("/tests/") {
            continue;
        }

        // Only check hot-path crates.
        let in_hotpath = hot_path_crates
            .iter()
            .any(|prefix| file.starts_with(prefix));
        if !in_hotpath {
            continue;
        }

        // Quarantine publisher is intentionally JSON — skip it.
        if file.ends_with("quarantine.rs") {
            continue;
        }

        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Track #[cfg(test)] blocks by brace depth.
        let mut in_test_depth: i32 = 0;
        let mut brace_depth: i32 = 0;

        for (lineno, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            if trimmed.contains("#[cfg(test)]") {
                in_test_depth = brace_depth + 1;
            }

            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        if in_test_depth > 0 && brace_depth < in_test_depth {
                            in_test_depth = 0;
                        }
                    }
                    _ => {}
                }
            }

            if in_test_depth > 0 || trimmed.starts_with("//") {
                continue;
            }

            if line.contains("serde_json") {
                // Allow venue API message construction (subscribe/auth JSON bodies).
                // These are at the WS protocol boundary, not payload encoding.
                if line.contains("serde_json::json!")
                    || line.contains("serde_json::from_str")
                    || line.contains("serde_json::from_slice::<")
                {
                    // Parsing raw bytes from external APIs is allowed.
                    let is_raw_parse = line.contains("&raw")
                        || line.contains("&text")
                        || line.contains("&body")
                        || line.contains("&buf");
                    if is_raw_parse
                        || line.contains("serde_json::json!")
                        || line.contains("AlpacaMessage")
                        || line.contains("KrakenMessage")
                        || line.contains("ChartResponse")
                        || line.contains("CandlesResponse")
                        || line.contains("ZeroXQuoteResponse")
                        || line.contains("MarketResponse")
                        || line.contains("QuotesResponse")
                        || line.contains("RedditListing")
                    {
                        continue;
                    }
                }

                violations.push(format!(
                    "{}:{}: serde_json in hot-path crate — use rkyv\n  {}",
                    file,
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }

    if violations.is_empty() {
        println!("lint-no-json-hotpath: OK — no serde_json in market-lane hot paths");
    } else {
        eprintln!(
            "lint-no-json-hotpath: FAILED — {} violation(s):",
            violations.len()
        );
        for v in &violations {
            eprintln!("  {v}");
        }
        std::process::exit(1);
    }
}

/// Verify that nothing writes the retired `market_bars` table (Set L, L-0.7).
///
/// `market_bars` is `ReplacingMergeTree(revision) ORDER BY (instrument_id,
/// available_time)` with `timeframe` missing from the sorting key, so two bars of
/// different timeframes closing at the same instant destroy one another on merge —
/// and, when they arrive in the same insert batch, immediately (DATA-005 §3).
/// Writers moved to `market_bars_v2` on 2026-09-11 and the table is frozen.
///
/// This is a source guard rather than a database permission because the ClickHouse
/// user is defined in `users_xml`, which is read-only to SQL: `REVOKE INSERT` fails
/// with ACCESS_STORAGE_READONLY, and a grant would not survive a container rebuild
/// anyway. A CI check does survive, and it fails at the moment someone reintroduces
/// the write rather than months later when a merge eats the bars.
///
/// Reads of `market_bars` are still permitted — the table is kept as a fallback
/// until it is retired at the end of Set L. Only inserts are forbidden.
fn check_bars_v1_frozen() {
    let output = Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .output()
        .expect("git ls-files failed");

    if !output.status.success() {
        eprintln!("git ls-files failed");
        std::process::exit(2);
    }

    let file_list = String::from_utf8(output.stdout).expect("invalid utf8 from git ls-files");
    let mut violations: Vec<String> = vec![];

    for file in file_list.lines() {
        if file.contains("target/") {
            continue;
        }
        // Tests may still write v1 deliberately — `bars_v2_writer.rs` seeds it in
        // order to prove it destroys a bar, which is the premise of the whole fix.
        if file.contains("/tests/") {
            continue;
        }
        // The guard describes the forbidden pattern, so it necessarily contains it.
        if file.ends_with("xtask/src/main.rs") {
            continue;
        }

        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                continue;
            }
            // An insert targeting v1: `.insert("market_bars")`, or SQL that writes it.
            let writes_v1 = line.contains("insert(\"market_bars\")")
                || line.contains("INSERT INTO market_bars ")
                || line.contains("INSERT INTO market_bars(")
                || line.trim_end().ends_with("INSERT INTO market_bars");
            if writes_v1 {
                violations.push(format!("{}:{}: {}", file, idx + 1, line.trim()));
            }
        }
    }

    if violations.is_empty() {
        println!("check-bars-v1-frozen: OK — nothing writes the retired market_bars table");
        return;
    }

    eprintln!(
        "check-bars-v1-frozen: FAILED — {} write(s) to the retired market_bars table:",
        violations.len()
    );
    for v in &violations {
        eprintln!("  {v}");
    }
    eprintln!();
    eprintln!("market_bars collapses bars of different timeframes that close at the same");
    eprintln!("instant (DATA-005 §3). Write the canonical `market_bar` instead — not");
    eprintln!("market_bars_v2, which is itself frozen (ADR-D-29, check-bars-v2-frozen).");
    std::process::exit(1);
}

/// Writers of `market_bars_v2` that predate the freeze, with the number of write
/// sites each file still holds.
///
/// These four are the whole of the pre-freeze debt, measured on 2026-09-21. They
/// live outside the dataplane and were left in place deliberately: every one of
/// them is inert against the live database (the `market_bars` → v2 backfill has
/// zero rows left to copy, and the v2 → `market_bar` migration recorded all 17 of
/// its chunks as copied at 21:19:28 that day), so removing them is a cleanup for
/// the crate's owner rather than a correctness fix this check should force.
///
/// The count is pinned in both directions on purpose. A new write site in one of
/// these files fails the check, and so does *removing* one without shrinking the
/// entry — which is what makes the allowlist retire itself instead of becoming a
/// permanent licence.
const V2_PRE_FREEZE_WRITERS: &[(&str, usize)] = &[
    // `INSERT INTO market_bars_v2` — the one real insert.
    ("crates/storage/src/clickhouse/backfill.rs", 1),
    // `EXCHANGE TABLES market_bars_v2 AND <staging>` — replaces the table wholesale.
    ("crates/storage/src/clickhouse/migrate.rs", 1),
    // Calls the backfill: the standalone operator example, and platform boot.
    ("crates/storage/examples/backfill_bars_v2.rs", 1),
    ("apps/platform/src/main.rs", 1),
];

/// Verify that no new code writes the frozen `market_bars_v2` table (ADR-D-29).
///
/// `market_bars_v2` is `ReplacingMergeTree(ingested_time) ORDER BY (instrument_id,
/// timeframe, venue_id, source, event_time, revision)`. `available_time` — the
/// column that carries when a bar became knowable — is **not in that sorting key**.
/// So two rows for the same bar and the same `revision` that differ only in when
/// they became known collapse on a background merge, and the one kept is whichever
/// arrived later by `ingested_time`. A restatement is destroyed with no error, no
/// surviving row and nothing to read afterwards: a non-PIT read path that nobody
/// chose, inherited from a sort key. The canonical `market_bar` carries
/// `knowledge_time` inside its sorting key and does not have this failure.
///
/// The hazard is not theoretical. On 2026-09-21 the live table held 1,441 sorting-key
/// groups with two physical rows each, differing only in `ingested_time` — 1,441 rows
/// queued for silent destruction on the next merge. They happened to agree on price
/// and on `available_time`, so nothing of value was lost *that* time; the mechanism
/// that would lose it was fully armed.
///
/// Authority went to `market_bar` on the evidence: it was ingesting minutes ago
/// while v2's newest row was a week old, it holds every bar v2 holds (an anti-join
/// over all nine series returned zero rows unique to v2, and all 156,817 shared bars
/// agree on OHLCV to 1e-6), and it holds 13,747 BTC-USD/coinbase 1m bars besides.
///
/// A source guard rather than a `REVOKE INSERT`, for the same reason
/// [`check_bars_v1_frozen`] is one: the ClickHouse user is defined in `users_xml`,
/// which SQL cannot modify, and a grant would not survive a container rebuild.
fn check_bars_v2_frozen() {
    let output = Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .output()
        .expect("git ls-files failed");

    if !output.status.success() {
        eprintln!("git ls-files failed");
        std::process::exit(2);
    }

    let file_list = String::from_utf8(output.stdout).expect("invalid utf8 from git ls-files");
    let mut violations: Vec<String> = vec![];
    let mut allowed_seen: Vec<(&str, usize)> =
        V2_PRE_FREEZE_WRITERS.iter().map(|(p, _)| (*p, 0)).collect();

    for file in file_list.lines() {
        if file.contains("target/") {
            continue;
        }
        // Tests write v2 deliberately, to demonstrate the collapse that is the
        // premise of this whole rule. A test that could not write the broken table
        // could not prove it is broken.
        if file.contains("/tests/") {
            continue;
        }
        // The guard names the forbidden patterns, so it necessarily contains them.
        if file.ends_with("xtask/src/main.rs") {
            continue;
        }

        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (idx, line) in content.lines().enumerate() {
            if !line_writes_v2(line) {
                continue;
            }
            match allowed_seen.iter_mut().find(|(p, _)| file.ends_with(p)) {
                Some((_, n)) => *n += 1,
                None => violations.push(format!("{}:{}: {}", file, idx + 1, line.trim())),
            }
        }
    }

    let mut drift: Vec<String> = vec![];
    for (path, expected) in V2_PRE_FREEZE_WRITERS {
        let found = allowed_seen
            .iter()
            .find(|(p, _)| p == path)
            .map_or(0, |(_, n)| *n);
        if found != *expected {
            drift.push(format!(
                "  {path}: allowlist expects {expected} write site(s), found {found}"
            ));
        }
    }

    if violations.is_empty() && drift.is_empty() {
        println!(
            "check-bars-v2-frozen: OK — no new writes to the frozen market_bars_v2 table \
             ({} pre-freeze write site(s) accounted for)",
            V2_PRE_FREEZE_WRITERS.iter().map(|(_, n)| n).sum::<usize>()
        );
        return;
    }

    if !violations.is_empty() {
        eprintln!(
            "check-bars-v2-frozen: FAILED — {} write(s) to the frozen market_bars_v2 table:",
            violations.len()
        );
        for v in &violations {
            eprintln!("  {v}");
        }
        eprintln!();
        eprintln!("market_bars_v2 has no `available_time` in its sorting key, so a restatement");
        eprintln!("of an existing bar is destroyed on the next merge with no error and no");
        eprintln!("surviving row (ADR-D-29). Write the canonical `market_bar` instead, via");
        eprintln!("`backtest::store::BarStore` — it keys on `knowledge_time` and keeps both.");
    }

    if !drift.is_empty() {
        eprintln!();
        eprintln!("check-bars-v2-frozen: FAILED — V2_PRE_FREEZE_WRITERS is out of date:");
        for d in &drift {
            eprintln!("{d}");
        }
        eprintln!();
        eprintln!("If you removed a write site, shrink its entry (and delete the entry at 0).");
        eprintln!("If you added one, it is a new write to a frozen table — remove it instead.");
    }

    std::process::exit(1);
}

/// True when this line writes `market_bars_v2`, in any of the forms the workspace
/// actually uses to do it.
///
/// `trading.market_bars_v2` is matched as well as the bare name. The database-
/// qualified spelling is not hypothetical: it is how `qc_census.rs` read the table
/// straight past AT-01 in `crates/invariants`, whose regex expects the table name to
/// follow `FROM` immediately.
fn line_writes_v2(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
        return false;
    }
    for name in ["market_bars_v2", "trading.market_bars_v2"] {
        // A direct insert, in SQL or through the client builder.
        if line.contains(&format!("INSERT INTO {name} "))
            || line.contains(&format!("INSERT INTO {name}("))
            || line.trim_end().ends_with(&format!("INSERT INTO {name}"))
            || line.contains(&format!("insert(\"{name}\")"))
        {
            return true;
        }
        // Swapping another table into v2's name replaces every row it holds.
        if line.contains(&format!("EXCHANGE TABLES {name} "))
            || line.contains(&format!("RENAME TABLE {name} "))
        {
            return true;
        }
    }
    // Calling the backfill writes v2 just as surely as the INSERT inside it does.
    // The definition itself is not a call.
    if line.contains("backfill_market_bars_v2(") && !line.contains("fn backfill_market_bars_v2(") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::line_writes_v2;

    #[test]
    fn plain_inserts_are_writes() {
        assert!(line_writes_v2(r#"    .query("INSERT INTO market_bars_v2 (a) VALUES (1)")"#));
        assert!(line_writes_v2(r#"    .query("INSERT INTO market_bars_v2(a) VALUES (1)")"#));
        assert!(line_writes_v2(r#"    .insert("market_bars_v2")"#));
    }

    /// The hole this check exists to close. `crates/invariants`' AT-01 expects the
    /// table name to follow `FROM`/`INTO` immediately, so database-qualifying it
    /// walks straight past — which is how the QC census read v2 unnoticed.
    #[test]
    fn database_qualified_writes_are_writes() {
        assert!(line_writes_v2(
            r#"    .query("INSERT INTO trading.market_bars_v2 (a) VALUES (1)")"#
        ));
        assert!(line_writes_v2(r#"    .insert("trading.market_bars_v2")"#));
    }

    /// Swapping a staging table into v2's name replaces every row it holds, which is
    /// a write however little it looks like one.
    #[test]
    fn table_swaps_are_writes() {
        assert!(line_writes_v2(
            r#"    .query(&format!("EXCHANGE TABLES market_bars_v2 AND {staging}"))"#
        ));
        assert!(line_writes_v2(
            r#"    .query(&format!("RENAME TABLE market_bars_v2 TO {archive}"))"#
        ));
    }

    #[test]
    fn calling_the_backfill_is_a_write_but_defining_it_is_not() {
        assert!(line_writes_v2(
            "    let r = storage::clickhouse::backfill::backfill_market_bars_v2(&url, &src)"
        ));
        assert!(!line_writes_v2("pub async fn backfill_market_bars_v2("));
    }

    /// Reads stay legal: the v2 → `market_bar` migration has to read the table it is
    /// draining, and a check that forbade that would forbid the fix.
    #[test]
    fn reads_and_prose_are_not_writes() {
        assert!(!line_writes_v2(r#"    .query("SELECT count() FROM market_bars_v2")"#));
        assert!(!line_writes_v2(
            r#"    .query("SELECT * FROM trading.market_bars_v2 FINAL")"#
        ));
        assert!(!line_writes_v2(r#"    .query("INSERT INTO market_bar (a) VALUES (1)")"#));
        assert!(!line_writes_v2("// INSERT INTO market_bars_v2 is forbidden"));
        assert!(!line_writes_v2("//! writes its bars to `market_bars_v2` with"));
        assert!(!line_writes_v2(" * INSERT INTO market_bars_v2"));
    }
}
