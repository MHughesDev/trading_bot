-- Set-J Phase 0: high-volume per-Run time series (append-only analytics).
-- Decimal-bearing fields are stored as strings (ADR-0002); statistical series
-- (equity, exposure) are Float64 (D-10). Keyed by run_id so a Run's full curve
-- is reconstructable, and immutable like the Postgres metadata row.

-- The equity + net-exposure curve, one row per bar.
CREATE TABLE IF NOT EXISTS backtest_run_equity (
    run_id          String,
    ts_ns           Int64,        -- available_time in unix nanoseconds
    equity          Float64,
    net_exposure    Float64
)
ENGINE = MergeTree()
ORDER BY (run_id, ts_ns);

-- The realized trade list, one row per round-trip.
CREATE TABLE IF NOT EXISTS backtest_run_trades (
    run_id              String,
    symbol              String,
    side                String,       -- long | short
    entry_ts_ns         Int64,
    exit_ts_ns          Int64,
    entry_price_str     String,       -- Decimal as string
    exit_price_str      String,
    qty_str             String,
    mae                 Float64,
    mfe                 Float64,
    holding_period_secs Int64,
    costs_paid_str      String,
    pnl_str             String
)
ENGINE = MergeTree()
ORDER BY (run_id, entry_ts_ns);
