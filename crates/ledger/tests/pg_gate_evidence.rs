//! Live-Postgres checks for the shaped gate evidence (SPEC §12.3, migration 0119).
//!
//! `cargo test -p ledger --test pg_gate_evidence -- --ignored --test-threads=1`
//!
//! Three properties, and only the first is the happy path:
//!
//! 1. What the funnel writes is what the gate worker reads — the ladder in
//!    multiple order, the curve in AUM order, the neighbourhood in parameter
//!    order, and every `None` still `None`.
//! 2. **Absence reads as absence.** A trial with no evidence returns `None`, not
//!    an empty bundle: a caller handed `TrialEvidence::default()` cannot tell
//!    "nobody measured this" from "it measured zero", and that confusion is the
//!    one every gate here exists to prevent.
//! 3. **The database refuses what the types refuse.** A mis-shaped factor
//!    series, a negative notional, an edit to a recorded row. The Rust
//!    validation is a courtesy; the CHECK is the mechanism, because a type only
//!    constrains code that goes through it.

use ledger::evidence::{
    CapacityBase, CapacityPoint, CostRung, FactorSeries, InstrumentPnl, NeighbourhoodPoint,
    TrialEvidence,
};
use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, Registration, TrialSubject};
use sqlx::{Connection, Executor, PgConnection, PgPool};

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0119: &str = include_str!("../../../migrations/0119_gate_evidence_sources.sql");

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// The roles 0044 creates. They are cluster-wide, so a scratch database inherits
/// them when the developer database has been migrated — and does not when it has
/// not, which would fail 0119's GRANTs for a reason that has nothing to do with
/// what is being tested.
const ROLES: &[&str] = &["app_role", "agent_role", "internal_ml_role", "backtest_role"];

async fn scratch(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    for role in ROLES {
        let _ = admin
            .execute(format!("CREATE ROLE {role} NOLOGIN").as_str())
            .await;
    }
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    pool.execute(M0043).await.expect("0043 applies");
    pool.execute(M0119).await.expect("0119 applies");
    pool
}

fn subject(n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({ "n": n }),
        dataset_id: "slice:sha256:abc".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1+exits".into(),
        seed_set: vec![n as i64],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    }
}

async fn trial(l: &PgTrialLedger, ctx: &DispatchContext, n: u64) -> uuid::Uuid {
    let s = subject(n);
    l.register_async(&Registration::new(ctx, &s, 1.0))
        .await
        .expect("registered")
        .trial_id()
}

fn full_bundle(members: &[uuid::Uuid]) -> TrialEvidence {
    let t0 = chrono::Utc::now();
    TrialEvidence {
        cost_ladder: vec![
            CostRung { multiple: 1.0, net_metric: 0.20 },
            CostRung { multiple: 3.0, net_metric: 0.05 },
            CostRung { multiple: 5.0, net_metric: -0.08 },
        ],
        capacity_base: Some(CapacityBase { base_aum_usd: 1_000_000.0, single_venue: true }),
        capacity_curve: members
            .iter()
            .enumerate()
            .map(|(i, m)| CapacityPoint {
                aum_usd: 1_000_000.0 * (i as f64 + 1.0),
                net_sharpe: 2.0 - i as f64 * 0.4,
                member_trial_id: *m,
            })
            .collect(),
        instrument_pnl: vec![
            InstrumentPnl {
                instrument: "BTC-USD".into(),
                numeraire: "USD".into(),
                net_pnl: 1_200.0,
                traded_notional: 900_000.0,
                peak_daily_notional: 120_000.0,
                adv_usd: Some(50_000_000.0),
                daily_vol: Some(0.032),
            },
            InstrumentPnl {
                instrument: "ETH-USD".into(),
                numeraire: "USD".into(),
                net_pnl: 800.0,
                traded_notional: 400_000.0,
                peak_daily_notional: 60_000.0,
                // Deliberately unmeasured: it must come back `None`, not zero.
                adv_usd: None,
                daily_vol: None,
            },
        ],
        neighbourhood: (8..=12)
            .map(|v| NeighbourhoodPoint {
                param: "fast".into(),
                param_value: f64::from(v),
                metric: 1.8 + f64::from((v - 10).abs()) * -0.05,
                is_center: v == 10,
            })
            .collect(),
        factors: vec![FactorSeries {
            factor: "cmkt".into(),
            ts: (0..40).map(|i| t0 + chrono::Duration::days(i)).collect(),
            returns: (0..40).map(|i| f64::from(i % 7) / 1000.0 - 0.003).collect(),
        }],
        produced_by: "funnel_advance@backtest@0.1.0+sim-sdk".into(),
    }
}

/// The round trip, part by part.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn what_the_funnel_writes_is_what_the_gate_reads() {
    let pool = scratch("gate_evidence_roundtrip").await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("tenant-e", "mason", 0.1).with_experiment("exp-e");
    let subject_trial = trial(&l, &ctx, 0).await;
    let mut members = Vec::new();
    for n in 1..=5 {
        members.push(trial(&l, &ctx, n).await);
    }

    let written = full_bundle(&members);
    l.record_gate_evidence_async("tenant-e", subject_trial, &written)
        .await
        .expect("evidence recorded");

    let read = l
        .gate_evidence_async("tenant-e", subject_trial)
        .await
        .expect("read")
        .expect("something was written");

    assert_eq!(read.cost_ladder, written.cost_ladder, "in multiple order");
    assert_eq!(read.capacity_base, written.capacity_base);
    assert_eq!(read.capacity_curve, written.capacity_curve, "in AUM order");
    assert_eq!(read.neighbourhood.len(), 5);
    assert_eq!(
        read.neighbourhood.iter().filter(|p| p.is_center).count(),
        1,
        "the centre survives the round trip and is not inferred from position"
    );
    assert_eq!(read.factors.len(), 1);
    assert_eq!(read.factors[0].returns.len(), 40);
    assert_eq!(read.produced_by, written.produced_by, "provenance survives");

    // The unmeasured ADV comes back unmeasured. A zero here would make Gate 4
    // compute a participation share against a denominator nobody measured.
    let eth = read
        .instrument_pnl
        .iter()
        .find(|r| r.instrument == "ETH-USD")
        .expect("both instruments");
    assert_eq!(eth.adv_usd, None, "absent must not round-trip as zero");
    assert_eq!(eth.daily_vol, None);
    let btc = read
        .instrument_pnl
        .iter()
        .find(|r| r.instrument == "BTC-USD")
        .expect("both instruments");
    assert_eq!(btc.adv_usd, Some(50_000_000.0));

    // Every capacity rung names a trial that is really on the ledger — the
    // foreign key says so, and INV-16 is why it is there.
    let counted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mlops.trial_capacity_point p
           JOIN mlops.trial t ON t.trial_id = p.member_trial_id
          WHERE p.trial_id = $1",
    )
    .bind(subject_trial)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counted, 5, "every rung is a counted trial");
}

/// Nothing written reads as nothing, not as zero.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn a_trial_with_no_evidence_reads_as_none_not_as_an_empty_bundle() {
    let pool = scratch("gate_evidence_absent").await;
    let l = PgTrialLedger::new(pool);
    let ctx = DispatchContext::human("tenant-e", "mason", 0.1).with_experiment("exp-e");
    let t = trial(&l, &ctx, 0).await;

    assert!(l
        .gate_evidence_async("tenant-e", t)
        .await
        .expect("read")
        .is_none());

    // And writing an empty bundle is a no-op rather than an error or a row of
    // zeros: a producer that could measure nothing has nothing to say.
    l.record_gate_evidence_async("tenant-e", t, &TrialEvidence::produced_by("funnel"))
        .await
        .expect("an empty bundle is accepted and writes nothing");
    assert!(l
        .gate_evidence_async("tenant-e", t)
        .await
        .expect("read")
        .is_none());
}

/// The database refuses what the types refuse. Each of these is written with raw
/// SQL, bypassing `validate`, because a type only constrains code that goes
/// through it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn the_schema_refuses_evidence_that_is_not_evidence() {
    let pool = scratch("gate_evidence_adversarial").await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("tenant-e", "mason", 0.1).with_experiment("exp-e");
    let t = trial(&l, &ctx, 0).await;

    // A factor series whose timestamps and returns disagree is not a series.
    let mis_shaped = sqlx::query(
        "INSERT INTO mlops.trial_factor_return (tenant_id, trial_id, factor, ts, returns, produced_by)
         VALUES ('tenant-e', $1, 'cmkt', ARRAY[now(), now()], ARRAY[0.1], 'x')",
    )
    .bind(t)
    .execute(&pool)
    .await;
    assert!(mis_shaped.is_err(), "a mis-aligned factor series must be refused");

    // An *empty* one passes an `array_length` CHECK (NULL is not false) and is
    // refused here only because the constraint uses `cardinality`.
    let empty = sqlx::query(
        "INSERT INTO mlops.trial_factor_return (tenant_id, trial_id, factor, ts, returns, produced_by)
         VALUES ('tenant-e', $1, 'cmkt', ARRAY[]::timestamptz[], ARRAY[]::double precision[], 'x')",
    )
    .bind(t)
    .execute(&pool)
    .await;
    assert!(empty.is_err(), "an empty factor series is not a factor series");

    // A negative notional is not a notional.
    let negative = sqlx::query(
        "INSERT INTO mlops.trial_instrument_pnl
           (tenant_id, trial_id, instrument, numeraire, net_pnl, traded_notional,
            peak_daily_notional, produced_by)
         VALUES ('tenant-e', $1, 'BTC-USD', 'USD', 1.0, -5.0, 1.0, 'x')",
    )
    .bind(t)
    .execute(&pool)
    .await;
    assert!(negative.is_err());

    // An unnamed numeraire is how a share over two units gets computed by
    // accident.
    let unnamed = sqlx::query(
        "INSERT INTO mlops.trial_instrument_pnl
           (tenant_id, trial_id, instrument, numeraire, net_pnl, traded_notional,
            peak_daily_notional, produced_by)
         VALUES ('tenant-e', $1, 'BTC-USD', '', 1.0, 5.0, 1.0, 'x')",
    )
    .bind(t)
    .execute(&pool)
    .await;
    assert!(unnamed.is_err());

    // A cost rung at zero times costs is not a cost assumption.
    let zero_rung = sqlx::query(
        "INSERT INTO mlops.trial_cost_rung (tenant_id, trial_id, multiple, net_metric, produced_by)
         VALUES ('tenant-e', $1, 0.0, 0.1, 'x')",
    )
    .bind(t)
    .execute(&pool)
    .await;
    assert!(zero_rung.is_err());

    // A capacity rung must name a trial that exists: an uncounted look cannot be
    // laundered into evidence by writing a uuid nobody registered.
    let uncounted = sqlx::query(
        "INSERT INTO mlops.trial_capacity_point
           (tenant_id, trial_id, aum_usd, net_sharpe, member_trial_id, produced_by)
         VALUES ('tenant-e', $1, 1000.0, 1.0, $2, 'x')",
    )
    .bind(t)
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(uncounted.is_err(), "a rung must name a registered trial (INV-16)");

    // And a recorded row cannot be edited afterwards.
    l.record_gate_evidence_async(
        "tenant-e",
        t,
        &TrialEvidence {
            cost_ladder: vec![
                CostRung { multiple: 1.0, net_metric: 0.1 },
                CostRung { multiple: 4.0, net_metric: -0.1 },
            ],
            produced_by: "funnel".into(),
            ..TrialEvidence::default()
        },
    )
    .await
    .expect("recorded");
    let edit = sqlx::query("UPDATE mlops.trial_cost_rung SET net_metric = 1.0 WHERE trial_id = $1")
        .bind(t)
        .execute(&pool)
        .await;
    assert!(edit.is_err(), "gate evidence is append-only");
    let delete = sqlx::query("DELETE FROM mlops.trial_cost_rung WHERE trial_id = $1")
        .bind(t)
        .execute(&pool)
        .await;
    assert!(delete.is_err(), "and cannot be removed");
}
