//! Set E audit item #3: every order's audit trail reaches the journal.
//!
//! `ExecutionEngine::submit` built `AuditEntry` records and returned them to
//! callers that dropped them; no table existed. The engine now writes the trail
//! itself, so this binary installs a journal and asserts on what the journal
//! received — not on `SubmitResult.audit`, which is exactly the vector nobody
//! persisted.
//!
//! The Postgres half (`order_audit`, migration 0152) is `#[ignore]`d and needs
//! `DATABASE_URL` pointing at a database migrated through 0152.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderState, OrderType, Side},
};
use execution::audit::AuditEntry;
use execution::broker::BrokerOrderState;
use execution::journal::{self, memory::MemoryOrderJournal, OrderJournal, PgOrderJournal};
use execution::{mock_broker::MockBroker, ExecutionEngine};
use risk::{ApprovedOrder, GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;
use uuid::Uuid;

static JOURNAL: OnceLock<Arc<MemoryOrderJournal>> = OnceLock::new();

fn journal() -> Arc<MemoryOrderJournal> {
    Arc::clone(JOURNAL.get_or_init(|| {
        let j = Arc::new(MemoryOrderJournal::new());
        journal::install(Arc::clone(&j) as Arc<dyn OrderJournal>);
        j
    }))
}

fn approved(intent: OrderIntent) -> ApprovedOrder {
    let gate = RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    );
    let ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_str("50000").unwrap()),
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    );
    gate.check(intent, &ctx).expect("approved")
}

fn market_buy(instrument: &str) -> OrderIntent {
    OrderIntent::new(
        instrument,
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    )
}

fn transitions(entries: &[AuditEntry]) -> Vec<(Option<OrderState>, OrderState)> {
    entries
        .iter()
        .filter_map(|e| match e {
            AuditEntry::StateTransition { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_filled_order_leaves_its_whole_trail_in_the_journal() {
    let j = journal();
    let mut broker = MockBroker::new();
    broker.filled_qty = Decimal::ONE;
    broker.fill_price = Some(Price::from_str("50000").unwrap());
    broker.query_state = BrokerOrderState::Filled;
    let engine = ExecutionEngine::new(Arc::new(broker));

    let intent = market_buy("BTC-USD");
    let key = intent.idempotency_key;
    engine.submit(approved(intent)).await.expect("submitted");

    let trail = j.audit_for(key);
    assert_eq!(
        transitions(&trail),
        vec![
            (None, OrderState::Accepted),
            (Some(OrderState::Accepted), OrderState::Submitted),
            (Some(OrderState::Submitted), OrderState::Filled),
        ],
        "the journal must hold every transition of the order, in order: {trail:#?}"
    );
    assert!(
        trail
            .iter()
            .any(|e| matches!(e, AuditEntry::BrokerSubmit { .. })),
        "the broker submission is part of the trail"
    );
}

#[tokio::test]
async fn a_venue_rejection_is_in_the_trail_with_its_reason() {
    let j = journal();
    let mut broker = MockBroker::new();
    broker.submit_succeeds = false;
    let engine = ExecutionEngine::new(Arc::new(broker));

    let intent = market_buy("ETH-USD");
    let key = intent.idempotency_key;
    assert!(engine.submit(approved(intent)).await.is_err());

    let trail = j.audit_for(key);
    let rejected = trail.iter().find_map(|e| match e {
        AuditEntry::StateTransition {
            to: OrderState::Rejected,
            reason,
            ..
        } => Some(reason.clone()),
        _ => None,
    });
    assert!(
        rejected
            .flatten()
            .is_some_and(|r| r.contains("mock rejection")),
        "a rejection must be recorded with the venue's reason: {trail:#?}"
    );
}

#[tokio::test]
async fn a_duplicate_submit_is_recorded_and_does_not_reach_the_venue() {
    let j = journal();
    let mut broker = MockBroker::new();
    broker.filled_qty = Decimal::ONE;
    broker.fill_price = Some(Price::from_str("50000").unwrap());
    broker.query_state = BrokerOrderState::Filled;
    let broker = Arc::new(broker);
    let engine = ExecutionEngine::new(Arc::clone(&broker) as Arc<_>);

    let intent = market_buy("SOL-USD");
    let key = intent.idempotency_key;
    engine.submit(approved(intent.clone())).await.unwrap();
    let again = engine.submit(approved(intent)).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(broker.submit_call_count(), 1);
    assert!(j.audit_for(key).iter().any(|e| matches!(
        e,
        AuditEntry::BrokerQuery { reason, .. } if reason.starts_with("duplicate-submit")
    )));
}

// ── Postgres: the table, its append-only rule and its grant ─────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0152"]
async fn the_trail_is_written_to_order_audit_and_cannot_be_edited() {
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connect");
    let pg = PgOrderJournal::new(pool.clone());
    let key = Uuid::new_v4();
    pg.record_audit(&[
        AuditEntry::state_transition(key, None, OrderState::Accepted, None),
        AuditEntry::broker_submit(key, "b-1".into()),
        AuditEntry::state_transition(key, Some(OrderState::Accepted), OrderState::Submitted, None),
    ])
    .await
    .expect("recorded");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, to_state FROM order_audit WHERE idempotency_key = $1 ORDER BY audit_id",
    )
    .bind(key)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("state_transition".into(), Some("accepted".into())),
            ("broker_submit".into(), None),
            ("state_transition".into(), Some("submitted".into())),
        ]
    );

    // Append-only for everyone, owner included.
    let edit = sqlx::query("UPDATE order_audit SET reason = 'x' WHERE idempotency_key = $1")
        .bind(key)
        .execute(&pool)
        .await;
    assert!(edit.is_err(), "an audit row was edited");
    let del = sqlx::query("DELETE FROM order_audit WHERE idempotency_key = $1")
        .bind(key)
        .execute(&pool)
        .await;
    assert!(del.is_err(), "an audit row was deleted");

    // The platform's role can append and read (owner-connected tests cannot
    // see a missing grant on their own).
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO order_audit (idempotency_key, kind, to_state, entry, occurred_at) \
         VALUES ($1, 'state_transition', 'accepted', '{}', now())",
    )
    .bind(key)
    .execute(&mut *tx)
    .await
    .expect("app_role can append to order_audit");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM order_audit WHERE idempotency_key = $1")
        .bind(key)
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can read order_audit");
    assert_eq!(n, 4);
    tx.rollback().await.unwrap();
}
