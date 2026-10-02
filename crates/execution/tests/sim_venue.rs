//! Set U Phase 2 acceptance: the simulated venue, the paper venue behind the
//! same port, and the order vocabulary in both (U-2.2, U-2.3, U-2.5).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::broker::BrokerOrderState;
use execution::paper::{
    ClobFillSimulator, PaperEngineConfig, PaperTradingEngine, PaperVenue, SimulatorSet,
};
use execution::sim_venue::{FillPolicy, SimBar, SimulatedVenue};
use execution::Venue;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

const C: AssetClass = AssetClass::CryptoSpotCex;
const BTC: &str = "BTC-USD";

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap() // 2026-01-01
}

fn bar(i: i64, o: Decimal, h: Decimal, l: Decimal, c: Decimal) -> SimBar {
    SimBar {
        instrument_id: BTC.into(),
        open_time: t0() + Duration::minutes(i),
        close_time: t0() + Duration::minutes(i + 1),
        open: o,
        high: h,
        low: l,
        close: c,
    }
}

fn flat(i: i64, p: Decimal) -> SimBar {
    bar(i, p, p, p, p)
}

fn venue(policy: FillPolicy) -> SimulatedVenue {
    venue_with(SimulatorSet::realistic(), policy)
}

fn venue_with(sims: SimulatorSet, policy: FillPolicy) -> SimulatedVenue {
    SimulatedVenue::new(
        &HashMap::new(),
        sims,
        PaperEngineConfig::default(),
        t0(),
        policy,
    )
}

fn intent(side: Side, t: OrderType, qty: Decimal) -> OrderIntent {
    let mut i = OrderIntent::new(BTC, side, t, Size::from_decimal(qty), None, None);
    i.order_type = t;
    i
}

fn mkt(side: Side, qty: Decimal) -> OrderIntent {
    intent(side, OrderType::Market, qty)
}

fn lim(side: Side, qty: Decimal, p: Decimal) -> OrderIntent {
    let mut i = intent(side, OrderType::Limit, qty);
    i.limit_price = Some(Price::from_decimal(p));
    i
}

fn held(v: &dyn Venue) -> Decimal {
    v.positions(C).first().map_or(Decimal::ZERO, |p| p.quantity)
}

// ── U-2.2 ────────────────────────────────────────────────────────────────────

#[test]
fn a_market_order_at_bar_t_fills_at_bar_t_plus_1s_open_plus_costs() {
    let v = venue(FillPolicy::NextBarOpen);
    v.on_bar(&bar(
        0,
        dec!(50_000),
        dec!(50_100),
        dec!(49_900),
        dec!(50_050),
    ));
    let key = v.submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
    // Decided at bar 0's close: nothing has happened yet, not even arrival.
    assert!(v.order(key).is_none());
    assert_eq!(held(&v), Decimal::ZERO);

    let r = v.on_bar(&bar(
        1,
        dec!(50_200),
        dec!(50_400),
        dec!(50_150),
        dec!(50_300),
    ));
    let s = v.order(key).unwrap();
    assert_eq!(s.state, BrokerOrderState::Filled);
    let px = s.avg_fill_price.unwrap().inner();
    assert!(
        px > dec!(50_200),
        "paid the open plus half-spread and slippage, got {px}"
    );
    assert!(px < dec!(50_300), "not the close, got {px}");
    assert!(
        s.commission.is_some_and(|c| c > Decimal::ZERO),
        "fees charged"
    );
    assert_eq!(r.updates.len(), 1);
    assert!(r.rejected.is_empty());
}

#[test]
fn a_limit_touched_only_by_bar_t_plus_3s_low_fills_on_bar_t_plus_3() {
    let v = venue(FillPolicy::NextBarOpen);
    v.on_bar(&flat(0, dec!(50_000)));
    let key = v.submit(C, &lim(Side::Buy, dec!(1), dec!(49_000))).unwrap();
    v.on_bar(&bar(
        1,
        dec!(50_000),
        dec!(50_200),
        dec!(49_500),
        dec!(49_800),
    ));
    v.on_bar(&bar(
        2,
        dec!(49_800),
        dec!(49_900),
        dec!(49_100),
        dec!(49_300),
    ));
    assert_eq!(v.order(key).unwrap().state, BrokerOrderState::New);
    let r = v.on_bar(&bar(
        3,
        dec!(49_300),
        dec!(49_400),
        dec!(48_950),
        dec!(49_200),
    ));
    let s = v.order(key).unwrap();
    assert_eq!(s.state, BrokerOrderState::Filled);
    assert_eq!(s.avg_fill_price.unwrap().inner(), dec!(49_000));
    assert_eq!(v.now(), t0() + Duration::minutes(4));
    assert_eq!(r.updates.len(), 1);
}

#[test]
fn a_bar_spanning_both_stop_and_target_exits_at_the_stop() {
    let v = venue(FillPolicy::NextBarOpen);
    v.on_bar(&flat(0, dec!(50_000)));
    v.submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
    v.on_bar(&flat(1, dec!(50_000)));
    assert_eq!(held(&v), dec!(1));

    let mut stop = intent(Side::Sell, OrderType::Stop, dec!(1))
        .reduce_only()
        .in_oco("b");
    stop.stop_price = Some(Price::from_decimal(dec!(49_000)));
    let target = lim(Side::Sell, dec!(1), dec!(51_000))
        .reduce_only()
        .in_oco("b");
    let (sk, tk) = (v.submit(C, &stop).unwrap(), v.submit(C, &target).unwrap());
    v.on_bar(&flat(2, dec!(50_000))); // both arrive and rest
    v.on_bar(&bar(
        3,
        dec!(50_000),
        dec!(52_000),
        dec!(48_000),
        dec!(50_500),
    ));

    let s = v.order(sk).unwrap();
    assert_eq!(
        s.state,
        BrokerOrderState::Filled,
        "the stop wins the ambiguous bar"
    );
    assert_eq!(v.order(tk).unwrap().state, BrokerOrderState::Cancelled);
    assert!(s.avg_fill_price.unwrap().inner() <= dec!(49_000));
    assert_eq!(held(&v), Decimal::ZERO);
}

#[test]
fn current_close_fills_at_once_and_is_declared_optimistic() {
    let v = venue(FillPolicy::CurrentClose);
    assert!(v.policy().is_optimistic());
    assert!(!FillPolicy::NextBarOpen.is_optimistic());
    v.on_bar(&flat(0, dec!(50_000)));
    let key = v.submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
    assert_eq!(v.order(key).unwrap().state, BrokerOrderState::Filled);
    assert_eq!(v.take_updates().len(), 1);
}

#[test]
fn an_order_refused_on_arrival_is_reported_not_dropped() {
    let v = venue(FillPolicy::NextBarOpen);
    v.on_bar(&flat(0, dec!(50_000)));
    // Far more than the account's cash.
    let key = v.submit(C, &mkt(Side::Buy, dec!(1_000))).unwrap();
    let r = v.on_bar(&flat(1, dec!(50_000)));
    assert_eq!(r.rejected.len(), 1);
    assert_eq!(r.rejected[0].key, key);
    assert_eq!(v.order(key).unwrap().state, BrokerOrderState::Rejected);
}

#[test]
fn the_same_run_twice_is_the_same_run() {
    let run = || {
        let v = venue(FillPolicy::NextBarOpen);
        let mut ids = Vec::new();
        for i in 0..20 {
            let p = dec!(50_000) + Decimal::from(i * 37 % 11) * dec!(25);
            v.on_bar(&bar(i, p, p + dec!(40), p - dec!(40), p + dec!(10)));
            let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
            let k = v.submit(C, &mkt(side, dec!(0.5))).unwrap();
            ids.push(k);
        }
        v.on_bar(&flat(20, dec!(50_000)));
        let orders: Vec<String> = ids
            .iter()
            .filter_map(|k| v.order(*k))
            .map(|s| {
                format!(
                    "{} {:?} {:?}",
                    s.broker_order_id, s.avg_fill_price, s.commission
                )
            })
            .collect();
        (orders, v.snapshot(C).equity)
    };
    let (a, b) = (run(), run());
    assert_eq!(a.0.len(), 20);
    assert!(a.0[0].starts_with("sim-000000000001"), "{}", a.0[0]);
    assert_eq!(a, b);
}

// ── the oracle, against this venue ──────────────────────────────────────────

/// Synthetic 1m bars: a reproducible pseudo-Gaussian walk at ~`sigma_bps` a
/// bar, each bar opening at the previous close.
fn walk(n: usize, sigma_bps: f64) -> Vec<SimBar> {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((s >> 11) as f64) / ((1u64 << 53) as f64)
    };
    let mut p = 50_000.0f64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (u1, u2) = (next().max(1e-12), next());
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        let c = p * (1.0 + z * sigma_bps / 10_000.0);
        let d = |x: f64| Decimal::from_f64_retain((x * 100.0).round() / 100.0).unwrap();
        out.push(bar(i as i64, d(p), d(p.max(c)), d(p.min(c)), d(c)));
        p = c;
    }
    out
}

/// A long/flat strategy that knows each next bar's close, run through the
/// venue. Returns net P&L.
fn run_oracle(v: &SimulatedVenue, bars: &[SimBar]) -> Decimal {
    let start = v.snapshot(C).equity;
    for (i, b) in bars.iter().enumerate() {
        v.on_bar(b);
        let Some(next) = bars.get(i + 1) else { break };
        let want_long = next.close > next.open;
        let long = held(v) > Decimal::ZERO;
        if want_long && !long {
            v.submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
        } else if !want_long && long {
            v.submit(C, &mkt(Side::Sell, held(v))).unwrap();
        }
    }
    v.snapshot(C).equity - start
}

#[test]
fn the_oracle_loses_money_at_one_minute_through_this_venue() {
    // Perfect foresight at 1m earns the mean absolute move (~8 bps at σ = 10
    // bps) and pays a round trip of ~50 bps plus spread. If this venue ever
    // makes it money, the venue is under-charging and every backtest through it
    // is suspect (EXTERNAL-REVIEW-2026-09 §3.4).
    let bars = walk(600, 10.0);
    let pnl = run_oracle(&venue(FillPolicy::NextBarOpen), &bars);
    assert!(
        pnl < Decimal::ZERO,
        "the oracle made {pnl} — the venue is under-charging"
    );

    // And the test is load-bearing: with costs removed the same oracle wins.
    let mut free = SimulatorSet::realistic();
    free.clob_overrides.insert(
        C,
        ClobFillSimulator {
            half_spread_bps: Decimal::ZERO,
            slippage_ticks: Decimal::ZERO,
            fee_rate: Decimal::ZERO,
            depth_notional: None,
            impact_bps_at_depth: Decimal::ZERO,
            max_impact_bps: Decimal::ZERO,
            ..ClobFillSimulator::default()
        },
    );
    let free_pnl = run_oracle(&venue_with(free, FillPolicy::NextBarOpen), &bars);
    assert!(free_pnl > Decimal::ZERO, "costless oracle made {free_pnl}");
}

// ── U-2.3: every order type, in both venues, against the same script ────────

/// Drive a script of prices through a venue. `submit_after` is the index after
/// which the order is placed. Returns the index of the price at which it
/// filled, if it did.
trait Driver {
    fn venue(&self) -> &dyn Venue;
    fn price(&self, i: usize, p: Decimal);
}

struct Paper(PaperVenue);
impl Driver for Paper {
    fn venue(&self) -> &dyn Venue {
        &self.0
    }
    fn price(&self, _: usize, p: Decimal) {
        self.0.engine().on_mark(BTC, Price::from_decimal(p));
    }
}

struct Sim(SimulatedVenue);
impl Driver for Sim {
    fn venue(&self) -> &dyn Venue {
        &self.0
    }
    fn price(&self, i: usize, p: Decimal) {
        self.0.on_bar(&flat(i as i64, p));
    }
}

fn filled_at(d: &dyn Driver, script: &[Decimal], order: &OrderIntent) -> Option<usize> {
    // Open a long first so sells have something to sell.
    d.price(0, script[0]);
    d.venue().submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
    d.price(1, script[0]);
    let key = d.venue().submit(C, order).unwrap();
    for (i, p) in script.iter().enumerate() {
        d.price(i + 2, *p);
        if d.venue()
            .order(key)
            .is_some_and(|s| s.state == BrokerOrderState::Filled)
        {
            return Some(i);
        }
    }
    None
}

fn both() -> [Box<dyn Driver>; 2] {
    [
        Box::new(Paper(PaperVenue::new(Arc::new(PaperTradingEngine::new())))),
        Box::new(Sim(venue(FillPolicy::NextBarOpen))),
    ]
}

#[test]
fn every_order_type_fills_on_the_same_price_in_paper_and_in_simulation() {
    let script = [
        dec!(50_000),
        dec!(50_000),
        dec!(51_000),
        dec!(53_000),
        dec!(51_900),
        dec!(48_800),
        dec!(48_000),
        dec!(48_600),
    ];
    let p = |v| Some(Price::from_decimal(v));
    let mut stop = intent(Side::Sell, OrderType::Stop, dec!(1));
    stop.stop_price = p(dec!(49_000));
    let mut stop_limit = intent(Side::Sell, OrderType::StopLimit, dec!(1));
    stop_limit.stop_price = p(dec!(49_000));
    stop_limit.limit_price = p(dec!(48_500));
    let mut trailing = intent(Side::Sell, OrderType::TrailingStop, dec!(1));
    trailing.trail = p(dec!(1_000));
    let cases: [(&str, OrderIntent, Option<usize>); 5] = [
        ("market", mkt(Side::Sell, dec!(1)), Some(0)),
        ("limit", lim(Side::Sell, dec!(1), dec!(52_000)), Some(3)),
        // Peak 53 000 → trail at 52 000 → 51 900 fires it.
        ("trailing_stop", trailing, Some(4)),
        ("stop", stop, Some(5)),
        // Triggered at 48 800 (limit 48 500 holds: 48 800 ≥ 48 500, fills),
        ("stop_limit", stop_limit, Some(5)),
    ];
    for (name, order, want) in cases {
        for d in both() {
            let mut o = order.clone();
            o.idempotency_key = uuid::Uuid::new_v4();
            assert_eq!(filled_at(d.as_ref(), &script, &o), want, "{name}");
        }
    }
}

#[test]
fn a_reduce_only_order_never_increases_exposure_in_either_venue() {
    for d in both() {
        d.price(0, dec!(50_000));
        d.venue().submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
        d.price(1, dec!(50_000));
        let k = d.venue().submit(C, &mkt(Side::Sell, dec!(5)).reduce_only());
        d.price(2, dec!(50_000));
        // Paper refuses or fills now, the simulator on arrival; either way the
        // position is closed and never reversed.
        assert!(k.is_ok());
        assert_eq!(held(d.venue()), Decimal::ZERO);
        let again = d.venue().submit(C, &mkt(Side::Sell, dec!(1)).reduce_only());
        d.price(3, dec!(50_000));
        assert_eq!(held(d.venue()), Decimal::ZERO, "flat stays flat");
        if let Ok(k) = again {
            let s = d.venue().order(k).unwrap();
            assert_ne!(s.state, BrokerOrderState::Filled);
        }
    }
}

#[test]
fn a_post_only_limit_that_would_take_is_rejected_in_either_venue() {
    for d in both() {
        d.price(0, dec!(50_000));
        d.price(1, dec!(50_000));
        let r = d
            .venue()
            .submit(C, &lim(Side::Buy, dec!(1), dec!(50_500)).post_only());
        d.price(2, dec!(50_000));
        let state = match r {
            Err(_) => BrokerOrderState::Rejected,
            Ok(k) => d.venue().order(k).unwrap().state,
        };
        assert_eq!(state, BrokerOrderState::Rejected);
        assert_eq!(held(d.venue()), Decimal::ZERO);
    }
}

// ── U-2.5: one port ─────────────────────────────────────────────────────────

/// An environment that holds a venue and never learns which.
struct Environment {
    venue: Box<dyn Venue>,
}

#[test]
fn an_environment_is_built_with_either_venue_without_a_type_change() {
    let envs = [
        Environment {
            venue: Box::new(PaperVenue::new(Arc::new(PaperTradingEngine::new()))),
        },
        Environment {
            venue: Box::new(venue(FillPolicy::NextBarOpen)),
        },
    ];
    for e in &envs {
        assert!(e.venue.open_orders(Some(C)).is_empty());
        assert_eq!(e.venue.snapshot(C).asset_class, C);
    }
}

// ── U-2.6: one fee schedule, maker and taker, in both venues ────────────────

#[test]
fn a_resting_limit_pays_maker_and_a_market_order_pays_taker_in_either_venue() {
    let fees = domain::fees::paper_crypto();
    let rate = |bps: f64| Decimal::from_f64_retain(bps).unwrap() / dec!(10000);
    for d in both() {
        d.price(0, dec!(50_000));
        let buy = d.venue().submit(C, &mkt(Side::Buy, dec!(1))).unwrap();
        d.price(1, dec!(50_000));
        let s = d.venue().order(buy).unwrap();
        let px = s.avg_fill_price.unwrap().inner();
        assert_eq!(
            s.commission,
            Some(px * rate(fees.taker_bps)),
            "market order: taker"
        );

        let sell = d
            .venue()
            .submit(C, &lim(Side::Sell, dec!(1), dec!(52_000)))
            .unwrap();
        d.price(2, dec!(50_000)); // rests
        d.price(3, dec!(52_500)); // the market comes to it
        let s = d.venue().order(sell).unwrap();
        assert_eq!(s.state, BrokerOrderState::Filled);
        assert_eq!(
            s.commission,
            Some(dec!(52_000) * rate(fees.maker_bps)),
            "resting limit: maker"
        );
    }
}

#[test]
fn a_bar_range_fill_of_a_resting_limit_is_a_maker_fill() {
    let fees = domain::fees::paper_crypto();
    let v = venue(FillPolicy::NextBarOpen);
    v.on_bar(&flat(0, dec!(50_000)));
    let k = v.submit(C, &lim(Side::Buy, dec!(1), dec!(49_000))).unwrap();
    v.on_bar(&flat(1, dec!(50_000)));
    v.on_bar(&bar(
        2,
        dec!(50_000),
        dec!(50_100),
        dec!(48_900),
        dec!(49_500),
    ));
    let s = v.order(k).unwrap();
    assert_eq!(s.state, BrokerOrderState::Filled);
    let maker = Decimal::from_f64_retain(fees.maker_bps).unwrap() / dec!(10000);
    assert_eq!(s.commission, Some(dec!(49_000) * maker));
}

#[test]
fn the_paper_venue_charges_the_shared_schedule() {
    let fees = domain::fees::paper_crypto();
    let sims = SimulatorSet::realistic();
    let crypto = &sims.clob_overrides[&C];
    let rate = |bps: f64| Decimal::from_f64_retain(bps).unwrap() / dec!(10000);
    assert_eq!(crypto.fee_rate, rate(fees.taker_bps));
    assert_eq!(crypto.maker_fee_rate, Some(rate(fees.maker_bps)));
    assert_eq!(
        crypto.half_spread_bps,
        Decimal::from_f64_retain(fees.half_spread_bps).unwrap()
    );
}
