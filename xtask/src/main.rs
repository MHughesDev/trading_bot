//! Dev automation tasks.
//! Usage: cargo xtask <command>
//! Commands:
//!   check-money-f64       — scan workspace for f64 usage on price/size (CI enforced)
//!   lint-no-json-hotpath  — verify serde_json absent from market-lane hot paths (CI enforced)
//!   check-bars-v1-frozen  — verify nothing writes the retired market_bars table (CI enforced)

use std::process::Command;

fn main() {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-money-f64") => check_money_f64(),
        Some("lint-no-json-hotpath") => lint_no_json_hotpath(),
        Some("check-bars-v1-frozen") => check_bars_v1_frozen(),
        Some(t) => {
            eprintln!("Unknown xtask: {t}");
            std::process::exit(1);
        }
        None => {
            println!(
                "Usage: cargo xtask <check-money-f64|lint-no-json-hotpath|check-bars-v1-frozen>"
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
            if has_money_keyword {
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
    eprintln!("instant (DATA-005 §3). Write market_bars_v2 instead.");
    std::process::exit(1);
}
