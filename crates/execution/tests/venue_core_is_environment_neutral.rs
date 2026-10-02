//! Set U U-2.1: the venue core paper and backtest share has no idea which it is
//! running in. It reads time only through its injected clock and learns prices
//! only from marks and bars pushed into it.
//!
//! A source scan, because the failure is a single `Utc::now()` added to a fill
//! path one day: every test still passes, paper still works, and every backtest
//! from then on stamps its fills with the day it was run rather than the bar it
//! describes.

use std::path::Path;

/// The venue core: the engine, its accounts and ledger, the fill simulators,
/// and the simulated venue's driver.
const CORE: &[&str] = &[
    "src/paper/engine.rs",
    "src/paper/account.rs",
    "src/paper/ledger.rs",
    "src/paper/clob.rs",
    "src/paper/broker_quote.rs",
    "src/paper/amm_swap.rs",
    "src/paper/prediction.rs",
    "src/sim_venue.rs",
    "src/sim_risk.rs",
];

/// Clock reads that bypass the injected clock.
const CLOCK_READS: &[&str] = &[
    "Utc::now()",
    "Local::now()",
    "SystemTime::now()",
    "Instant::now()",
];

/// Anything that could fetch a price instead of being handed one.
const MARK_SOURCES: &[&str] = &["reqwest", "tokio::", "tungstenite", "sqlx", "async fn"];

/// Source with `//` comments and the `#[cfg(test)]` module removed.
fn production_code(path: &Path) -> String {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let body = src.split("#[cfg(test)]").next().unwrap_or("");
    body.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_venue_core_reads_no_clock_but_its_own() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for rel in CORE {
        let code = production_code(&root.join(rel));
        for needle in CLOCK_READS.iter().chain(MARK_SOURCES) {
            if code.contains(needle) {
                violations.push(format!("{rel} contains `{needle}`"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the venue core must take time from its VenueClock and prices from what it is handed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_wall_clock_lives_in_exactly_one_place() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let clock = production_code(&root.join("src/clock.rs"));
    assert_eq!(
        clock.matches("Utc::now()").count(),
        1,
        "WallClock is the one wall-clock read"
    );
}
