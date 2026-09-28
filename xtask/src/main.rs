//! Dev automation tasks.
//! Usage: cargo xtask <command>
//! Commands:
//!   check-money-f64       — scan workspace for f64 usage on price/size (CI enforced)
//!   lint-no-json-hotpath  — verify serde_json absent from market-lane hot paths (CI enforced)
//!   check-bars-v1-frozen  — verify nothing writes the retired market_bars table (CI enforced)
//!   check-bars-v2-frozen  — verify nothing new writes the frozen market_bars_v2 (CI enforced)
//!   check-free-sources    — verify every declared data source is free, or waived by an ADR (CI enforced)

use std::process::Command;

fn main() {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-money-f64") => check_money_f64(),
        Some("lint-no-json-hotpath") => lint_no_json_hotpath(),
        Some("check-bars-v1-frozen") => check_bars_v1_frozen(),
        Some("check-bars-v2-frozen") => check_bars_v2_frozen(),
        Some("check-free-sources") => check_free_sources(),
        Some(t) => {
            eprintln!("Unknown xtask: {t}");
            std::process::exit(1);
        }
        None => {
            println!(
                "Usage: cargo xtask <check-money-f64|lint-no-json-hotpath|check-bars-v1-frozen|check-bars-v2-frozen|check-free-sources>"
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
/// These four are the whole of the pre-freeze debt. They live outside the dataplane
/// and are left in place deliberately, on two measurements taken against the live
/// database on 2026-09-22:
///
/// - the `market_bars` → v2 backfill has nothing left to copy — a fingerprint
///   anti-join of all 99,789 v1 rows against v2 returns 0 missing;
/// - the v2 → `market_bar` migration is complete — `canonical_migration_chunk`
///   records all 17 `(timeframe, month)` chunks copied at 2026-09-21 21:19:28,
///   156,817 rows, matching v2's 156,817 distinct `event_id`s exactly.
///
/// So all four are inert, and removing them is a cleanup for the crate's owner
/// rather than a correctness fix this check should force. Deleting them is in fact
/// the *riskier* move: `market_bars` → v2 → `market_bar` is the only path by which
/// a box still holding v1-era bars reaches the canonical table, and a freeze that
/// severs it would strand that data rather than protect it.
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
/// The hazard is not theoretical. Measured on the live table on 2026-09-22, v2 held
/// 158,258 physical rows over 156,817 distinct `event_id`s: 1,441 sorting-key groups
/// with two rows each — 1,441 rows queued for silent destruction on the next merge.
/// Column by column, those pairs agree on OHLCV, on `trade_count`, on `available_time`,
/// on `event_id` and on `sequence`; they differ in `ingested_time` **and in
/// `snapshot_id`** (1,441 groups apiece). So no price and no knowledge time would be
/// lost on that merge, but the record of *which ingest batch delivered the row* would
/// be, for every one of the 1,441. An earlier note here said the pairs differ only in
/// `ingested_time`; that was one column short, and the missing column is the
/// provenance one. The canonical table has no equivalent hazard: `market_bar` holds
/// zero groups sharing its full sorting key.
///
/// It may already have fired once. Six files in this workspace record v2 as holding
/// 158,259 rows on 2026-09-15; it holds 158,258 now, and `system.part_log` shows a
/// `MergeParts` on `trading.market_bars_v2` at 2026-09-16 11:22:03, between the two
/// readings. That is circumstantial — `part_log` does not say which row a merge
/// dropped — but the arithmetic and the timing agree, and 1,441 more are queued
/// behind it. Those files are not edited to say 158,258: each is a dated statement
/// that was true when it was written, and the *change* between the two readings is
/// the finding, not a typo to paper over.
///
/// Authority went to `market_bar` on the evidence, all of it re-measured 2026-09-22:
///
/// - **Freshness.** `market_bar`'s newest event is 2026-09-22 02:05, ingested 02:06.
///   v2's newest is 2026-09-14 01:00 — eight days stale, and nothing has written it
///   since. Read honestly, this is a gap of eight days between the two tables and
///   *not* a live feed against a dead one: re-checked at 19:19 on 2026-09-22,
///   `market_bar`'s own writer had also been silent for seventeen hours (its last
///   hourly buckets run 2026-09-21 21:00 through 2026-09-22 02:00, then stop). An
///   earlier reading described `market_bar` as ingesting "minutes ago", which was
///   true when it was taken. Authority rests on the eight-day gap and on containment
///   below, neither of which a paused collector changes.
/// - **Containment.** An anti-join of every v2 bar (keyed on instrument, venue,
///   period and `bar_open_time`, which is v2's open and `market_bar`'s `event_time`)
///   against `market_bar` returns **zero** rows unique to v2, across all nine series
///   and all 158,258 rows — every one of which resolves through the identity dims,
///   so nothing was dropped by the join.
/// - **Surplus.** `market_bar` holds 170,924 rows over 170,851 distinct bars to v2's
///   158,258 rows over 156,817 — 14,034 distinct bars the frozen table does not have.
///   On BTC-USD/coinbase/1m alone it holds 105,760 rows against v2's 93,453 physical
///   (92,013 distinct): 12,307 more rows, 13,747 more distinct bars.
/// - **Agreement.** On the 156,817 bars both tables carry, a point-in-time resolution
///   of each side (`argMax` on `ingested_time` for v2, on `(knowledge_time,
///   revision_seq)` for `market_bar`) agrees on open, high, low, close *and* volume
///   to 1e-6 on every one. Zero disagreements, so the surplus is strictly additional
///   and not a second opinion.
///
/// # Freezing v2 strands nothing
///
/// The one way a freeze could destroy data is by severing `market_bars` → v2 →
/// `market_bar` while v1 still held bars that had not reached the canonical table.
/// Checked against the destination directly on 2026-09-22 rather than inferred from
/// the middle of the chain: all 99,789 rows of `market_bars` resolve through the
/// identity dimensions, and anti-joined against `market_bar` on (instrument, venue,
/// period, open time — v1 stamps the close, so the period is subtracted) **zero** are
/// missing. The chain has nothing left to carry in either of its two hops.
///
/// # What only v2 holds — why it is frozen and not dropped
///
/// Not one bar: the anti-join above is zero in that direction too. What v2 still
/// holds alone is provenance, in four columns `market_bar` has no home for, counted
/// on 2026-09-22:
///
/// - `event_id` / `dedup_key` — 156,817 distinct apiece, one per logical bar: the
///   identity under which those bars were first ingested.
/// - `snapshot_id` — 10,521 distinct: which ingest batch delivered each row.
/// - `sequence` — 112,769 distinct.
/// - `lane` (2 values) and `trust_tier` (2) are derivable from timeframe and venue
///   and hold nothing extra.
///
/// So dropping v2 would cost the ingest-batch and event-identity history of the
/// platform's first 156,817 bars and no market data whatsoever. That is a real cost,
/// small, and not this check's to spend — hence frozen, not dropped, and no migration
/// written here.
///
/// One more thing worth stating because it bounds the damage: `available_time`, the
/// column whose absence from the sorting key *is* this hazard, equals `event_time` on
/// **all 158,258 rows** — every bar v2 holds was stamped as knowable at its own close.
/// The column is degenerate in the data actually present. The mechanism is armed and
/// the guard is warranted, but no knowledge time is currently distinguishable enough
/// for a merge to destroy. This is a measurement, not an assumption, and it is a
/// statement about today's rows only: the mechanism would destroy a *future*
/// restatement, which is exactly why nothing may write here again.
///
/// A source guard rather than a `REVOKE INSERT`, for the same reason
/// [`check_bars_v1_frozen`] is one: the ClickHouse user is defined in `users_xml`,
/// which SQL cannot modify, and a grant would not survive a container rebuild.
///
/// # Scope
///
/// Rust sources only, like [`check_bars_v1_frozen`] — `clickhouse/*.sql` still
/// carries v2's `CREATE TABLE`, which must stay so the table remains readable.
fn check_bars_v2_frozen() {
    let (violations, drift) = match scan_v2_writers(None) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("check-bars-v2-frozen: {e}");
            std::process::exit(2);
        }
    };

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

/// Walk the workspace's Rust sources and classify every line that writes
/// `market_bars_v2`.
///
/// Returns `(violations, drift)`: write sites in files the allowlist does not name,
/// and files whose site count no longer matches [`V2_PRE_FREEZE_WRITERS`].
///
/// `root` is the repository root. `None` means the current directory, which is what
/// `cargo xtask` gets; the self-test passes an explicit path so it does not depend
/// on where the harness happens to be standing.
fn scan_v2_writers(root: Option<&std::path::Path>) -> Result<(Vec<String>, Vec<String>), String> {
    let mut git = Command::new("git");
    if let Some(r) = root {
        git.arg("-C").arg(r);
    }
    let output = git
        .args(["ls-files", "--full-name", "--", "*.rs"])
        .output()
        .map_err(|e| format!("git ls-files failed: {e}"))?;

    if !output.status.success() {
        return Err("git ls-files failed".into());
    }

    let file_list = String::from_utf8(output.stdout)
        .map_err(|_| "invalid utf8 from git ls-files".to_string())?;
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

        let on_disk = root.map_or_else(
            || std::path::PathBuf::from(file),
            |r| r.join(file.replace('/', std::path::MAIN_SEPARATOR_STR)),
        );
        let content = match std::fs::read_to_string(&on_disk) {
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

    Ok((violations, drift))
}

/// The two spellings of the frozen table. `trading.market_bars_v2` is not
/// hypothetical: the database-qualified form is how `qc_census.rs` read the table
/// straight past AT-01 in `crates/invariants`, whose regex expects the table name to
/// follow `FROM`/`INTO` immediately.
const V2_TABLE_NAMES: [&str; 2] = ["market_bars_v2", "trading.market_bars_v2"];

/// SQL verbs that change what `market_bars_v2` contains, each matched as
/// `<verb> <table>`.
///
/// `INSERT INTO` is the obvious one. The rest are here because a freeze that only
/// stops inserts is not a freeze:
///
/// - `OPTIMIZE TABLE` is the single most dangerous line anyone could write against
///   this table. The 1,441 doomed rows die on *a merge*; `OPTIMIZE ... FINAL` is how
///   you ask for that merge on demand. It reads like maintenance and it is the
///   trigger.
/// - `ALTER TABLE ... DELETE|UPDATE` mutates rows in place.
/// - `DROP`/`TRUNCATE`/`DETACH` remove them outright.
/// - `EXCHANGE TABLES`/`RENAME TABLE`/`ATTACH TABLE` substitute a different table's
///   contents under v2's name, which replaces every row it holds.
const V2_WRITE_VERBS: &[&str] = &[
    "INSERT INTO",
    "OPTIMIZE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "DROP TABLE IF EXISTS",
    "TRUNCATE TABLE",
    "TRUNCATE TABLE IF EXISTS",
    "EXCHANGE TABLES",
    "RENAME TABLE",
    "ATTACH TABLE",
    "DETACH TABLE",
];

/// True when `hay` (already lowercased) contains `needle` as a whole SQL identifier —
/// not followed by a character that could continue one.
///
/// The boundary is load-bearing rather than tidy. `migrate.rs` names its staging and
/// archive tables `market_bars_v2_rebuild_<ts>` and `market_bars_v2_premerge_<ts>`,
/// and the live database holds a `market_bars_v2_premerge_20260911184559`. Writing
/// *those* is how the rebuild preserved the data; a prefix match would call it a
/// violation and make the check cry wolf at the one operation that saved the rows.
fn mentions_identifier(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let end = from + rel + needle.len();
        let continues = hay[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if !continues {
            return true;
        }
        from += rel + needle.len();
    }
    false
}

/// True when this line writes `market_bars_v2`, in any of the forms the workspace
/// actually uses to do it.
///
/// Reads stay legal and must: the v2 → `market_bar` migration has to read the table
/// it is draining, and a check that forbade that would forbid the fix.
fn line_writes_v2(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
        return false;
    }
    // Keywords are matched case-insensitively. The workspace spells its SQL in
    // upper case today, and a guard that a lower-case `insert into` walks past is
    // a guard that documents a rule instead of enforcing one.
    let lower = line.to_ascii_lowercase();
    for name in V2_TABLE_NAMES {
        // The client builder's typed form: `.insert("market_bars_v2")`.
        if lower.contains(&format!("insert(\"{name}\")")) {
            return true;
        }
        for verb in V2_WRITE_VERBS {
            let pat = format!("{} {name}", verb.to_ascii_lowercase());
            if mentions_identifier(&lower, &pat) {
                return true;
            }
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
        assert!(line_writes_v2(
            r#"    .query("INSERT INTO market_bars_v2 (a) VALUES (1)")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("INSERT INTO market_bars_v2(a) VALUES (1)")"#
        ));
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

    /// `OPTIMIZE ... FINAL` is the line that actually kills the bars. The 1,441
    /// doomed rows in v2 die *on a merge*; this is how you ask for one on demand. A
    /// freeze that stopped inserts and let this through would stop nothing.
    #[test]
    fn forcing_a_merge_is_a_write() {
        assert!(line_writes_v2(
            r#"    .query("OPTIMIZE TABLE market_bars_v2 FINAL")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("OPTIMIZE TABLE trading.market_bars_v2 FINAL")"#
        ));
    }

    /// Mutating and removing rows count too — a frozen table that anyone may
    /// `ALTER ... DELETE` is not frozen.
    #[test]
    fn mutations_and_removals_are_writes() {
        assert!(line_writes_v2(
            r#"    .query("ALTER TABLE market_bars_v2 DELETE WHERE timeframe = '1h'")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("ALTER TABLE market_bars_v2 UPDATE revision = 1 WHERE 1")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("TRUNCATE TABLE market_bars_v2")"#
        ));
        assert!(line_writes_v2(r#"    .query("DROP TABLE market_bars_v2")"#));
        assert!(line_writes_v2(
            r#"    .query("DROP TABLE IF EXISTS market_bars_v2")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("DETACH TABLE market_bars_v2")"#
        ));
    }

    /// Lower-case SQL is the same SQL. A guard that only knows one casing documents
    /// the rule rather than enforcing it.
    #[test]
    fn casing_does_not_matter() {
        assert!(line_writes_v2(
            r#"    .query("insert into market_bars_v2 (a) values (1)")"#
        ));
        assert!(line_writes_v2(
            r#"    .query("Optimize Table market_bars_v2 Final")"#
        ));
    }

    /// The staging and archive tables the rebuild creates are *different tables*,
    /// and writing them is how the rebuild preserved the rows. A prefix match would
    /// flag the one operation that saved the data. The live database holds a
    /// `market_bars_v2_premerge_20260911184559` from exactly that rebuild.
    #[test]
    fn the_staging_and_archive_tables_are_not_the_frozen_one() {
        assert!(!line_writes_v2(
            r#"    .query(&format!("INSERT INTO market_bars_v2_rebuild_{stamp} SELECT 1"))"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("DROP TABLE market_bars_v2_premerge_20260911184559")"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("OPTIMIZE TABLE market_bars_v2_rebuild_7 FINAL")"#
        ));
    }

    /// Reads stay legal: the v2 → `market_bar` migration has to read the table it is
    /// draining, and a check that forbade that would forbid the fix.
    #[test]
    fn reads_and_prose_are_not_writes() {
        assert!(!line_writes_v2(
            r#"    .query("SELECT count() FROM market_bars_v2")"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("SELECT * FROM trading.market_bars_v2 FINAL")"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("INSERT INTO market_bar (a) VALUES (1)")"#
        ));
        assert!(!line_writes_v2(
            r#"    .query(&format!("CREATE TABLE {staging} AS market_bars_v2"))"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("INSERT INTO {staging} SELECT DISTINCT * FROM market_bars_v2")"#
        ));
        assert!(!line_writes_v2(
            "// INSERT INTO market_bars_v2 is forbidden"
        ));
        assert!(!line_writes_v2(
            "//! writes its bars to `market_bars_v2` with"
        ));
        assert!(!line_writes_v2(" * INSERT INTO market_bars_v2"));
    }

    /// The check, run for real against this checkout.
    ///
    /// Without this the guard is a binary nobody invokes: `cargo test --workspace
    /// --lib` does not reach a `[[bin]]`'s tests, and `.github/workflows/ci.yml`
    /// has a `check-bars-v1-frozen` job with no v2 sibling. A rule enforced only by
    /// a workflow step that does not exist is a comment. This makes `cargo test -p
    /// xtask` fail the moment a new writer appears, whatever CI is or is not
    /// configured to run.
    ///
    /// # Seen to fail
    ///
    /// A guard nobody has watched fail is a guard nobody has tested. Both halves were
    /// exercised against this checkout on 2026-09-22:
    ///
    /// - **Violation.** `const PLANTED_P4_PROBE: &str = "insert into
    ///   trading.market_bars_v2 (open) values (1)";` appended to
    ///   `crates/dataplane/src/census.rs` — lower case and database-qualified, the two
    ///   evasions the matcher was widened for. The test failed naming
    ///   `crates/dataplane/src/census.rs:766`. Removed; the file is byte-identical to
    ///   its committed form.
    /// - **Drift.** The `backfill.rs` entry below was flipped from 1 to 2. The test
    ///   failed with "allowlist expects 2 write site(s), found 1". Reverted.
    ///
    /// The drift half is what stops this test passing vacuously. If the path handling
    /// broke and `scan_v2_writers` read nothing at all, every allowlisted count would
    /// come back 0 against an expected 1 and the test would fail rather than report a
    /// clean workspace — which is the failure mode a scanner-based guard actually dies
    /// of.
    #[test]
    fn nothing_new_writes_the_frozen_v2_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent")
            .to_path_buf();
        let (violations, drift) =
            super::scan_v2_writers(Some(&root)).expect("scan the workspace for v2 writers");
        assert!(
            violations.is_empty(),
            "new writes to the frozen market_bars_v2 table:\n{}",
            violations.join("\n")
        );
        assert!(
            drift.is_empty(),
            "V2_PRE_FREEZE_WRITERS is out of date:\n{}",
            drift.join("\n")
        );
    }
}

/// Every data source this platform declares is free, or has an allowlist entry
/// naming the ADR that approved paying for it (Set P P-4.5, DA-14, ADR-0027).
///
/// **The predicate is not here.** It lives in `dataplane::source`, beside the
/// declarations it governs and with its own tests, and this task calls it. That is
/// the whole reason `xtask` has a dependency at all: a grep in this file restating
/// "paid needs an ADR" would be a second implementation of the fence, free to
/// drift from the first, which is the defect shape the fence itself is guarding
/// against one level up.
///
/// **What this reads, and what that does and does not prove.** It reads the text
/// of `migrations/*.sql`, not the database. So it proves that every source the
/// migrations *insert* also has a `cost_class` the migrations *declare*, and that
/// every declared value passes the fence. It does not prove anything about a row
/// inserted by hand into a running database — nothing static can, and the column's
/// `NOT NULL` with no `DEFAULT` plus its `CHECK` is what covers that case
/// (migration 0104). The two halves are deliberate: the CHECK stops an
/// inadmissible *value*, and this stops an undeclared *source*, which the CHECK
/// cannot see because a source added without the column never reaches it.
fn check_free_sources() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits one level under the workspace root")
        .join("migrations");
    let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(&root) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sql"))
            .collect(),
        Err(e) => {
            eprintln!("check-free-sources: FAILED — cannot read {}: {e}", root.display());
            std::process::exit(1);
        }
    };
    files.sort();

    // name -> the file that inserted it, in migration order.
    let mut inserted: Vec<(String, String)> = Vec::new();
    // name -> the last cost_class declared for it, and where.
    let mut declared: Vec<(String, String, String)> = Vec::new();

    for f in &files {
        let Ok(sql) = std::fs::read_to_string(f) else {
            continue;
        };
        let where_ = f
            .file_name()
            .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().to_string());
        for name in source_names_inserted(&sql) {
            if !inserted.iter().any(|(n, _)| *n == name) {
                inserted.push((name, where_.clone()));
            }
        }
        for (name, value) in cost_classes_declared(&sql) {
            declared.retain(|(n, _, _)| *n != name);
            declared.push((name, value, where_.clone()));
        }
    }

    let mut problems: Vec<String> = Vec::new();

    if inserted.is_empty() {
        eprintln!(
            "check-free-sources: FAILED — no `INSERT INTO dataplane.source` found in any migration."
        );
        eprintln!("Either the table moved or this scanner's pattern no longer matches it. A fence");
        eprintln!("that silently finds nothing to check is worse than no fence: it reports OK.");
        std::process::exit(1);
    }

    for (name, at) in &inserted {
        if !declared.iter().any(|(n, _, _)| n == name) {
            problems.push(format!(
                "`{name}` is inserted in {at} and no migration declares its cost_class"
            ));
        }
    }

    // The fence itself, over every declared pair, through `dataplane`.
    let pairs: Vec<(&str, Option<&str>)> = declared
        .iter()
        .map(|(n, v, _)| (n.as_str(), Some(v.as_str())))
        .collect();
    if let Err(e) = dataplane::source::check_declared_cost_classes(pairs) {
        problems.push(e.to_string());
    }

    if problems.is_empty() {
        println!(
            "check-free-sources: OK — {} source(s), every one with a declared cost_class that passes the fence",
            inserted.len()
        );
        return;
    }

    eprintln!("check-free-sources: FAILED — {} problem(s):", problems.len());
    for p in &problems {
        eprintln!("  {p}");
    }
    eprintln!();
    eprintln!("ADR-0027 makes this platform free-only. Adding a paid source is a decision");
    eprintln!("someone signs, not a dependency that appears (DA-14). To add one: set");
    eprintln!("cost_class = 'paid' in the migration AND add a PaidSourceWaiver to");
    eprintln!("dataplane::source::PAID_SOURCE_ALLOWLIST naming the ADR that approved it.");
    eprintln!("To add a free one: say `cost_class = 'free'` for it in the migration. The");
    eprintln!("column has no DEFAULT on purpose, so silence is not an answer.");
    std::process::exit(1);
}

/// Where the SQL statement starting at `from` ends, honouring string literals.
///
/// **Not `find(';')`.** That was the first version and it was wrong in a way that
/// reported success: `dataplane.source` row 1 carries the note *"trades aggregated
/// to bars in-process; knowledge_time observed at close"*, so a naive scan ended
/// the 0046 insert after its first row and the check reported `OK -- 4 source(s)`
/// over a table holding nineteen. A guard that silently checks a quarter of its
/// subject is worse than no guard, because the number it prints looks like a
/// measurement. `''` inside a literal is Postgres' own escape for a quote and is
/// handled here for the same reason.
fn statement_end(sql: &str, from: usize) -> usize {
    let bytes = sql.as_bytes();
    let mut i = from;
    let mut in_str = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                if in_str && bytes.get(i + 1) == Some(&b'\'') {
                    i += 1; // an escaped quote, still inside the literal
                } else {
                    in_str = !in_str;
                }
            }
            b';' if !in_str => return i,
            _ => {}
        }
        i += 1;
    }
    sql.len()
}

/// Source names an `INSERT INTO dataplane.source ... VALUES (id, 'name', ...)`
/// adds, for every such statement in one migration.
///
/// Positional rather than column-aware, because every insert into this table in
/// this repository is `(source_id, name, …)` and a column-name parser would be a
/// small SQL implementation. If that shape ever changes, this returns nothing for
/// the new statement — which the empty-set check in the caller turns into a
/// failure rather than an OK.
fn source_names_inserted(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = sql.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("insert into dataplane.source") {
        let start = from + at;
        let end = statement_end(sql, start);
        for tuple in sql[start..end].split('(').skip(1) {
            let mut parts = tuple.split(',');
            let Some(first) = parts.next() else { continue };
            if first.trim().parse::<i64>().is_err() {
                continue; // not a `(id, 'name', …)` value tuple
            }
            if let Some(name) = parts.next().and_then(|p| quoted(p)) {
                out.push(name);
            }
        }
        from = end.max(start + 1);
    }
    out
}

/// `(name, cost_class)` for every `SET cost_class = '…' WHERE name = '…'` in one
/// migration.
///
/// Only the UPDATE form, deliberately: it is the form that names both halves on
/// one statement, so there is no positional guessing and no way for this scanner
/// to pair a class with the wrong source. A future migration that sets the column
/// inside an INSERT will leave the source undeclared as far as this check is
/// concerned, and the caller fails — which is the safe direction.
fn cost_classes_declared(sql: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lower = sql.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("set cost_class = ") {
        let start = from + at;
        let end = statement_end(sql, start);
        let stmt = &sql[start..end];
        let lstmt = &lower[start..end];
        let class = quoted(&stmt["set cost_class = ".len()..]);
        let name = lstmt.find("name = ").and_then(|w| quoted(&stmt[w + 7..]));
        if let (Some(c), Some(n)) = (class, name) {
            out.push((n, c));
        }
        from = end.max(start + 1);
    }
    out
}

/// The first single-quoted literal in `s`.
fn quoted(s: &str) -> Option<String> {
    let a = s.find('\'')?;
    let rest = &s[a + 1..];
    let b = rest.find('\'')?;
    Some(rest[..b].to_string())
}

#[cfg(test)]
mod free_sources_tests {
    use super::{cost_classes_declared, quoted, source_names_inserted, statement_end};

    /// `[was a silent quarter-measurement]` The first version of
    /// `statement_end` was `find(';')`, and `dataplane.source` row 1's note
    /// contains a semicolon. The scan ended the 0046 insert after its first row
    /// and `check-free-sources` printed `OK -- 4 source(s)` over nineteen. The
    /// failure was invisible precisely because it printed a number.
    #[test]
    fn a_semicolon_inside_a_string_literal_does_not_end_the_statement() {
        let sql = "INSERT INTO dataplane.source (source_id, name) VALUES\n  \
                   (1, 'a', 'x; y'),\n  (2, 'b', 'plain');\n";
        assert_eq!(source_names_inserted(sql), vec!["a", "b"]);
    }

    #[test]
    fn a_doubled_quote_is_an_escape_and_not_the_end_of_the_literal() {
        // Postgres writes a literal quote as ''. Reading it as the close means the
        // rest of the row is parsed as if it were SQL.
        let sql = "INSERT INTO dataplane.source (source_id, name) VALUES (1, 'a', 'it''s; fine'), (2, 'b', 'q');";
        assert_eq!(source_names_inserted(sql), vec!["a", "b"]);
    }

    #[test]
    fn the_statement_ends_at_the_first_semicolon_outside_a_literal() {
        let sql = "SELECT 'a;b'; SELECT 2;";
        assert_eq!(statement_end(sql, 0), 12);
    }

    /// An unterminated literal must not loop or panic; it ends the statement at
    /// end of input, which makes the caller see one oversized statement rather
    /// than nothing.
    #[test]
    fn an_unterminated_literal_ends_at_end_of_input() {
        let sql = "SELECT 'oops";
        assert_eq!(statement_end(sql, 0), sql.len());
    }

    #[test]
    fn a_column_list_is_not_mistaken_for_a_value_tuple() {
        // `(source_id, name, kind)` has no leading integer, so it is skipped —
        // otherwise `name` itself would be collected as a source called "name".
        let sql = "INSERT INTO dataplane.source (source_id, name, kind) VALUES (7, 'x', 'rest_history');";
        assert_eq!(source_names_inserted(sql), vec!["x"]);
    }

    #[test]
    fn a_declaration_pairs_the_class_with_the_source_on_its_own_statement() {
        let sql = "UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'coinbase_rest' AND cost_class IS NULL;\n\
                   UPDATE dataplane.source SET cost_class = 'paid' WHERE name = 'polygon';";
        assert_eq!(
            cost_classes_declared(sql),
            vec![
                ("coinbase_rest".to_string(), "free".to_string()),
                ("polygon".to_string(), "paid".to_string()),
            ]
        );
    }

    /// The whole reason only the UPDATE form is read: a value and a name on the
    /// same statement cannot be mispaired. A bulk `SET cost_class = 'free'` with
    /// no `WHERE name` declares nothing, and must be reported as declaring
    /// nothing rather than as declaring it for whatever source was nearby.
    #[test]
    fn a_bulk_update_with_no_name_declares_nothing() {
        let sql = "UPDATE dataplane.source SET cost_class = 'free' WHERE cost_class IS NULL;";
        assert!(cost_classes_declared(sql).is_empty());
    }

    #[test]
    fn quoted_reads_the_first_literal_and_nothing_after_it() {
        assert_eq!(quoted(" = 'free' WHERE"), Some("free".to_string()));
        assert_eq!(quoted("no literal here"), None);
    }

    /// The scanner runs over the real migrations, and finds every source the real
    /// `dataplane.source` inserts. Pinning the count is what would fail if a
    /// migration added a source in a shape this scanner cannot see — at which
    /// point the fix is the scanner, not the number.
    #[test]
    fn every_source_the_migrations_insert_is_found_and_declared() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("migrations");
        let mut inserted: Vec<String> = Vec::new();
        let mut declared: Vec<String> = Vec::new();
        let mut files: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sql"))
            .collect();
        files.sort();
        for f in files {
            let sql = std::fs::read_to_string(&f).unwrap();
            for n in source_names_inserted(&sql) {
                if !inserted.contains(&n) {
                    inserted.push(n);
                }
            }
            for (n, _) in cost_classes_declared(&sql) {
                if !declared.contains(&n) {
                    declared.push(n);
                }
            }
        }
        assert!(
            inserted.len() >= 19,
            "only {} sources found; 0046 inserts 16 and 0098 inserts 3, so a count \
             below 19 means the scanner stopped seeing some of them: {inserted:?}",
            inserted.len()
        );
        let mut missing: Vec<&String> = inserted.iter().filter(|n| !declared.contains(n)).collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "sources with no declared cost_class: {missing:?}. The column has no DEFAULT \
             on purpose (ADR-0027, DA-14), so a source nobody classified must fail."
        );
    }
}
