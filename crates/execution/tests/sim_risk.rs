//! Set U U-2.4 acceptance: the production risk gate in front of a backtest.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use execution::paper::{PaperEngineConfig, SimulatorSet};
use execution::sim_risk::{Refusal, SimulatedRisk};
use execution::sim_venue::{FillPolicy, SimBar, SimulatedVenue};
use execution::Venue;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

const C: AssetClass = AssetClass::CryptoSpotCex;
const BTC: &str = "BTC-USD";

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap() // 2026-01-01 00:00
}

fn flat(at: DateTime<Utc>, p: Decimal) -> SimBar {
    SimBar {
        instrument_id: BTC.into(),
        open_time: at,
        close_time: at + Duration::minutes(1),
        open: p,
        high: p,
        low: p,
        close: p,
    }
}

fn mkt(side: Side, qty: Decimal) -> OrderIntent {
    OrderIntent::new(
        BTC,
        side,
        OrderType::Market,
        Size::from_decimal(qty),
        None,
        None,
    )
    .with_strategy_for_test()
}

trait ForTest {
    fn with_strategy_for_test(self) -> Self;
}
impl ForTest for OrderIntent {
    fn with_strategy_for_test(mut self) -> Self {
        self.strategy_id = Some("s1".into());
        self
    }
}

fn setup(limits: GlobalRiskLimits) -> (SimulatedVenue, SimulatedRisk) {
    let venue = SimulatedVenue::new(
        &HashMap::new(),
        SimulatorSet::realistic(),
        PaperEngineConfig::default(),
        t0(),
        FillPolicy::NextBarOpen,
    );
    let mut instruments = BTreeMap::new();
    instruments.insert(
        BTC.to_string(),
        InstrumentLimits {
            tick_size: dec!(0.01),
            lot_size: Decimal::ZERO,
        },
    );
    (
        venue,
        SimulatedRisk::new(limits, instruments, Some("s1".into())),
    )
}

#[test]
fn orders_over_the_position_cap_are_rejected_and_reported_per_trade() {
    let (v, gate) = setup(GlobalRiskLimits {
        max_position: dec!(1),
        max_orders_per_second: 1_000,
        max_orders_per_minute: 1_000,
        ..GlobalRiskLimits::default()
    });
    let mut at = t0();
    v.on_bar(&flat(at, dec!(50_000)));
    let mut refused = 0;
    // A strategy that keeps buying half a coin every bar.
    for _ in 0..6 {
        match gate.submit(&v, C, &mkt(Side::Buy, dec!(0.5))) {
            Ok(_) => {}
            Err(Refusal::Risk(RiskRejection::PositionLimitExceeded { .. })) => refused += 1,
            Err(e) => panic!("unexpected refusal {e:?}"),
        }
        at += Duration::minutes(1);
        v.on_bar(&flat(at, dec!(50_000)));
    }
    let held = v.positions(C)[0].quantity;
    assert_eq!(held, dec!(1), "held at the cap, never over it");
    assert_eq!(refused, 4);
    let log = gate.rejections();
    assert_eq!(log.len(), 4, "each refused order is its own event");
    assert!(log
        .iter()
        .all(|r| matches!(r.rejection, RiskRejection::PositionLimitExceeded { .. })));
    assert!(
        log.windows(2).all(|w| w[0].at < w[1].at),
        "stamped in simulated time"
    );

    // At the cap, selling out is a reduction, and the gate lets it through.
    gate.submit(&v, C, &mkt(Side::Sell, dec!(1)))
        .expect("an exit at the cap passes");
}

#[test]
fn a_daily_loss_breach_halts_the_run_for_the_rest_of_the_simulated_day() {
    let (v, gate) = setup(GlobalRiskLimits {
        max_daily_loss_usd: dec!(1_000),
        ..GlobalRiskLimits::default()
    });
    let morning = t0() + Duration::hours(9);
    v.on_bar(&flat(morning, dec!(50_000)));
    gate.submit(&v, C, &mkt(Side::Buy, dec!(1))).unwrap();
    v.on_bar(&flat(morning + Duration::minutes(1), dec!(50_000)));
    // A 4% drop: $2 000 down on the day.
    v.on_bar(&flat(morning + Duration::minutes(2), dec!(48_000)));
    let err = gate.submit(&v, C, &mkt(Side::Buy, dec!(0.1))).unwrap_err();
    assert!(matches!(
        err,
        Refusal::Risk(RiskRejection::DailyLossLimitExceeded { .. })
    ));

    // The price recovers in the afternoon: the loss is gone, the halt is not.
    v.on_bar(&flat(morning + Duration::hours(6), dec!(50_500)));
    let err = gate.submit(&v, C, &mkt(Side::Buy, dec!(0.1))).unwrap_err();
    assert!(
        matches!(err, Refusal::Risk(RiskRejection::StrategyHalted { .. })),
        "{err:?}"
    );

    // Next simulated day: trading resumes.
    v.on_bar(&flat(
        t0() + Duration::days(1) + Duration::hours(9),
        dec!(50_500),
    ));
    gate.submit(&v, C, &mkt(Side::Buy, dec!(0.1)))
        .expect("a new day lifts the halt");
    assert_eq!(gate.rejections().len(), 2);
}

#[test]
fn rate_limits_count_simulated_time_not_the_wall() {
    // Two minutes of one-a-minute orders replayed in microseconds: the live
    // gate's wall-clock counter would see a burst; this one sees two orders.
    let (v, gate) = setup(GlobalRiskLimits {
        max_orders_per_second: 1,
        max_orders_per_minute: 2,
        ..GlobalRiskLimits::default()
    });
    let mut at = t0();
    for i in 0..4 {
        v.on_bar(&flat(at, dec!(50_000)));
        let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
        gate.submit(&v, C, &mkt(side, dec!(0.1)))
            .unwrap_or_else(|e| panic!("order {i} refused: {e:?}"));
        at += Duration::minutes(1);
    }
    // Two in the same simulated second are a burst.
    v.on_bar(&flat(at, dec!(50_000)));
    gate.submit(&v, C, &mkt(Side::Buy, dec!(0.1))).unwrap();
    let err = gate.submit(&v, C, &mkt(Side::Sell, dec!(0.1))).unwrap_err();
    assert!(matches!(
        err,
        Refusal::Risk(RiskRejection::RateLimitPerSecondExceeded { .. })
    ));
}

#[test]
fn the_runs_kill_switch_stops_the_run_and_nothing_else() {
    let (v, gate) = setup(GlobalRiskLimits::default());
    let (_, other) = setup(GlobalRiskLimits::default());
    v.on_bar(&flat(t0(), dec!(50_000)));
    gate.kill_switch().trip();
    let err = gate.submit(&v, C, &mkt(Side::Buy, dec!(0.1))).unwrap_err();
    assert!(matches!(
        err,
        Refusal::Risk(RiskRejection::KillSwitchActive)
    ));
    assert!(!other.kill_switch().is_active());
}

#[test]
fn an_instrument_with_no_metadata_is_refused_as_live_refuses_it() {
    let (v, gate) = setup(GlobalRiskLimits::default());
    v.on_bar(&flat(t0(), dec!(50_000)));
    let mut eth = mkt(Side::Buy, dec!(1));
    eth.instrument_id = "ETH-USD".into();
    v.engine()
        .on_mark("ETH-USD", Price::from_decimal(dec!(3_000)));
    let err = gate.submit(&v, C, &eth).unwrap_err();
    assert!(matches!(
        err,
        Refusal::Risk(RiskRejection::InstrumentMetadataUnavailable { .. })
    ));
}
