//! An observer that blows up must not take the fill down with it.
//!
//! `FillObserver::on_fill` returns `()`, so an observer cannot *report* a
//! failure — but it can still panic, and the shape of that failure decides
//! whether the platform ends up with a fill recorded by nobody and no trace of
//! why. The rule this test fixes: the surviving observers still receive the
//! fill, and the lost delivery is counted rather than swallowed.
//!
//! Own test binary: it registers a deliberately panicking observer, which
//! would pollute every other test sharing the process-wide registry.
//!
//! Note for anyone reading a production stack trace: `profile.release` sets
//! `panic = "abort"`, so in a release build a panicking observer kills the
//! process instead of being caught here. That is also loud, which is the
//! property that matters — the one outcome that is not allowed is quiet.

use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use domain::money::Price;
use execution::fills::{self, FillEvent, FillObserver};
use rust_decimal::Decimal;
use uuid::Uuid;

struct Exploding;
impl FillObserver for Exploding {
    fn on_fill(&self, _fill: &FillEvent) {
        panic!("this observer is broken");
    }
}

#[derive(Default)]
struct Counting(AtomicUsize);
impl FillObserver for Counting {
    fn on_fill(&self, _fill: &FillEvent) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn a_fill() -> FillEvent {
    FillEvent {
        idempotency_key: Uuid::new_v4(),
        broker_order_id: "panic-1".to_owned(),
        filled_qty: Decimal::from_str("1").unwrap(),
        fill_price: Price::from_str("50000").unwrap(),
        commission: Decimal::ZERO,
        filled_at: Utc::now(),
    }
}

#[test]
fn a_panicking_observer_is_isolated_and_counted() {
    let healthy = Arc::new(Counting::default());
    // Registration order puts the broken one first: a panic must not prevent
    // the observers behind it from ever seeing the fill.
    fills::register_observer(Arc::new(Exploding) as Arc<dyn FillObserver>);
    fills::register_observer(Arc::clone(&healthy) as Arc<dyn FillObserver>);

    // The panic message from the broken observer is expected; keep it out of
    // the test output so a passing run does not read like a failing one.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    fills::publish(&a_fill());
    std::panic::set_hook(previous);

    assert_eq!(
        healthy.0.load(Ordering::SeqCst),
        1,
        "a broken observer must not stop the fill reaching the others"
    );

    let evidence = fills::evidence();
    assert_eq!(evidence.fills_published, 1);
    assert_eq!(
        evidence.observer_failures, 1,
        "the delivery that was lost must be counted, not swallowed"
    );
    assert_eq!(
        evidence.evidence_gap(),
        1,
        "a lost delivery is an evidence gap even though the fill was published"
    );
    assert!(
        fills::require_recording().is_err(),
        "a process that has already lost a delivery cannot claim a clean \
         forward record"
    );
}
