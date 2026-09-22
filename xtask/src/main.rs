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

    let file_list =
        String::from_utf8(output.stdout).map_err(|_| "invalid utf8 from git ls-files".to_string())?;
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
        assert!(line_writes_v2(r#"    .query("TRUNCATE TABLE market_bars_v2")"#));
        assert!(line_writes_v2(r#"    .query("DROP TABLE market_bars_v2")"#));
        assert!(line_writes_v2(
            r#"    .query("DROP TABLE IF EXISTS market_bars_v2")"#
        ));
        assert!(line_writes_v2(r#"    .query("DETACH TABLE market_bars_v2")"#));
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
        assert!(!line_writes_v2(r#"    .query("SELECT count() FROM market_bars_v2")"#));
        assert!(!line_writes_v2(
            r#"    .query("SELECT * FROM trading.market_bars_v2 FINAL")"#
        ));
        assert!(!line_writes_v2(r#"    .query("INSERT INTO market_bar (a) VALUES (1)")"#));
        assert!(!line_writes_v2(
            r#"    .query(&format!("CREATE TABLE {staging} AS market_bars_v2"))"#
        ));
        assert!(!line_writes_v2(
            r#"    .query("INSERT INTO {staging} SELECT DISTINCT * FROM market_bars_v2")"#
        ));
        assert!(!line_writes_v2("// INSERT INTO market_bars_v2 is forbidden"));
        assert!(!line_writes_v2("//! writes its bars to `market_bars_v2` with"));
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
