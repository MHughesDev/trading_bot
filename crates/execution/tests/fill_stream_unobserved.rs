//! An absent observer is not zero fills.
//!
//! Gate 15 reads a forward record. An empty one has two causes that look
//! identical from outside the process and mean opposite things: *nothing
//! traded*, and *things traded and nobody wrote them down*. This test pins the
//! difference, which is the only reason the counters in `execution::fills`
//! exist.
//!
//! It lives in its own test binary on purpose. The observer registry is
//! process-wide, so a test that asserts "nothing is listening" cannot share a
//! process with one that registers something.

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use domain::money::Price;
use execution::fills::{self, FillEvent, FillObserver, FillStreamGap, FillStreamVerdict};
use rust_decimal::Decimal;
use uuid::Uuid;

struct Sink;
impl FillObserver for Sink {
    fn on_fill(&self, _fill: &FillEvent) {}
}

fn a_fill() -> FillEvent {
    FillEvent {
        idempotency_key: Uuid::new_v4(),
        broker_order_id: "unobserved-1".to_owned(),
        filled_qty: Decimal::from_str("0.25").unwrap(),
        fill_price: Price::from_str("50000").unwrap(),
        commission: Decimal::ZERO,
        filled_at: Utc::now(),
    }
}

#[test]
fn a_fill_taken_with_no_observer_is_counted_never_silent() {
    // ── Before anything happens: measuring nothing, not measuring zero. ──
    let quiet = fills::evidence();
    assert_eq!(quiet.observers, 0, "this binary registers nothing up front");
    assert_eq!(quiet.fills_published, 0);
    assert!(!quiet.is_recording());
    assert_eq!(
        quiet.verdict(),
        FillStreamVerdict::NotRecording,
        "no observer and no fills is an absent measurement, not a zero"
    );
    assert!(
        matches!(fills::require_recording(), Err(FillStreamGap::NoObserver)),
        "a process that would trade without recording must fail its startup guard"
    );

    // ── A fill happens with nobody listening. ──
    fills::publish(&a_fill());

    let lost = fills::evidence();
    assert_eq!(lost.fills_published, 1);
    assert_eq!(
        lost.fills_unobserved, 1,
        "a fill nobody received must be counted; a silent no-op here is \
         indistinguishable from a process that never traded"
    );
    assert_eq!(lost.evidence_gap(), 1);
    assert_eq!(lost.verdict(), FillStreamVerdict::EvidenceLost { fills: 1 });

    // The whole point: these two states are not the same fact.
    assert_ne!(
        quiet.verdict(),
        lost.verdict(),
        "\"no fills happened\" and \"fills happened and nobody recorded them\" \
         must never compare equal"
    );

    // ── Attaching an observer afterwards does not repair the past. ──
    // The fill stream has no replay: the fill is in the journal and in no
    // forward record, permanently.
    fills::register_observer(Arc::new(Sink) as Arc<dyn FillObserver>);
    let repaired = fills::evidence();
    assert!(repaired.is_recording(), "an observer is attached now");
    assert_eq!(
        repaired.verdict(),
        FillStreamVerdict::EvidenceLost { fills: 1 },
        "a late registration must not erase a fill already taken unobserved"
    );
    assert!(
        matches!(
            fills::require_recording(),
            Err(FillStreamGap::EvidenceLost { fills: 1 })
        ),
        "the guard keeps reporting the gap after the observer arrives"
    );
}
