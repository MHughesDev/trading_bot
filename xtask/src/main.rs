//! Dev automation tasks.
//! Usage: cargo xtask <command>
//! Commands:
//!   check-money-f64  — scan workspace for f64 usage on price/size (CI enforced)

use std::process::Command;

fn main() {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-money-f64") => check_money_f64(),
        Some(t) => {
            eprintln!("Unknown xtask: {t}");
            std::process::exit(1);
        }
        None => {
            println!("Usage: cargo xtask <check-money-f64>");
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
            let has_money_keyword = money_keywords.iter().any(|kw| line.contains(kw));
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
