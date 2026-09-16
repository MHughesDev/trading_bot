//! Set T MASTER §5's hard boundary: **no task in this set may introduce a
//! live-venue code path**, and a test asserts the paper venue is the only
//! reachable one from anything the set adds.
//!
//! Two entry points were added by Set T Phase 0 — [`DeploymentSpec`] (which
//! names the venue a forward record belongs to) and [`ForwardRecorder`] (which
//! consumes fills for it). Both are checked here, plus the source of every
//! module the phase added, because a live URL reaches a venue whether or not it
//! passes through a typed value.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use domain::numeraire::Numeraire;
use reconciliation::forward::{DeploymentSpec, DivergenceThresholds, PaperVenue};
use reconciliation::ForwardRecorder;
use rust_decimal::Decimal;
use uuid::Uuid;

/// Files this phase added or extended. Each is checked for a live-venue path.
const PHASE_FILES: &[&str] = &[
    "crates/reconciliation/src/forward.rs",
    "crates/reconciliation/src/recorder.rs",
    "crates/reconciliation/src/divergence.rs",
    "crates/reconciliation/src/gate15.rs",
    "crates/reconciliation/src/store.rs",
    "crates/risk/src/strategy_halt.rs",
    "crates/domain/src/forward.rs",
    "crates/domain/src/numeraire.rs",
    "migrations/0105_forward_observation.sql",
];

/// Hosts and identifiers that would mean real money.
const LIVE_MARKERS: &[&str] = &[
    "api.alpaca.markets",
    "api.coinbase.com",
    "api.exchange.coinbase.com",
    "api.kraken.com",
    "live_base_url",
    "live_url",
    "is_live",
    "live_trading",
    "\"live\"",
    "'live'",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/reconciliation sits two levels below the repo root")
        .to_path_buf()
}

fn code_only(path: &Path) -> String {
    let rust = path.extension().and_then(|e| e.to_str()) == Some("rs");
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} is unreadable: {e}", path.display()))
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            if t.starts_with("//") || t.starts_with("--") {
                return false;
            }
            rust || !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

/// The venue enum has exactly one variant and it is paper. This is the boundary
/// stated where the compiler can enforce it: naming a live venue is not a
/// string somebody types, it is a source change and a review.
#[test]
fn the_only_venue_this_set_can_name_is_paper() {
    for v in [PaperVenue::AlpacaPaper] {
        assert!(v.is_paper(), "{v:?} is not a paper venue");
    }
    assert_eq!(PaperVenue::AlpacaPaper.as_str(), "alpaca_paper");

    // And the source says so: one variant, no `Live`.
    let src = code_only(&repo_root().join("crates/reconciliation/src/forward.rs"));
    assert!(
        !src.contains("live"),
        "PaperVenue's module must not name a live venue"
    );
}

/// Every entry point this phase added is reached only with a paper venue.
#[test]
fn every_entry_point_added_by_this_phase_takes_a_paper_venue() {
    let spec = DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: "sma-cross-1".into(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::AlpacaPaper,
        numeraire: Numeraire::Usd,
        bar_seconds: 60,
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 0.01,
            max_cum_return_divergence: 0.03,
            max_position_divergence: Decimal::from_str("0.000001").unwrap(),
            min_signal_match: 0.99,
        },
    };
    assert!(spec.venue.is_paper());

    let halts = Arc::new(risk::StrategyHaltRegistry::new());
    let recorder = ForwardRecorder::new(
        spec.clone(),
        Decimal::from_str("100000").unwrap(),
        Arc::clone(&halts),
    )
    .expect("a valid paper deployment");
    // The recorder cannot be constructed for anything else: `DeploymentSpec`
    // has no constructor that takes a venue string, and `PaperVenue` has no
    // live variant to hand it.
    assert!(recorder.spec_is_paper());
}

/// No module this phase added reaches a live venue, by URL or by flag.
#[test]
fn no_module_added_by_this_phase_names_a_live_venue() {
    let root = repo_root();
    let mut violations = Vec::new();
    for rel in PHASE_FILES {
        let path = root.join(rel);
        let body = code_only(&path);
        for marker in LIVE_MARKERS {
            if body.contains(marker) {
                violations.push(format!("{rel} contains {marker:?}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Set T may not create a live-venue code path:\n{}",
        violations.join("\n")
    );
}

/// The database says it too. Migration 0105's `venue` CHECK admits one value,
/// so widening it is a migration, a review, and roadmap gate H-4.
#[test]
fn the_deployment_table_admits_only_the_paper_venue() {
    let sql = std::fs::read_to_string(repo_root().join("migrations/0105_forward_observation.sql"))
        .expect("migration 0105");
    assert!(
        sql.contains("venue             TEXT NOT NULL CHECK (venue IN ('alpaca_paper'))"),
        "0105 must constrain `venue` to the paper venue alone"
    );
}
