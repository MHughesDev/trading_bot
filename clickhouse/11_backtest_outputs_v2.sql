-- Set M M-2.5 (FEAT-005 §8): the rich outputs of a Run, enough to explain every trade.
-- (07, 08, 09 and 10 are taken, so this is 11; ADR-M-07.)
--
-- Keyed by run_id and **ReplacingMergeTree on the full key**, so writing a Run's outputs a
-- second time (a re-run, a retried job) collapses onto the same rows rather than doubling
-- them: the writer is idempotent by run_id. Read with FINAL.
-- Money is a String (Decimal, ADR-0002); statistics are Float64.

-- One row per fill.
CREATE TABLE IF NOT EXISTS backtest_run_fills (
    run_id           String,
    seq              UInt32,
    decided_ns       Int64,
    filled_ns        Int64,
    side             String,
    qty_str          String,
    price_str        String,
    fee_str          String,
    spread_cost_str  String,
    impact_cost_str  String
)
ENGINE = ReplacingMergeTree()
ORDER BY (run_id, seq);

-- One row per bar: what the account held.
CREATE TABLE IF NOT EXISTS backtest_run_positions (
    run_id    String,
    ts_ns     Int64,
    qty       Float64,
    exposure  Float64,
    equity    Float64
)
ENGINE = ReplacingMergeTree()
ORDER BY (run_id, ts_ns);

-- One row per decision bar: what the strategy saw and chose. `slots` are the values it
-- computed, `signals` the conditions that fired, `intents` the orders it asked for.
CREATE TABLE IF NOT EXISTS backtest_run_decisions (
    run_id   String,
    ts_ns    Int64,
    slots    String,   -- JSON object
    signals  String,   -- JSON object
    intents  String,   -- JSON array
    regime   String,
    path     String    -- which branch of the strategy produced the intent
)
ENGINE = ReplacingMergeTree()
ORDER BY (run_id, ts_ns);

-- One row per round trip, self-contained so `explain_trade` needs no join: when it was
-- entered and left, its excursions, why it exited, how it entered, and the cost legs.
-- What the Run's trade list could not carry: why it exited, how it entered, and the cost legs.
-- Their sum is the trade's `costs_paid_str` in backtest_run_trades.
CREATE TABLE IF NOT EXISTS backtest_run_trade_detail (
    run_id              String,
    trade_idx           UInt32,
    symbol              String,
    side                String,
    entry_ts_ns         Int64,
    exit_ts_ns          Int64,
    mae                 Float64,
    mfe                 Float64,
    exit_reason         String,
    entry_order_type    String,
    fee_str             String,
    spread_cost_str     String,
    impact_cost_str     String,
    borrow_cost_str     String
)
ENGINE = ReplacingMergeTree()
ORDER BY (run_id, trade_idx);
