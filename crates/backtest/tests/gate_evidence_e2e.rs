//! Gates 3, 4, 10 and 12 judge on real numbers — and refuse to judge on
//! placeholders (SPEC §12.3, HANDOFF §5.3).
//!
//! Eleven of the sixteen gates judged on the ledger; these four judged on
//! nothing, returning `inconclusive` for every candidate. The evaluators were
//! never the problem — they are arithmetic and they were correct. Nothing
//! produced what they read.
//!
//! Every test here is a pair. One half drives the funnel with evidence and
//! asserts the gate renders a **decided** verdict. The other half removes
//! exactly one thing and asserts the gate returns to `inconclusive` **with a
//! stated reason**, because the failure this whole track could have caused is
//! the opposite: a gate given an input that is not evidence, converting an
//! honest `NotComputable` into a confident wrong verdict.

use std::collections::BTreeMap;
use std::sync::Arc;

use backtest::gates::evaluators::{
    gate_10_factor_attribution, gate_12_perturbation, gate_3_cost_sensitivity, gate_4_capacity,
    CapacityInputs, CostRung, FactorInputs, PerturbationInputs,
};
use backtest::gates::{AssetClassGates, GateProfile};
use backtest::run::{
    ComputeCost, InstrumentStat, RunConfig, RunResult, Side, Trade, ENGINE_VERSION,
};
use backtest::suite::{CreateExperimentSpec, SuiteManager};
use backtest::{nulls::NullKind, InMemoryLedger};
use chrono::{Duration, TimeZone, Utc};
use ledger::evidence::TrialEvidence;
use rust_decimal_macros::dec;
use uuid::Uuid;

/// What a run is allowed to have measured. Both knobs are the two the evidence
/// pipeline actually turns on: whether the engine charged anything, and whether
/// anybody measured the instrument's liquidity.
#[derive(Clone, Copy)]
struct Fixture {
    /// Costs charged per trade. Zero is the pre-ADR-P6-12 signature: every fill
    /// the simulator produced was charged zero commission and the gross number
    /// was labelled net.
    costs_per_trade: f64,
    /// Whether the executor measured ADV and volatility from the run's bars.
    liquidity_measured: bool,
}

/// How big this fixture's book is, as a multiple of the original sizing.
///
/// Quantity, cost and P&L move together, so every ratio the other gates read —
/// Gate 3's cost-to-P&L, Gate 12's concentration — is unchanged by it. The one
/// thing it does move is traded notional against ADV, which is the whole of
/// Gate 4's arithmetic.
///
/// At `1.0` — the size this fixture was written at — the book turns over about
/// $40k a day against a $4M ADV, `impact_drag` never grows large enough to
/// matter, and net Sharpe does not halve anywhere in 1×–100×. Gate 4 then
/// correctly reports that capacity lies beyond the range it measured. That is a
/// true statement about a small book, but it is `inconclusive`, so the
/// evidence-rich half of the Gate 4 pair below asserted something its own
/// numbers could not produce.
///
/// `3.0` puts the half-Sharpe point inside the ladder — impact drag scales as
/// size^1.5 while the base AUM it is divided by does not move, so a 3× book is a
/// ~5× drag — and leaves worst-case participation at 1.5%, well under the 5%
/// soft cap, so the gate judges on capacity rather than short-circuiting on
/// participation.
const SIZE: f64 = 3.0;

/// An executor that trades, pays costs, and (optionally) reports what it
/// measured about the instruments it traded.
fn executor(f: Fixture) -> impl Fn(&RunConfig) -> RunResult + Send + Sync + 'static {
    move |cfg: &RunConfig| {
        let start = Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap();
        // A mild, config-dependent edge so the funnel's own gates behave; what
        // matters here is the trade list beneath it.
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        for b in cfg.strategy_ref.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for (k, v) in &cfg.params {
            // `aum_usd` is deliberately excluded. The capacity ladder re-runs
            // this same strategy at seven sizes, and a fixture engine that
            // hashed the size hands every rung a *different edge* — enough to
            // reorder them, so the "curve" is noise wearing the shape of one.
            // This engine is size-blind on purpose, which leaves
            // `capacity::impact_drag` as the only thing shaping the ladder, and
            // that is the thing Gate 4 is actually judging. The measured
            // difference is in the Gate 4 block of
            // `a_strategy_with_evidence_gets_real_verdicts_at_gates_3_4_and_12`.
            if k == "aum_usd" {
                continue;
            }
            for b in k.as_bytes().iter().chain(v.to_string().as_bytes()) {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let unit = |x: u64| ((x >> 11) as f64) / ((1u64 << 53) as f64);
        let drift = if cfg.null_world.is_some() {
            (unit(h) - 0.5) * 0.001
        } else {
            0.0018 + unit(h) * 0.0012
        };
        let mut equity = 100_000.0_f64;
        let curve: Vec<(chrono::DateTime<Utc>, f64)> = (0..60)
            .map(|d: i64| {
                let wobble = unit(h ^ (d as u64 + 3).wrapping_mul(0x9e37_79b9_7f4a_7c15)) - 0.5;
                equity *= 1.0 + drift + wobble * 0.004;
                (start + Duration::days(d), equity)
            })
            .collect();

        // Three instruments, twenty trades, with a deliberate concentration in
        // the first so Gate 12's check has something to measure.
        let names = ["BTC-USD", "ETH-USD", "SOL-USD"];
        let trades: Vec<Trade> = (0..21_i64)
            .map(|i| {
                let symbol = names[(i % 3) as usize];
                let weight = if symbol == "BTC-USD" { 2.0 } else { 1.0 };
                let t = start + Duration::days(i);
                Trade {
                    symbol: symbol.into(),
                    side: Side::Long,
                    entry_time: t,
                    exit_time: t + Duration::hours(5),
                    entry_price: dec!(100),
                    exit_price: dec!(101),
                    qty: rust_decimal::Decimal::from_f64_retain(SIZE * 50.0 * weight).unwrap(),
                    mae: -0.004,
                    mfe: 0.010,
                    holding_period_secs: 18_000,
                    costs_paid: rust_decimal::Decimal::from_f64_retain(
                        SIZE * f.costs_per_trade * weight,
                    )
                    .unwrap(),
                    pnl: rust_decimal::Decimal::from_f64_retain(SIZE * 60.0 * weight).unwrap(),
                }
            })
            .collect();

        let instrument_stats = if f.liquidity_measured {
            names
                .iter()
                .map(|n| InstrumentStat {
                    instrument: (*n).to_string(),
                    numeraire: Some("USD".into()),
                    adv_usd: Some(4_000_000.0),
                    daily_vol: Some(0.035),
                })
                .collect()
        } else {
            Vec::new()
        };

        // Metrics are *computed from the curve*, by the same function every
        // executor on this platform goes through. This fixture originally
        // hand-built a `RunResult` with `MetricSet::empty()` beside a rising
        // equity curve, which is a run that traded and measured nothing: the
        // funnel reads `res.metrics.value(..)` and nothing else, so Gate 1 read
        // a median Sharpe of 0.0000 and stopped the candidate before any gate
        // evidence was gathered. It was right to. `instrument_stats` is the one
        // field `map_sim_result` cannot know — it is what the executor
        // *measured about the venue*, not about the returns — so it is attached
        // afterwards.
        let mut res = backtest::run::executor::map_sim_result(
            cfg,
            curve,
            vec![],
            trades,
            ComputeCost::default(),
            ENGINE_VERSION,
        );
        res.instrument_stats = instrument_stats;
        res
    }
}

/// The real `InMemoryLedger`, plus a reader for the one thing it writes and
/// never reads back.
///
/// `InMemoryLedger` accepts `record_statistic` and keeps the value in a private
/// map with no accessor: on this side of the platform, `mlops.trial_statistic`
/// is write-only. (Postgres has `statistics_async`; the in-process ledger has
/// no counterpart, so a test cannot read a scalar the funnel recorded.)
///
/// This witness is **not** a second write path and not a stub. Every method
/// delegates to the real ledger, `record_statistic` included — so the fixed
/// name list and the finiteness refusal still apply, and the witness only ever
/// sees a value the ledger itself accepted. It observes; it does not permit.
struct WitnessedLedger {
    inner: InMemoryLedger,
    /// `(tenant, trial, name) -> value`, newest write wins, exactly as the
    /// ledger's own map does.
    seen: std::sync::Mutex<BTreeMap<(String, Uuid, String), f64>>,
}

impl WitnessedLedger {
    fn new() -> Self {
        Self { inner: InMemoryLedger::new(), seen: std::sync::Mutex::new(BTreeMap::new()) }
    }

    /// The statistics the funnel recorded against one trial.
    fn statistics(&self, tenant_id: &str, trial_id: Uuid) -> BTreeMap<String, f64> {
        self.seen
            .lock()
            .expect("witness poisoned")
            .iter()
            .filter(|((t, id, _), _)| t == tenant_id && *id == trial_id)
            .map(|((_, _, name), v)| (name.clone(), *v))
            .collect()
    }
}

impl ledger::TrialLedger for WitnessedLedger {
    fn register(
        &self,
        reg: &ledger::Registration<'_>,
    ) -> Result<ledger::TrialTicket, ledger::LedgerError> {
        self.inner.register(reg)
    }
    fn transition(
        &self,
        ticket: &mut ledger::TrialTicket,
        event: ledger::TrialEvent,
    ) -> Result<(), ledger::LedgerError> {
        self.inner.transition(ticket, event)
    }
    fn settle(
        &self,
        ticket: ledger::TrialTicket,
        event: ledger::TrialEvent,
    ) -> Result<(), ledger::LedgerError> {
        self.inner.settle(ticket, event)
    }
    fn prior_trial(&self, tenant_id: &str, config_hash: &str) -> Option<Uuid> {
        self.inner.prior_trial(tenant_id, config_hash)
    }
    fn trial_count(&self, tenant_id: &str) -> usize {
        self.inner.trial_count(tenant_id)
    }
    fn exploration_fraction(&self, tenant_id: &str) -> f64 {
        self.inner.exploration_fraction(tenant_id)
    }
    fn persist_returns(
        &self,
        ticket: &ledger::TrialTicket,
        series: &ledger::neff::ReturnSeries,
    ) -> Result<String, ledger::LedgerError> {
        self.inner.persist_returns(ticket, series)
    }
    fn n_eff(&self, tenant_id: &str) -> Result<ledger::neff::NEff, ledger::LedgerError> {
        self.inner.n_eff(tenant_id)
    }
    fn record_statistic(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        produced_by: &str,
    ) -> Result<(), ledger::LedgerError> {
        // The real ledger first: a name it refuses is never witnessed.
        self.inner.record_statistic(tenant_id, trial_id, name, value, produced_by)?;
        self.seen
            .lock()
            .expect("witness poisoned")
            .insert((tenant_id.to_string(), trial_id, name.to_string()), value);
        Ok(())
    }
    fn record_gate_evidence(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        evidence: &TrialEvidence,
    ) -> Result<(), ledger::LedgerError> {
        self.inner.record_gate_evidence(tenant_id, trial_id, evidence)
    }
    fn claim_sealed_holdout(
        &self,
        tenant_id: &str,
        lineage: &str,
        requested_by: &str,
    ) -> Result<ledger::holdout::HoldoutClaim, ledger::LedgerError> {
        self.inner.claim_sealed_holdout(tenant_id, lineage, requested_by)
    }
    fn record_sealed_holdout(
        &self,
        tenant_id: &str,
        lineage: &str,
        trial_id: Uuid,
        result: &serde_json::Value,
    ) -> Result<(), ledger::LedgerError> {
        self.inner.record_sealed_holdout(tenant_id, lineage, trial_id, result)
    }
    fn define_campaign(
        &self,
        tenant_id: &str,
        def: &ledger::campaign::CampaignDefinition,
    ) -> Result<ledger::campaign::CampaignHandle, ledger::LedgerError> {
        self.inner.define_campaign(tenant_id, def)
    }
    fn campaign_exploration_fraction(&self, tenant_id: &str, campaign_id: Uuid) -> f64 {
        self.inner.campaign_exploration_fraction(tenant_id, campaign_id)
    }
}

/// Drive a whole funnel and hand back what it recorded.
fn funnel_evidence(f: Fixture) -> (Option<TrialEvidence>, BTreeMap<String, f64>, usize) {
    let led = Arc::new(WitnessedLedger::new());
    let mgr = SuiteManager::with_executor(
        Box::new(backtest::run::ClosureExecutor(executor(f))),
        led.clone(),
    );
    let user = Uuid::new_v4();
    let view = mgr
        .create_experiment(
            user,
            CreateExperimentSpec {
                experiment_id: "exp-evidence".into(),
                strategy_family: "trend".into(),
                strategy_type: "trend".into(),
                universe_ref: "u".into(),
                research_start: Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap(),
                research_end: Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
                holdout_start: Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
                holdout_end: Utc.with_ymd_and_hms(2023, 7, 1, 0, 0, 0).unwrap(),
                eval_resolution: Some("1d".into()),
                strategy_ref: None,
                objective: None,
                delta_practical: 0.1,
            },
        )
        .expect("experiment created");
    // INV-3: a null is chosen before the funnel runs. Since this test was
    // written, `choose_null` also refuses an *unexplained* override — a kind
    // differing from `recommend_null(strategy_type)` with no logged reason.
    // This fixture has nothing to say about null choice (it is about evidence),
    // so it takes the recommendation rather than inventing a reason to depart
    // from it. Asked for, not hard-coded, so the pair cannot drift apart.
    let recommended = backtest::nulls::generators::recommend_null("trend");
    let chosen = mgr
        .choose_null(user, view.id, recommended, None)
        .expect("a null is chosen before the funnel runs (INV-3)");
    assert!(
        !chosen.was_override && chosen.override_reason.is_none(),
        "this fixture must take the recommendation, not depart from it unexplained"
    );
    assert_ne!(
        chosen.kind,
        NullKind::BlockPermutation,
        "the kind this test originally asked for is the one `trend` is not recommended"
    );
    let board = mgr.advance_funnel(user, view.id).expect("the funnel runs");

    // Gate evidence is gathered *after* Gate 3, so a fixture that stops the
    // candidate earlier records nothing and every assertion below degrades into
    // `expect("the funnel recorded evidence")` — a fixture failure wearing the
    // costume of a platform failure. This says which it is. (It has already
    // happened once: the original fixture handed back `MetricSet::empty()`
    // beside a rising equity curve, Gate 1 read a median Sharpe of 0.0000, and
    // the funnel stopped at "not profitable" three gates early.)
    let stopped: Vec<&str> = board
        .gates
        .iter()
        .filter(|g| !matches!(g.status, backtest::suite::GateStatus::Passed))
        .map(|g| g.summary.as_deref().unwrap_or("locked, no verdict"))
        .collect();
    assert!(
        stopped.len() <= 1,
        "the fixture must reach the evidence step: the funnel stopped at {stopped:?}"
    );

    let tenant = user.to_string();
    let trials = led.inner.rows(&tenant);
    // The evidence is attached to the observed run's trial; find whichever trial
    // carries it.
    let evidence = trials
        .iter()
        .find_map(|t| led.inner.gate_evidence(&tenant, t.trial_id));
    let stats = trials
        .iter()
        .find_map(|t| {
            let s = led.statistics(&tenant, t.trial_id);
            (!s.is_empty()).then_some(s)
        })
        .unwrap_or_default();
    (evidence, stats, trials.len())
}

const EVIDENCE_RICH: Fixture = Fixture {
    costs_per_trade: 12.0,
    liquidity_measured: true,
};

/// The whole point, in one test: a candidate whose run charged costs and whose
/// instruments had measured liquidity produces evidence for all three of the
/// gates the funnel can feed, and all three render **decided** verdicts.
#[test]
fn a_strategy_with_evidence_gets_real_verdicts_at_gates_3_4_and_12() {
    let (evidence, stats, _) = funnel_evidence(EVIDENCE_RICH);
    let e = evidence.expect("the funnel recorded evidence");
    let profile = GateProfile::strict_v1();

    // ── Gate 3 ───────────────────────────────────────────────────────────────
    assert!(e.cost_ladder.len() >= 2, "a ladder, not a single assumption");
    let ladder: Vec<CostRung> = e
        .cost_ladder
        .iter()
        .map(|r| CostRung { multiple: r.multiple, net_metric: r.net_metric })
        .collect();
    let g3 = gate_3_cost_sensitivity(&profile, &ladder);
    assert!(!g3.inconclusive, "gate 3 judged on nothing: {}", g3.detail);
    assert!(g3.statistic.is_some(), "{}", g3.detail);
    // And the scalar on the record agrees with the shape.
    let recorded = stats
        .get("breakeven_cost_multiple")
        .copied()
        .expect("the funnel recorded the breakeven it measured");
    let interpolated = backtest::gates::evaluators::breakeven_multiple(&ladder);
    if let Some(b) = interpolated {
        assert!(
            (b - recorded).abs() < 1e-6,
            "the stored scalar and the stored shape must be the same number: {b} vs {recorded}"
        );
    }

    // ── Gate 4 ───────────────────────────────────────────────────────────────
    let base = e.capacity_base.expect("a base AUM was recorded");
    assert!(
        e.capacity_curve.len() >= 5,
        "§12.3 wants roughly 5-8 rungs, got {}",
        e.capacity_curve.len()
    );

    // The ladder is a *curve*, not seven numbers. `Ladder::from_runs` charges
    // each rung the impact its own size implies, so net Sharpe must fall
    // monotonically with AUM.
    //
    // This is not decoration. Before this fixture's engine was made size-blind
    // it hashed `aum_usd` into its edge, which put ±25% of pseudo-random noise
    // on every rung. Measured, with everything else held as it is here:
    //
    //   size-blind  17.96 17.10 15.38 13.44  9.59 5.25 -0.88  → capacity $2.85M
    //   hashed      16.91 10.69 12.25  8.90  7.20 3.75 -4.44  → capacity $1.39M
    //
    // The second curve rises between rung 2 and rung 3, and it makes Gate 4
    // render a **decided** verdict on a capacity that is 2.05x off — the exact
    // failure this file's header names, a gate handed an input that is not
    // evidence and converting it into a confident wrong number. Monotonicity is
    // what tells the two apart, so it is asserted rather than assumed.
    let sharpes: Vec<f64> = e.capacity_curve.iter().map(|p| p.net_sharpe).collect();
    assert!(
        e.capacity_curve.windows(2).all(|w| w[0].aum_usd < w[1].aum_usd),
        "rungs must be stored in AUM order: {:?}",
        e.capacity_curve.iter().map(|p| p.aum_usd).collect::<Vec<_>>()
    );
    assert!(
        sharpes.windows(2).all(|w| w[1] < w[0]),
        "modelled impact only ever costs; net Sharpe must fall with size: {sharpes:?}"
    );
    // And it has to fall *far enough to be found*: the half-Sharpe point must
    // lie inside the ladder, or the gate is honestly inconclusive and the
    // evidence-rich half of this pair asserts nothing. See `SIZE`.
    assert!(
        backtest::gates::evaluators::capacity_at_half_sharpe(
            &e.capacity_curve
                .iter()
                .map(|p| (p.aum_usd, p.net_sharpe))
                .collect::<Vec<_>>()
        )
        .is_some(),
        "net Sharpe never halves in 1x-100x, so Gate 4 below measures nothing: {sharpes:?}"
    );
    let mut adv = BTreeMap::new();
    let mut vol = BTreeMap::new();
    let mut peak = BTreeMap::new();
    let mut by_aum = Vec::new();
    let inputs = backtest::capacity::capacity_inputs(
        &e.instrument_pnl,
        base,
        &e.capacity_curve,
        &mut adv,
        &mut vol,
        &mut peak,
        &mut by_aum,
    )
    .expect("both halves present");
    let g4 = gate_4_capacity(&profile, &inputs);
    assert!(!g4.inconclusive, "gate 4 judged on nothing: {}", g4.detail);
    assert!(g4.statistic.is_some(), "{}", g4.detail);

    // ── Gate 12 ──────────────────────────────────────────────────────────────
    let (unit, pairs) =
        ledger::evidence::per_instrument_pnl(&e.instrument_pnl).expect("one numeraire");
    assert_eq!(unit, "USD");
    let nbhd: Vec<(f64, f64)> = e
        .neighbourhood
        .iter()
        .map(|p| (p.param_value, p.metric))
        .collect();
    let centre = e
        .neighbourhood
        .iter()
        .find(|p| p.is_center)
        .expect("the neighbourhood declares its centre");
    let g12 = gate_12_perturbation(
        &profile,
        &PerturbationInputs {
            center: centre.metric,
            neighbourhood: &nbhd,
            per_instrument_pnl: &pairs,
            numeraire: Some(&unit),
        },
    );
    assert!(!g12.inconclusive, "gate 12 judged on nothing: {}", g12.detail);
    assert!(g12.statistic.is_some(), "{}", g12.detail);
}

/// Every rung of the capacity sweep is a registered trial, and the sweep raises
/// the trial count by exactly its own length.
///
/// INV-16 has no exception for runs taken to satisfy a gate. A capacity sweep
/// that dodged the ledger would be precisely the kind of uncounted look this
/// platform exists to make impossible.
#[test]
fn every_capacity_rung_is_a_counted_trial() {
    let (with_sweep, _, counted_with) = funnel_evidence(EVIDENCE_RICH);
    let rungs = with_sweep.expect("evidence").capacity_curve;
    assert_eq!(rungs.len(), backtest::capacity::AUM_MULTIPLES.len());

    // No two rungs share a trial id — each is its own look.
    let ids: std::collections::BTreeSet<Uuid> =
        rungs.iter().map(|p| p.member_trial_id).collect();
    assert_eq!(ids.len(), rungs.len(), "one counted trial per rung");

    // And the same funnel without the sweep counts exactly that many fewer.
    let (_, _, counted_without) = funnel_evidence(Fixture {
        liquidity_measured: false,
        ..EVIDENCE_RICH
    });
    assert_eq!(
        counted_with - counted_without,
        rungs.len(),
        "the sweep's cost shows up in the trial counter, in full"
    );
}

/// The adversarial half for Gate 3, and the one that matters most.
///
/// A run that paid no costs is the signature of every simulation produced before
/// ADR-P6-12: the engine charged zero commission on every fill and labelled the
/// gross number net. A cost sweep over such a run returns the same metric at
/// every rung, and `gate_3_cost_sensitivity` reads a flat, positive ladder as
/// "the edge survives every rung tried" and **passes** it.
///
/// So no ladder is built at all, no breakeven is recorded, and Gate 3 says it
/// cannot judge.
#[test]
fn gate_3_refuses_a_run_that_was_never_charged() {
    let (evidence, stats, _) = funnel_evidence(Fixture {
        costs_per_trade: 0.0,
        liquidity_measured: true,
    });
    let e = evidence.expect("the other evidence is still recorded");
    assert!(
        e.cost_ladder.is_empty(),
        "a costless run must yield no ladder, not a flat one"
    );
    assert!(
        !stats.contains_key("breakeven_cost_multiple"),
        "and no breakeven scalar either — an absent measurement is not a zero"
    );

    let g3 = gate_3_cost_sensitivity(&GateProfile::strict_v1(), &[]);
    assert!(g3.inconclusive && !g3.passed, "{}", g3.detail);
    assert!(g3.statistic.is_none(), "no statistic was computed");
}

/// The adversarial half for Gate 4: without measured ADV the sweep is not merely
/// skipped at the gate, it is never **dispatched** — seven counted trials is a
/// real price to pay for evidence that cannot be completed.
#[test]
fn gate_4_refuses_and_the_sweep_is_never_dispatched_without_adv() {
    let (evidence, _, _) = funnel_evidence(Fixture {
        liquidity_measured: false,
        ..EVIDENCE_RICH
    });
    let e = evidence.expect("the cost ladder and cross-section are still recorded");
    assert!(e.capacity_curve.is_empty(), "no curve without ADV");
    assert!(e.capacity_base.is_none(), "and no base to imply one");
    assert!(
        e.instrument_pnl.iter().all(|r| r.adv_usd.is_none()),
        "absent ADV must be stored as absent, never as zero"
    );

    // The gate then refuses by name rather than passing on a guess.
    let mut adv = BTreeMap::new();
    let mut vol = BTreeMap::new();
    let mut peak: BTreeMap<String, f64> = e
        .instrument_pnl
        .iter()
        .map(|r| (r.instrument.clone(), r.peak_daily_notional))
        .collect();
    let curve = [(1e5, 2.0), (1e6, 0.9)];
    let g4 = gate_4_capacity(
        &GateProfile::strict_v1(),
        &CapacityInputs {
            adv_usd: &adv,
            daily_vol: &vol,
            peak_daily_notional: &peak,
            sharpe_by_aum: &curve,
            base_aum_usd: 100_000.0,
            single_venue: true,
        },
    );
    assert!(g4.inconclusive && !g4.passed, "{}", g4.detail);
    adv.clear();
    vol.clear();
    peak.clear();
}

/// Gate 10's two halves, and the fact that only one of them was ever the
/// visible gap.
///
/// The factor *series* were missing — but so was the battery's **declaration**:
/// `GateProfile::strict_v1()` carries no asset classes, so Gate 10 reported
/// "declares no factor battery for asset class `crypto`" whatever evidence
/// existed. Both halves have to be present for a verdict, and this pins each of
/// them failing alone.
#[test]
fn gate_10_needs_both_the_declared_battery_and_the_series() {
    let declared = ["cmkt", "csmb", "cmom", "ccarry"];
    let panel = crypto_panel(12, 300, true);
    let battery = backtest::factors::build_declared(
        &declared.iter().map(ToString::to_string).collect::<Vec<_>>(),
        &panel,
        backtest::factors::BatteryConfig::default(),
    )
    .expect("a twelve-instrument panel with funding builds the declared battery");

    let strategy_ts = battery[0].ts.clone();
    let strategy: Vec<f64> = (0..strategy_ts.len())
        .map(|i| 0.002 + f64::from(u32::try_from(i % 5).unwrap()) / 2_000.0 - 0.001)
        .collect();
    let (net, factors) =
        backtest::factors::align_to_strategy(&strategy_ts, &strategy, &battery, 30)
            .expect("the series overlap");

    // (a) Series but no declaration: inconclusive, and it names the asset class.
    let undeclared = gate_10_factor_attribution(
        &GateProfile::strict_v1(),
        "crypto",
        &FactorInputs {
            strategy_net: &net,
            factors: &factors,
            market_factor: "cmkt".into(),
            neutral_claim: false,
        },
    );
    assert!(undeclared.inconclusive && !undeclared.passed, "{}", undeclared.detail);
    assert!(undeclared.detail.contains("declares no factor battery"));

    let profile = GateProfile::strict_v1().with_asset_class(
        "crypto",
        AssetClassGates {
            factor_battery: declared.iter().map(ToString::to_string).collect(),
            crisis_windows: vec![],
        },
    );

    // (b) Declaration but no series: inconclusive, and it names what is missing.
    let empty = BTreeMap::new();
    let no_series = gate_10_factor_attribution(
        &profile,
        "crypto",
        &FactorInputs {
            strategy_net: &net,
            factors: &empty,
            market_factor: "cmkt".into(),
            neutral_claim: false,
        },
    );
    assert!(no_series.inconclusive && !no_series.passed, "{}", no_series.detail);
    assert!(no_series.detail.contains("ccarry"), "{}", no_series.detail);

    // (c) Both: a decided verdict with a t-statistic.
    let decided = gate_10_factor_attribution(
        &profile,
        "crypto",
        &FactorInputs {
            strategy_net: &net,
            factors: &factors,
            market_factor: "cmkt".into(),
            neutral_claim: false,
        },
    );
    assert!(!decided.inconclusive, "gate 10 judged on nothing: {}", decided.detail);
    assert!(decided.statistic.is_some(), "{}", decided.detail);
}

/// Carry is the funding rate, and funding is forward-collect-only. On a universe
/// with no funding history the declared battery cannot be completed, and Gate 10
/// must say so rather than regressing against three factors while claiming four.
#[test]
fn gate_10_refuses_a_battery_it_cannot_complete() {
    let declared: Vec<String> = ["cmkt", "csmb", "cmom", "ccarry"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let panel = crypto_panel(12, 300, false);
    let gap = backtest::factors::build_declared(
        &declared,
        &panel,
        backtest::factors::BatteryConfig::default(),
    )
    .expect_err("no funding, no carry");
    assert!(
        matches!(gap, backtest::factors::FactorGap::CarryUnavailable(_)),
        "{gap}"
    );
}

/// A universe of instrument return series, built the only way the battery
/// accepts: from a cross-section of instruments, never from a strategy.
fn crypto_panel(
    instruments: usize,
    periods: usize,
    funding: bool,
) -> backtest::factors::MarketPanel {
    let t0 = Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap();
    let list = (0..instruments)
        .map(|i| {
            let p = backtest::factors::InstrumentPanel {
                instrument: format!("SYM{i}-USD"),
                ts: (0..periods)
                    .map(|k| t0 + Duration::days(i64::try_from(k).unwrap()))
                    .collect(),
                returns: (0..periods)
                    .map(|k| {
                        let a = f64::from(u32::try_from((k * 13 + i * 7) % 23).unwrap());
                        let b = f64::from(u32::try_from(i).unwrap());
                        (a - 11.0) / 1_500.0 + (b - 5.0) * 0.0001
                    })
                    .collect(),
                dollar_volume: (0..periods)
                    .map(|k| {
                        1e6 * f64::from(u32::try_from(i + 1).unwrap())
                            + f64::from(u32::try_from(k % 9).unwrap())
                    })
                    .collect(),
                funding: None,
            };
            if funding {
                let f = (0..periods)
                    .map(|k| f64::from(u32::try_from((k + i) % 5).unwrap()) * 0.0001 - 0.0002)
                    .collect();
                p.with_funding(f)
            } else {
                p
            }
        })
        .collect();
    backtest::factors::MarketPanel {
        universe: "test-crypto".into(),
        instruments: list,
    }
}
