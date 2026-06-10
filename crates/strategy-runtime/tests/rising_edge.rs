//! P3-T07 acceptance tests — rising-edge execution with idempotency key.

use uuid::Uuid;

use strategy_runtime::automation::edge::{IdempotencyKey, RisingEdgeTracker};

fn key(automation_id: Uuid, instrument: &str, stage: &str, epoch: u64) -> IdempotencyKey {
    IdempotencyKey {
        automation_id,
        instrument_id: instrument.into(),
        stage_id: stage.into(),
        signal_epoch: epoch,
    }
}

/// A condition staying true across N evaluations fires exactly once.
#[test]
fn condition_staying_true_emits_one_order() {
    let id = Uuid::new_v4();
    let mut tracker = RisingEdgeTracker::new();

    // First true after false-initial → rising edge → fire.
    assert!(tracker.should_fire(key(id, "BTC", "s1", 1), true));

    // Stays true across subsequent evaluations with the same epoch → no-op.
    for _ in 0..5 {
        assert!(!tracker.should_fire(key(id, "BTC", "s1", 1), true));
    }
}

/// The condition going false then true again emits a second order with a new epoch.
#[test]
fn second_rising_edge_emits_second_order() {
    let id = Uuid::new_v4();
    let mut tracker = RisingEdgeTracker::new();

    // First rising edge.
    assert!(tracker.should_fire(key(id, "ETH", "s1", 1), true));

    // Condition falls false.
    assert!(!tracker.should_fire(key(id, "ETH", "s1", 1), false));

    // Condition rises again — new epoch (epoch = 2).
    assert!(tracker.should_fire(key(id, "ETH", "s1", 2), true));
}

/// A replayed signal with a seen key emits nothing.
#[test]
fn replayed_signal_with_seen_key_emits_nothing() {
    let id = Uuid::new_v4();
    let mut tracker = RisingEdgeTracker::new();

    let k = key(id, "SOL", "s1", 42);
    // First call fires.
    assert!(tracker.should_fire(k.clone(), true));

    // State falls so the next rising edge fires with the same epoch.
    // This simulates a replay where the same (epoch=42) signal is redelivered.
    // State reset: set to false first.
    tracker.should_fire(key(id, "SOL", "s1", 42), false);

    // Redelivery of the same epoch → should NOT fire (key already seen).
    assert!(!tracker.should_fire(k, true));
}

/// False condition never fires regardless of epoch.
#[test]
fn false_condition_never_fires() {
    let id = Uuid::new_v4();
    let mut tracker = RisingEdgeTracker::new();

    for epoch in 0..10_u64 {
        assert!(!tracker.should_fire(key(id, "AAPL", "s1", epoch), false));
    }
}
