//! The seam Set T's evidence clock runs on: a fill the engine takes reaches a
//! registered observer, exactly once per genuinely new fill.
//!
//! `crates/reconciliation`'s `ForwardRecorder` is one such observer; this test
//! owns the execution half of the contract so the crate that produces fills
//! proves it publishes them, without depending on the crate that consumes
//! them.
//!
//! Delete `fills::publish(&event)` from `ExecutionEngine::capture_fill` and
//! `the_engine_publishes_a_new_fill_to_the_stream` fails.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Once};

use async_trait::async_trait;
use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderState, OrderType, Side},
};
use execution::broker::BrokerOrderState;
use execution::fills::{self, FillEvent, FillObserver};
use execution::journal::{
    self, JournalError, OrderJournal, PersistedFill, RecordedOrder, Uuid, WriteOutcome,
};
use execution::{mock_broker::MockBroker, ExecutionEngine};
use risk::{ApprovedOrder, GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

// ── A journal that is durable enough for one process ─────────────────────────

/// The in-process stand-in for migration 0121's `orders`/`fills` tables.
///
/// Written here rather than taken from `journal::memory` so this test does not
/// depend on a cargo feature being switched on: a test that silently stops
/// compiling into the run is a test that stops protecting anything.
#[derive(Default)]
struct TestJournal {
    orders: Mutex<HashMap<Uuid, RecordedOrder>>,
    fills: Mutex<Vec<PersistedFill>>,
}

#[async_trait]
impl OrderJournal for TestJournal {
    async fn find_order(&self, key: Uuid) -> Result<Option<RecordedOrder>, JournalError> {
        Ok(self.orders.lock().unwrap().get(&key).cloned())
    }

    async fn record_order(
        &self,
        intent: &OrderIntent,
        state: OrderState,
    ) -> Result<WriteOutcome, JournalError> {
        let mut orders = self.orders.lock().unwrap();
        if orders.contains_key(&intent.idempotency_key) {
            return Ok(WriteOutcome::Duplicate);
        }
        orders.insert(
            intent.idempotency_key,
            RecordedOrder {
                idempotency_key: intent.idempotency_key,
                broker_order_id: None,
                state,
            },
        );
        Ok(WriteOutcome::Inserted)
    }

    async fn set_broker_order(
        &self,
        key: Uuid,
        broker_order_id: &str,
        state: OrderState,
    ) -> Result<(), JournalError> {
        if let Some(order) = self.orders.lock().unwrap().get_mut(&key) {
            order.broker_order_id = Some(broker_order_id.to_owned());
            order.state = state;
        }
        Ok(())
    }

    async fn set_state(&self, key: Uuid, state: OrderState) -> Result<(), JournalError> {
        if let Some(order) = self.orders.lock().unwrap().get_mut(&key) {
            order.state = state;
        }
        Ok(())
    }

    async fn record_fill(&self, fill: &PersistedFill) -> Result<WriteOutcome, JournalError> {
        let mut fills = self.fills.lock().unwrap();
        // Migration 0121's UNIQUE (idempotency_key, broker_order_id,
        // fill_price, filled_size), plus the cumulative quantity that keeps two
        // equal partials distinct.
        let duplicate = fills.iter().any(|existing| {
            existing.idempotency_key == fill.idempotency_key
                && existing.broker_order_id == fill.broker_order_id
                && existing.fill_price == fill.fill_price
                && existing.filled_size == fill.filled_size
                && existing.cumulative_filled == fill.cumulative_filled
        });
        if duplicate {
            return Ok(WriteOutcome::Duplicate);
        }
        fills.push(fill.clone());
        Ok(WriteOutcome::Inserted)
    }

    async fn fills_for(&self, key: Uuid) -> Result<Vec<PersistedFill>, JournalError> {
        Ok(self
            .fills
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.idempotency_key == key)
            .cloned()
            .collect())
    }

    async fn record_audit(
        &self,
        _entries: &[execution::journal::AuditEntry],
    ) -> Result<(), JournalError> {
        // The audit trail is not what this file tests; `journal::memory` and
        // `order_audit_trail` cover it.
        Ok(())
    }
}

// ── The observer under test ──────────────────────────────────────────────────

/// Counts fills per idempotency key.
///
/// Per key, not in total: the registry is process-wide and this binary's tests
/// run in parallel, so an unfiltered counter would assert on how busy the
/// suite is rather than on the seam.
#[derive(Default)]
struct KeyCounter {
    seen: Mutex<HashMap<Uuid, usize>>,
    prices: Mutex<HashMap<Uuid, (Decimal, Price)>>,
}

impl KeyCounter {
    fn count(&self, key: Uuid) -> usize {
        self.seen.lock().unwrap().get(&key).copied().unwrap_or(0)
    }

    fn detail(&self, key: Uuid) -> Option<(Decimal, Price)> {
        self.prices.lock().unwrap().get(&key).copied()
    }
}

impl FillObserver for KeyCounter {
    fn on_fill(&self, fill: &FillEvent) {
        *self
            .seen
            .lock()
            .unwrap()
            .entry(fill.idempotency_key)
            .or_insert(0) += 1;
        self.prices
            .lock()
            .unwrap()
            .insert(fill.idempotency_key, (fill.filled_qty, fill.fill_price));
    }
}

static INIT: Once = Once::new();
static COUNTER: Mutex<Option<Arc<KeyCounter>>> = Mutex::new(None);

/// Install the journal and attach the observer, once per process, *before* any
/// order can be submitted. Every test in this binary calls it first, so no
/// fill in this process can ever be published into an empty registry.
fn observed_platform() -> Arc<KeyCounter> {
    INIT.call_once(|| {
        journal::install(Arc::new(TestJournal::default()));
        let counter = Arc::new(KeyCounter::default());
        fills::register_observer(Arc::clone(&counter) as Arc<dyn FillObserver>);
        *COUNTER.lock().unwrap() = Some(counter);
    });
    Arc::clone(COUNTER.lock().unwrap().as_ref().expect("initialised"))
}

fn filling_broker(qty: &str, price: &str) -> Arc<MockBroker> {
    let mut broker = MockBroker::new();
    broker.filled_qty = Decimal::from_str(qty).unwrap();
    broker.fill_price = Some(Price::from_str(price).unwrap());
    broker.query_state = BrokerOrderState::Filled;
    Arc::new(broker)
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

fn market_buy(instrument: &str, qty: &str) -> OrderIntent {
    OrderIntent::new(
        instrument,
        Side::Buy,
        OrderType::Market,
        Size::from_str(qty).unwrap(),
        None,
        None,
    )
}

// ── The tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_engine_publishes_a_new_fill_to_the_stream() {
    let counter = observed_platform();
    let broker = filling_broker("1", "50000");
    let engine = ExecutionEngine::new(Arc::clone(&broker) as Arc<_>);

    let intent = market_buy("BTC-USD", "1");
    let key = intent.idempotency_key;
    assert_eq!(counter.count(key), 0, "nothing has been submitted yet");

    let result = engine.submit(approved(intent)).await.expect("submitted");
    assert!(result.fill.is_some(), "the mock venue filled on submit");

    assert_eq!(
        counter.count(key),
        1,
        "a fill the engine recorded must reach the fill stream — without this \
         wire a paper credential buys elapsed time and no forward evidence"
    );
    let (qty, price) = counter.detail(key).expect("the observer kept the fill");
    assert_eq!(qty, Decimal::from(1));
    assert_eq!(price, Price::from_str("50000").unwrap());

    let evidence = fills::evidence();
    assert!(evidence.is_recording());
    assert_eq!(
        evidence.fills_unobserved, 0,
        "an observer was attached before the first order, so no fill in this \
         process can have gone unobserved"
    );
}

#[tokio::test]
async fn a_resubmitted_order_is_not_a_second_observation() {
    let counter = observed_platform();
    let broker = filling_broker("1", "50000");

    let intent = market_buy("ETH-USD", "1");
    let key = intent.idempotency_key;

    let first = ExecutionEngine::new(Arc::clone(&broker) as Arc<_>);
    first
        .submit(approved(intent.clone()))
        .await
        .expect("submitted");
    assert_eq!(counter.count(key), 1);

    // The restart case: a fresh engine (empty in-memory dedup) resubmitting a
    // key the journal already holds.
    let after_restart = ExecutionEngine::new(Arc::clone(&broker) as Arc<_>);
    let again = after_restart
        .submit(approved(intent))
        .await
        .expect("the journal answers without touching the broker");
    assert!(again.duplicate, "the key was already recorded");

    assert_eq!(
        counter.count(key),
        1,
        "a redelivery must not fire the stream again — a fill counted twice is \
         a forward observation that never happened"
    );
}
