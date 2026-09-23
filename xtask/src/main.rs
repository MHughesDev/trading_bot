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

/// Path segments under `serde_json::` that turn a value into JSON or JSON back
/// into a value, plus the dynamic document that carries one.
///
/// Everything else `serde_json` exports — `RawValue`, `Number`, `Error` — describes
/// the *shape* of bytes somebody else already sent us. Naming one of those in a
/// struct field or a signature is not payload encoding, and the version of this
/// check that ran until 2026-09-22 counted all nine `Option<serde_json::Number>`
/// fields in `derivs/funding.rs` as violations of a rule about how *we* encode events.
const JSON_CODEC_ITEMS: &[&str] = &[
    "from_reader",
    "from_slice",
    "from_str",
    "from_value",
    "json",
    "to_string",
    "to_string_pretty",
    "to_value",
    "to_vec",
    "to_vec_pretty",
    "to_writer",
    "to_writer_pretty",
    "Map",
    "Value",
];

/// The one way to use `serde_json` in a hot-path crate and keep the check green.
///
/// It is a marker rather than a spelling because no pattern over source text can tell
/// a venue's response body from one of our own event payloads — only the author
/// knows, so the author writes it down. The reason after the colon is mandatory: an
/// exemption nobody had to justify is an exemption nobody reviews.
const BOUNDARY_MARKER: &str = "json-at-boundary:";

/// How many code lines a standalone marker stays open over before it is reported as
/// covering nothing.
///
/// A marker covers the *statement* below it, not the line below it, because `cargo fmt`
/// decides which of those two the call lands on. The cap is what stops "the statement
/// below" degrading into "the rest of the function" when a file is malformed or a
/// statement never terminates: a marker cannot silently acquire a call the author never
/// looked at.
const MARKER_SPAN_LINES: u32 = 8;

/// A standalone marker that has not yet found the call it covers.
#[derive(Clone, Copy)]
struct OpenMarker {
    line: usize,
    lines_left: u32,
}

/// Crates that carry `EventEnvelope` payloads on the market-data hot path.
const JSON_HOTPATH_CRATES: &[&str] = &[
    "crates/collectors",
    "crates/event-bus",
    "crates/strategy-runtime",
    "crates/features",
];

/// Forbid `serde_json` payload encoding in the market-lane hot-path crates, where
/// ADR-0003's in-process pipeline carries rkyv-encoded envelopes.
///
/// The rule is narrow and worth stating exactly, because the version that ran until
/// 2026-09-22 enforced a different and unsatisfiable one:
///
/// - **Flagged:** a call into `serde_json`'s codec API (`to_vec`, `from_str`,
///   `json!`, …) or a use of the dynamic `serde_json::Value`/`Map` document.
/// - **Not flagged:** `use serde_json::value::RawValue;`, `serde_json::Number` in a
///   field type, `serde_json::Error` in a signature. Those name someone else's bytes.
/// - **Exempt:** any flagged line carrying `// json-at-boundary: <reason>`, either
///   trailing on the line itself or alone on the line directly above it.
///
/// The old check matched substrings of the *argument spelling*: a `from_str` line
/// passed if it contained `&body`, `&raw`, `&text` or `&buf`. For a `&str` body the
/// only spelling that satisfied it was `serde_json::from_str::<T>(&body[..])`, which
/// is `clippy::redundant_slicing`, which `-D warnings` rejects. The escape hatch and
/// the clippy gate were mutually unsatisfiable, so this check sat red and unwired
/// while fifteen legitimate boundary parses accumulated under it.
///
/// Skips: `*/tests/*`, `#[cfg(test)]` blocks, and `event-bus`'s quarantine lane,
/// which is JSON on purpose.
fn lint_no_json_hotpath() {
    let (violations, files_scanned) = match scan_json_hotpath(None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lint-no-json-hotpath: {e}");
            std::process::exit(2);
        }
    };

    // A scanner that reads nothing reports a clean workspace. Refuse to call that a
    // pass: every hot-path crate listed above has Rust in it.
    if files_scanned == 0 {
        eprintln!(
            "lint-no-json-hotpath: FAILED — scanned 0 files across {} hot-path crate(s). \
             The scan is broken, not the workspace.",
            JSON_HOTPATH_CRATES.len()
        );
        std::process::exit(2);
    }

    if violations.is_empty() {
        println!(
            "lint-no-json-hotpath: OK — {files_scanned} file(s) checked, no unmarked serde_json \
             codec use in the market-lane hot path"
        );
        return;
    }

    eprintln!(
        "lint-no-json-hotpath: FAILED — {} violation(s):",
        violations.len()
    );
    for v in &violations {
        eprintln!("  {v}");
    }
    eprintln!();
    eprintln!(
        "Hot-path payloads are rkyv (ADR-0003). If a line decodes a venue's REST or\n\
         WebSocket bytes rather than one of our envelopes, say so at the call site:\n\
         \n    // {BOUNDARY_MARKER} Kraken OHLC REST response\
         \n    let bars: Vec<Ohlc> = serde_json::from_str(body)?;\n\
         \n\
         The reason is required, and a marker covering nothing is itself a violation."
    );
    std::process::exit(1);
}

/// Walk the hot-path crates' Rust sources and return `(violations, files_scanned)`.
///
/// `root` is the repository root. `None` means the current directory, which is what
/// `cargo xtask` gets; the self-test passes an explicit path so it does not depend on
/// where the harness happens to be standing.
fn scan_json_hotpath(root: Option<&std::path::Path>) -> Result<(Vec<String>, usize), String> {
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
    let mut files_scanned = 0usize;
    // During an unresolved merge `git ls-files` prints a conflicted path once per stage,
    // so without this every violation in a conflicted file is reported three times.
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();

    for file in file_list.lines().filter(|f| seen.insert(f)) {
        if file.contains("target/") || file.contains("/tests/") {
            continue;
        }
        if !JSON_HOTPATH_CRATES.iter().any(|p| file.starts_with(p)) {
            continue;
        }
        // The quarantine publisher is JSON by design: a payload that failed to decode
        // has no typed form left to re-encode.
        if file.ends_with("quarantine.rs") {
            continue;
        }

        let path = match root {
            Some(r) => r.join(file),
            None => std::path::PathBuf::from(file),
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        files_scanned += 1;
        violations.extend(scan_json_hotpath_file(file, &content));
    }

    Ok((violations, files_scanned))
}

/// Classify every line of one file. Pure, so the rules above are testable without a
/// workspace to scan.
fn scan_json_hotpath_file(file: &str, content: &str) -> Vec<String> {
    let mut violations: Vec<String> = vec![];
    let mut in_test_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    // A marker standing on its own line, still open over the statement below it.
    let mut pending: Option<OpenMarker> = None;

    let at = |lineno: usize, line: &str, msg: &str| {
        format!("{}:{}: {}\n  {}", file, lineno + 1, msg, line.trim())
    };
    let stale = |lineno: usize| {
        format!(
            "{}:{}: `{BOUNDARY_MARKER}` marker covers no serde_json use — remove it or \
             move it onto the line it exempts",
            file,
            lineno + 1
        )
    };

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

        if in_test_depth > 0 {
            pending = None;
            continue;
        }

        let comment_only =
            trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*');
        let code = if comment_only {
            ""
        } else {
            code_before_comment(line)
        };
        let marker = boundary_reason(line);

        // An import hides the `serde_json::` prefix from every call site below it, so a
        // marker would have nothing left to attach to. Codec items stay qualified.
        if let Some(item) = imported_codec_item(code) {
            violations.push(at(
                lineno,
                line,
                &format!(
                    "`use serde_json::{item}` hides the call sites — keep the `serde_json::` \
                     path so each one can carry its own `{BOUNDARY_MARKER}` marker"
                ),
            ));
            pending = None;
            continue;
        }

        match (json_codec_item(code), marker) {
            (Some(_), Some(reason)) => {
                if reason.is_empty() {
                    violations.push(at(
                        lineno,
                        line,
                        &format!("`{BOUNDARY_MARKER}` needs a reason after the colon"),
                    ));
                }
                pending = None;
            }
            (Some(_), None) if pending.is_some() => {
                pending = None;
            }
            (Some(item), None) => {
                violations.push(at(
                    lineno,
                    line,
                    &format!(
                        "`serde_json::{item}` on the market-data hot path — payloads are rkyv; \
                         mark a venue-boundary decode with `{BOUNDARY_MARKER} <reason>`"
                    ),
                ));
            }
            (None, marker) => {
                // The marker stays open across the rest of its statement. Closing it at
                // the next line would make it rustfmt-dependent: `cargo fmt` breaks
                // `let env: T = serde_json::from_str(body).map_err(…)?;` after the `=`
                // when the line is over 100 columns, which puts the call one line
                // further down than the author left it. That is not a decision about
                // whether the decode is legal, and a check that changes its answer when
                // the formatter runs is a check nobody can keep green.
                if let Some(open) = pending.as_mut() {
                    if !code.trim().is_empty() {
                        open.lines_left = open.lines_left.saturating_sub(1);
                        if open.lines_left == 0 || ends_statement(code) {
                            violations.push(stale(open.line));
                            pending = None;
                        }
                    }
                }
                match marker {
                    Some("") => violations.push(at(
                        lineno,
                        line,
                        &format!("`{BOUNDARY_MARKER}` needs a reason after the colon"),
                    )),
                    Some(_) if comment_only => {
                        if let Some(orphan) = pending.replace(OpenMarker {
                            line: lineno,
                            lines_left: MARKER_SPAN_LINES,
                        }) {
                            violations.push(stale(orphan.line));
                        }
                    }
                    Some(_) => violations.push(stale(lineno)),
                    None => {}
                }
            }
        }
    }

    if let Some(orphan) = pending {
        violations.push(stale(orphan.line));
    }

    violations
}

/// True when this line of code closes a statement or opens a block, so an open marker
/// above it has run out of statement to cover.
///
/// A line ending in `=`, `(`, `,`, `.` or an operator is a continuation — which is
/// exactly what `cargo fmt` produces when it breaks a long `let` after the `=`.
fn ends_statement(code: &str) -> bool {
    matches!(
        code.trim_end().chars().next_back(),
        Some(';') | Some('{') | Some('}')
    )
}

/// The part of `line` before its `//` comment, with `//` inside a string literal left
/// alone — a URL in a subscribe message must not truncate the code being classified.
fn code_before_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// The reason text of a `json-at-boundary:` marker, when the line carries one inside a
/// comment. `Some("")` is a marker with no reason, a violation in its own right.
fn boundary_reason(line: &str) -> Option<&str> {
    let idx = line.find(BOUNDARY_MARKER)?;
    // Inside a comment, not inside a string literal that happens to quote the marker.
    if !line[..idx].contains("//") {
        return None;
    }
    Some(line[idx + BOUNDARY_MARKER.len()..].trim())
}

/// The codec item a `use serde_json…` line brings into scope under a bare name, if any.
///
/// `use serde_json::value::RawValue;` is not one: `RawValue` borrows a venue's own
/// bytes without decoding them, which is the opposite of the thing being forbidden.
fn imported_codec_item(code: &str) -> Option<&'static str> {
    let trimmed = code.trim_start();
    if !(trimmed.starts_with("use ") || trimmed.starts_with("pub use ")) {
        return None;
    }
    if !trimmed.contains("serde_json") {
        return None;
    }
    identifiers(trimmed).find_map(|ident| JSON_CODEC_ITEMS.iter().copied().find(|i| *i == ident))
}

/// The codec item named on a `serde_json::…` path in `code`, if any.
///
/// Reads whole `::`-separated segments rather than substrings, so `serde_json::value`
/// does not read as `serde_json::Value` and `serde_json::Number` does not match at all.
fn json_codec_item(code: &str) -> Option<&'static str> {
    const PREFIX: &str = "serde_json::";
    let mut from = 0;
    while let Some(rel) = code[from..].find(PREFIX) {
        let mut pos = from + rel + PREFIX.len();
        loop {
            let rest = &code[pos..];
            let len = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if len == 0 {
                break;
            }
            let segment = &rest[..len];
            if let Some(hit) = JSON_CODEC_ITEMS.iter().copied().find(|i| *i == segment) {
                return Some(hit);
            }
            if rest[len..].starts_with("::") {
                pos += len + 2;
            } else {
                break;
            }
        }
        from += rel + PREFIX.len();
    }
    None
}

/// Every Rust identifier in `code`, in order.
fn identifiers(code: &str) -> impl Iterator<Item = &str> {
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
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
    // ---- lint-no-json-hotpath -------------------------------------------------
    //
    // The rules below are the ones the previous version of this check got wrong.
    // Each test names the line it used to mis-read.

    use super::{boundary_reason, code_before_comment, imported_codec_item, json_codec_item};

    /// What the check is actually for: JSON where an rkyv envelope belongs.
    #[test]
    fn codec_calls_are_hits() {
        assert_eq!(
            json_codec_item("serde_json::to_vec(&envelope)"),
            Some("to_vec")
        );
        assert_eq!(
            json_codec_item("let v: T = serde_json::from_slice(body)?;"),
            Some("from_slice")
        );
        assert_eq!(
            json_codec_item("serde_json::from_str(&text)"),
            Some("from_str")
        );
        assert_eq!(
            json_codec_item("let m = serde_json::json!({});"),
            Some("json")
        );
        assert_eq!(
            json_codec_item("let v: serde_json::Value = x;"),
            Some("Value")
        );
    }

    /// The hole this rewrite closes. `serde_json::Number` in a struct field and
    /// `serde_json::value::RawValue` in a signature are declarations about *someone
    /// else's* bytes — they encode nothing. The old substring check counted eleven of
    /// them as violations of a rule about how we encode our own events, which is most
    /// of why it sat red and unwired.
    #[test]
    fn type_mentions_are_not_hits() {
        assert_eq!(
            json_codec_item("    funding_rate: Option<serde_json::Number>,"),
            None
        );
        assert_eq!(
            json_codec_item("pub fn raw_u64(v: &serde_json::value::RawValue) -> Result<u64, E> {"),
            None
        );
        assert_eq!(
            json_codec_item("fn f() -> Result<T, serde_json::Error> {"),
            None
        );
    }

    /// `value` is not `Value`. A substring match on the dynamic document would swallow
    /// every `RawValue` path in `backfill/`, which is the zero-copy reader that exists
    /// precisely so venue numbers never round-trip through `f64`.
    #[test]
    fn segments_are_whole_not_substrings() {
        assert_eq!(json_codec_item("serde_json::value::RawValue"), None);
        assert_eq!(json_codec_item("serde_json::Value"), Some("Value"));
        assert_eq!(
            json_codec_item("serde_json::from_value(v)"),
            Some("from_value")
        );
    }

    /// The other half of the old failure: whether a line passed depended on how its
    /// argument happened to be spelled. `&body[..]` passed, `body` did not — and
    /// `&body[..]` is `clippy::redundant_slicing`, which `-D warnings` rejects. Both
    /// spellings are now the same hit, and the marker is what decides.
    #[test]
    fn argument_spelling_does_not_decide() {
        let plain = "    let p = serde_json::from_str::<T>(body)";
        let sliced = "    let p = serde_json::from_str::<T>(&body[..])";
        assert_eq!(json_codec_item(plain), json_codec_item(sliced));
        assert_eq!(json_codec_item(plain), Some("from_str"));
    }

    #[test]
    fn a_marker_needs_a_comment_and_a_reason() {
        assert_eq!(
            boundary_reason("    // json-at-boundary: Kraken OHLC"),
            Some("Kraken OHLC")
        );
        assert_eq!(
            boundary_reason("    let x = 1; // json-at-boundary: why"),
            Some("why")
        );
        assert_eq!(boundary_reason("    // json-at-boundary:"), Some(""));
        assert_eq!(
            boundary_reason("    let s = \"json-at-boundary: not a comment\";"),
            None
        );
        assert_eq!(boundary_reason("    let x = 1;"), None);
    }

    /// A `//` inside a URL must not truncate the code being classified, or a subscribe
    /// frame would hide its own `json!` behind `wss://`.
    #[test]
    fn a_url_is_not_a_comment() {
        let line = "    let url = \"wss://ws.kraken.com\"; serde_json::to_vec(&x)";
        assert_eq!(json_codec_item(code_before_comment(line)), Some("to_vec"));
        assert_eq!(
            code_before_comment("    let x = 1; // serde_json::to_vec(&x)").trim(),
            "let x = 1;"
        );
    }

    #[test]
    fn importing_a_codec_item_is_a_hit_but_importing_rawvalue_is_not() {
        assert_eq!(
            imported_codec_item("use serde_json::value::RawValue;"),
            None
        );
        assert_eq!(imported_codec_item("use serde_json::Number;"), None);
        assert_eq!(
            imported_codec_item("use serde_json::{json, Value};"),
            Some("json")
        );
        assert_eq!(imported_codec_item("use serde_json::Value;"), Some("Value"));
        // Not an import at all.
        assert_eq!(
            imported_codec_item("let v = serde_json::Value::Null;"),
            None
        );
    }

    #[test]
    fn an_unmarked_codec_call_is_a_violation() {
        let v = super::scan_json_hotpath_file("f.rs", "fn f() {\n    serde_json::to_vec(&e);\n}\n");
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].starts_with("f.rs:2:"), "{v:?}");
    }

    #[test]
    fn a_marker_exempts_the_line_above_it_and_its_own_line() {
        let above = "fn f() {\n    // json-at-boundary: venue REST body\n    serde_json::from_str(body);\n}\n";
        assert!(super::scan_json_hotpath_file("f.rs", above).is_empty());

        let trailing =
            "fn f() {\n    serde_json::from_str(body); // json-at-boundary: venue REST body\n}\n";
        assert!(super::scan_json_hotpath_file("f.rs", trailing).is_empty());
    }

    /// An exemption nobody had to justify is an exemption nobody reviews.
    #[test]
    fn a_marker_without_a_reason_is_a_violation() {
        let src = "fn f() {\n    // json-at-boundary:\n    serde_json::from_str(body);\n}\n";
        let v = super::scan_json_hotpath_file("f.rs", src);
        // Two: the marker is named as unusable, and the call it failed to cover is
        // still unmarked. Reporting only the first would leave the author thinking the
        // line below it had been accepted.
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].contains("needs a reason"), "{v:?}");
        assert!(v[1].contains("payloads are rkyv"), "{v:?}");
    }

    /// A marker covers the *statement* below it, not the line below it.
    ///
    /// This is not a convenience. `cargo fmt` breaks a `let` after the `=` when the line
    /// is over 100 columns, which moves the call one line further down than the author
    /// left it — and the workspace-wide `cargo fmt --all` of 2026-09-22 did exactly that
    /// to both `serde_json::from_str` calls in `derivs/funding.rs`. Under a next-line
    /// rule each one became two violations, a stale marker and an unmarked call, from a
    /// commit that changed no code. A check whose answer depends on whether the
    /// formatter has run is a check nobody can keep green.
    #[test]
    fn a_marker_survives_rustfmt_breaking_the_line() {
        let formatted = "fn f() {\n    // json-at-boundary: venue REST body\n    let env: TickersEnvelope =\n        serde_json::from_str(body).map_err(|e| E::Http(e.to_string()))?;\n}\n";
        assert!(
            super::scan_json_hotpath_file("f.rs", formatted).is_empty(),
            "{:?}",
            super::scan_json_hotpath_file("f.rs", formatted)
        );
    }

    /// The bound on the rule above: "the statement below" must not decay into "the rest
    /// of the function", or a marker quietly acquires a call nobody reviewed.
    #[test]
    fn a_marker_does_not_reach_past_its_own_statement() {
        let src = "fn f() {\n    // json-at-boundary: venue REST body\n    let x = 1;\n    serde_json::to_vec(&envelope);\n}\n";
        let v = super::scan_json_hotpath_file("f.rs", src);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].contains("covers no serde_json use"), "{v:?}");
        assert!(v[1].contains("payloads are rkyv"), "{v:?}");
    }

    #[test]
    fn a_continuation_line_does_not_close_a_statement() {
        assert!(!super::ends_statement("    let env: TickersEnvelope ="));
        assert!(!super::ends_statement("        .map_err(|e| E::Http(e))"));
        assert!(super::ends_statement("    let x = 1;"));
        assert!(super::ends_statement(
            "    match serde_json::from_slice(&raw) {"
        ));
    }

    /// The rot the old allowlist died of, made loud. When the call a marker covered is
    /// deleted or moved, the marker is reported instead of silently widening.
    #[test]
    fn a_marker_covering_nothing_is_a_violation() {
        let orphan = "fn f() {\n    // json-at-boundary: venue REST body\n    let x = 1;\n}\n";
        let v = super::scan_json_hotpath_file("f.rs", orphan);
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("covers no serde_json use"), "{v:?}");

        let at_eof = "fn f() {}\n// json-at-boundary: venue REST body\n";
        assert_eq!(super::scan_json_hotpath_file("f.rs", at_eof).len(), 1);
    }

    /// Test code round-trips its own types through JSON on purpose; the rule is about
    /// what ships.
    #[test]
    fn cfg_test_blocks_are_exempt() {
        let src = "#[cfg(test)]\nmod tests {\n    fn f() {\n        serde_json::to_string(&x);\n    }\n}\n";
        assert!(super::scan_json_hotpath_file("f.rs", src).is_empty());
    }

    /// The scan half of the guard, run against the real workspace.
    ///
    /// `files_scanned` is what stops this passing vacuously: if the path handling broke
    /// and the scanner read nothing at all, `violations` would be empty and the check
    /// would report a clean hot path — the failure mode a scanner-based guard actually
    /// dies of. The floor is deliberately far below the current 76.
    #[test]
    fn the_hot_path_carries_no_unmarked_json() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent")
            .to_path_buf();
        let (violations, files_scanned) =
            super::scan_json_hotpath(Some(&root)).expect("scan the hot-path crates");
        assert!(
            violations.is_empty(),
            "unmarked serde_json on the market-data hot path:\n{}",
            violations.join("\n")
        );
        assert!(
            files_scanned >= 40,
            "scanned only {files_scanned} hot-path files — the scan is broken, not the workspace"
        );
    }

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
