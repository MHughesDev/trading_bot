//! P2-T01 acceptance tests: LaneKey-based acquire/release with 120-second warm period.
//!
//! Two acquires + one release keeps lane live.
//! Final release schedules teardown; an acquire within 120 s cancels it.
//! An acquire after the warm window starts a fresh lane.

use std::sync::Arc;

use demand_manager::{DemandRegistry, LaneKey, NoopPipelineFactory};
use domain::{AssetClass, DataType, SupportedVenue};

fn btc_key() -> LaneKey {
    LaneKey::new(
        SupportedVenue::Kraken,
        AssetClass::CryptoSpotCex,
        DataType::MarketOhlcv,
        "BTC-USD",
    )
}

fn registry() -> DemandRegistry {
    DemandRegistry::new(Arc::new(NoopPipelineFactory))
}

#[test]
fn two_acquires_one_release_keeps_lane_live() {
    let reg = registry();
    let key = btc_key();

    reg.acquire(key.clone());
    reg.acquire(key.clone());
    reg.release(key.clone());

    assert_eq!(reg.acquire_count(&key), 1, "one consumer still active");
}

#[tokio::test]
async fn final_release_reduces_count_to_zero() {
    let reg = registry();
    let key = btc_key();

    reg.acquire(key.clone());
    reg.release(key.clone());
    // Give the spawned teardown task a chance to run.
    tokio::task::yield_now().await;

    assert_eq!(
        reg.acquire_count(&key),
        0,
        "count should be 0 after final release"
    );
}

#[tokio::test]
async fn acquire_within_warm_period_cancels_teardown() {
    tokio::time::pause();

    let reg = Arc::new(registry());
    let key = btc_key();

    reg.acquire(key.clone());
    reg.release(key.clone());

    // Advance less than the 120-second warm period.
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    // Yield so the spawned teardown task has a chance to observe cancellation.
    tokio::task::yield_now().await;

    // Re-acquire within the warm window — should cancel the pending teardown.
    reg.acquire(key.clone());
    assert_eq!(reg.acquire_count(&key), 1, "re-acquired within warm period");

    // Advance past what would have been the original teardown time.
    tokio::time::advance(std::time::Duration::from_secs(120)).await;
    tokio::task::yield_now().await;

    // Lane should still be alive (not torn down by the cancelled timer).
    assert_eq!(
        reg.acquire_count(&key),
        1,
        "lane still live after cancelled teardown"
    );
}

#[tokio::test]
async fn acquire_after_warm_period_starts_fresh_lane() {
    tokio::time::pause();

    let reg = registry();
    let key = btc_key();

    reg.acquire(key.clone());
    reg.release(key.clone());

    // Advance past the full 120-second warm period.
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    tokio::task::yield_now().await;

    // Acquire after the warm window — should start a fresh lane.
    reg.acquire(key.clone());
    assert_eq!(
        reg.acquire_count(&key),
        1,
        "fresh lane started after warm period"
    );
}

#[tokio::test]
async fn different_lane_keys_are_independent() {
    let reg = registry();
    let btc = LaneKey::new(
        SupportedVenue::Kraken,
        AssetClass::CryptoSpotCex,
        DataType::MarketOhlcv,
        "BTC-USD",
    );
    let eth = LaneKey::new(
        SupportedVenue::Kraken,
        AssetClass::CryptoSpotCex,
        DataType::MarketOhlcv,
        "ETH-USD",
    );

    reg.acquire(btc.clone());
    reg.acquire(eth.clone());
    reg.release(btc.clone());
    tokio::task::yield_now().await;

    assert_eq!(reg.acquire_count(&btc), 0);
    assert_eq!(reg.acquire_count(&eth), 1);
}
