//! An absent forward record is not an empty one.
//!
//! Gate 15 reads `mlops.forward_observation`. Zero rows there has four causes
//! and they do not mean the same thing:
//!
//! * reconciliation never attached to the fill stream;
//! * it attached after fills had already been published, so rows are missing
//!   that cannot be supplied — the fill stream has no replay;
//! * it attached, with nothing deployed, so the forward clock never started;
//! * it attached, with deployments, and those deployments genuinely did not
//!   trade.
//!
//! Only the last one is a zero. This file pins the other three, which is the
//! only reason the counters in `recorder.rs` exist.
//!
//! Its own test binary, deliberately: the registry is a process-wide
//! `OnceLock`, so a test that asserts "nothing is attached" cannot share a
//! process with one that attaches something. It is also why the whole file is
//! one `#[test]` — the states are ordered in time and a second test function
//! would observe them in whatever order the harness chose.

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use domain::money::Price;
use domain::numeraire::Numeraire;
use execution::fills::{FillEvent, FillStreamVerdict};
use reconciliation::forward::{DeploymentSpec, DivergenceThresholds, PaperVenue};
use reconciliation::{ForwardRecorders, ForwardRecordingGap, ForwardRecordingVerdict};
use rust_decimal::Decimal;
use uuid::Uuid;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn a_fill() -> FillEvent {
    FillEvent {
        idempotency_key: Uuid::new_v4(),
        broker_order_id: "absent-1".to_owned(),
        filled_qty: dec("1"),
        fill_price: Price::from_str("50000").unwrap(),
        commission: Decimal::ZERO,
        filled_at: Utc::now(),
    }
}

fn spec() -> DeploymentSpec {
    DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: "late-1".to_owned(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::Platform,
        numeraire: Numeraire::Usd,
        bar_seconds: 60,
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 1.0,
            max_cum_return_divergence: 1.0,
            max_position_divergence: dec("0.000001"),
            min_signal_match: 0.5,
        },
    }
}

#[test]
fn the_four_states_of_an_empty_forward_record_are_four_different_facts() {
    // ── 1. Nothing attached. ──
    assert!(
        ForwardRecorders::installed().is_none(),
        "this binary installs nothing up front"
    );
    let absent = ForwardRecorders::process_evidence();
    assert!(!absent.installed);
    assert_eq!(absent.verdict(), ForwardRecordingVerdict::NotInstalled);
    assert_eq!(
        ForwardRecorders::require_recording(),
        Err(ForwardRecordingGap::NotInstalled),
        "a process that would trade without recording must fail its guard, not \
         report a clean zero"
    );

    // ── 2. A fill happens anyway. ──
    //
    // The execution stream counts it as unobserved because nothing at all is
    // registered here; had some *other* observer been registered, that counter
    // would read zero while the forward record stayed empty. Which is exactly
    // why reconciliation keeps its own watermark instead of trusting it.
    execution::fills::publish(&a_fill());
    assert_eq!(
        execution::fills::evidence().verdict(),
        FillStreamVerdict::EvidenceLost { fills: 1 },
        "the execution stream reports the loss at its own level"
    );

    let still_absent = ForwardRecorders::process_evidence();
    assert_eq!(
        still_absent.fills_before_install, 1,
        "the fill this process took and reconciliation never saw must be \
         counted, not treated as an absence of trading"
    );
    assert_eq!(
        still_absent.verdict(),
        ForwardRecordingVerdict::NotInstalled
    );

    // ── 3. Attaching afterwards does not repair the past. ──
    let recorders = ForwardRecorders::install();
    let late = recorders.evidence();
    assert!(late.installed, "it is attached now");
    assert_eq!(late.deployments, 0);
    assert_eq!(
        late.verdict(),
        ForwardRecordingVerdict::AttachedLate { fills: 1 },
        "a late attachment must not erase a fill already taken — the fill \
         stream has no replay"
    );
    assert_eq!(
        ForwardRecorders::require_recording(),
        Err(ForwardRecordingGap::AttachedLate { fills: 1 }),
        "the guard keeps reporting the gap after the registry arrives"
    );
    // The registry must be able to state this without the caller assembling it.
    // A startup line that logs `deployment_count()` reads `0` here and reads `0`
    // in state 1 as well, so the one number a binary is most likely to reach for
    // is the one number that cannot tell "not attached" from "nothing deployed".
    assert_eq!(
        recorders.verdict(),
        late.verdict(),
        "the verdict is reachable from the registry itself, not only by way of evidence()"
    );

    // ── 4. "Attached, nothing deployed" is a fifth fact again. ──
    //
    // Shown here against a clean registry, because the one above is
    // permanently in `AttachedLate` and that is the point.
    let clean = reconciliation::ForwardRecordingEvidence {
        installed: true,
        deployments: 0,
        fills_before_install: 0,
        fills_seen: 3,
        fills_attributed: 0,
        fills_unattributed: 3,
        recorder_failures: 0,
    };
    assert_eq!(
        clean.verdict(),
        ForwardRecordingVerdict::NoDeployment { fills_seen: 3 },
        "three manual paper fills and no deployment is not a strategy that \
         failed to trade — the forward clock has not started"
    );
    let deployed = reconciliation::ForwardRecordingEvidence {
        deployments: 1,
        fills_attributed: 3,
        fills_unattributed: 0,
        ..clean
    };
    assert_ne!(
        clean.verdict(),
        deployed.verdict(),
        "\"nothing is deployed\" and \"a deployment traded\" must never compare equal"
    );

    // ── And a deployment made now inherits the gap, rather than hiding it. ──
    let halts = Arc::new(risk::StrategyHaltRegistry::new());
    let _recorder = recorders
        .deploy(spec(), dec("10000"), halts)
        .expect("a valid paper deployment");
    assert_eq!(
        recorders.evidence().verdict(),
        ForwardRecordingVerdict::AttachedLate { fills: 1 },
        "deploying a strategy into a process that already lost a fill does not \
         make its forward record complete"
    );
}
