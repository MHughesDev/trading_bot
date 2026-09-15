-- Set P Phase 0 (P-0.3, P-0.4, P-0.5): the forward-only series.
--
-- Funding rates, open interest and option chains cannot be bought. Under a
-- free-only data policy (ADR-0027) there is no vendor holding 2024's crypto option
-- chain, and none will appear. Every day these tables are not being written is a
-- permanent, unfillable hole in every study that ever uses the series —
-- `docs/artifact.md` FM-11, the only failure mode in the artifact marked
-- unrecoverable at any cost.
--
-- That is the entire reason this file exists before the interesting work.
--
-- Three disciplines, each load-bearing:
--
--   1. FOUR TIMESTAMPS, kept distinct. `event_time` is when the fact happened;
--      `venue_ts` is the venue's own clock; `ingest_time` is when the bytes
--      arrived; `knowledge_time` is when a strategy could first have used it
--      (INV-01). Conflating the last two is how lookahead gets in.
--
--   2. HONEST PROVENANCE. `quality_flags` carries BACKFILLED_KNOWLEDGE_TIME
--      (0x8000) on every row whose knowledge time was derived from a declared
--      vendor lag rather than observed at ingest. Gate 15 takes forward evidence
--      from unflagged rows only (Set P D-P3). This is not decorative: on
--      2026-09-15 all 158,259 rows of `market_bars_v2` carried
--      `available_time = event_time`, so a year of REST backfills was
--      indistinguishable from live observation. That must not happen again on a
--      series that cannot be re-collected.
--
--   3. IV, NEVER GREEKS (INV-07). `option_chain_snapshots` stores implied
--      volatility together with the inputs it was computed from; deltas and vegas
--      are derived at read time from those inputs. A stored greek is a number
--      nobody can reproduce once the model that made it changes. `AT-08` in
--      `crates/invariants` fails the build if a greek column is ever added here.
--
-- Prices and rates are Decimal — never Float64 (INV-05, SC-1). `iv` is the one
-- Float64, because implied volatility is a model output rather than money, exactly
-- as `option_bar_1m.iv_close` already is in 07_canonical_market.sql.
--
-- Engines are ReplacingMergeTree keyed on the full identity of an observation, so a
-- restart that re-reads the same window collapses to one row instead of duplicating
-- it (idempotence on `(venue, instrument, event_time)` and, for chains, the
-- contract). Nothing that differs in a meaningful dimension can be destroyed.
--
-- NOTE: this file runs on first database init via the /docker-entrypoint-initdb.d
-- mount AND on every boot through `storage::clickhouse::migrate`, which replays it
-- idempotently. Both are required; existing volumes never see the mount.

-- ── Funding rates (DATA-005 §11 S6, §12; DA-11) ───────────────────────────────
-- One row per (venue, instrument, kind, settlement) observation. `kind`
-- distinguishes a realized settlement from the venue's own prediction of the next
-- one: a predicted rate is knowable BEFORE its settlement time and a realized one
-- is not, and a table that stored them in one column without saying which would
-- hand a strategy tomorrow's funding today.
CREATE TABLE IF NOT EXISTS funding_rates (
    venue               String,
    instrument          String,            -- venue symbol, e.g. 'PF_XBTUSD'
    kind                String,            -- 'realized' | 'predicted'

    event_time          DateTime64(9, 'UTC'),   -- settlement instant
    venue_ts            Nullable(DateTime64(9, 'UTC')),
    ingest_time         DateTime64(9, 'UTC'),
    knowledge_time      DateTime64(9, 'UTC'),

    -- The venue's absolute funding rate, and the relative form where published.
    rate                Decimal(38, 18),
    relative_rate       Nullable(Decimal(38, 18)),

    quality_flags       UInt32,            -- 0x8000 = BACKFILLED_KNOWLEDGE_TIME
    source_id           Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (venue, instrument, kind, event_time, knowledge_time)
PARTITION BY toYYYYMM(event_time);

-- ── Open interest (DATA-005 §11 S6, §12) ──────────────────────────────────────
-- Forward-only in the strictest sense: Kraken Futures publishes the CURRENT open
-- interest on its ticker and offers no history endpoint for it. Whatever is not
-- sampled today is gone. `mark_price` rides along because OI without the price it
-- was measured at cannot be converted to notional after the fact.
CREATE TABLE IF NOT EXISTS open_interest (
    venue               String,
    instrument          String,

    event_time          DateTime64(9, 'UTC'),   -- the sample instant
    venue_ts            Nullable(DateTime64(9, 'UTC')),
    ingest_time         DateTime64(9, 'UTC'),
    knowledge_time      DateTime64(9, 'UTC'),

    open_interest       Decimal(38, 18),
    mark_price          Nullable(Decimal(38, 18)),

    quality_flags       UInt32,
    source_id           Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (venue, instrument, event_time, knowledge_time)
PARTITION BY toYYYYMM(event_time);

-- ── Option chain snapshots (DATA-005 §11 S5/S8, §12; DA-12; INV-07) ───────────
-- One row per contract per snapshot. IV is stored WITH the inputs that produced it
-- (`iv_model`, `iv_rate`, `iv_div`, `underlying_price`), so a reader can both use
-- the stored number and recompute it. Greeks are computed at read time from those
-- inputs and are never stored — see AT-08.
--
-- `in_liquid_universe` is ADR-P2-26's universe_id = 1 gate: |moneyness - 1| <= 0.30,
-- dte in [1, 400], two-sided quote. Contracts outside the gate are NOT dropped —
-- they are snapshotted at end-of-day cadence instead, and `snapshot_cadence` records
-- which cadence produced the row. Deleting them would be an irreversible decision
-- taken on a forward-only series to save disk, which is the trade this whole phase
-- exists to refuse.
CREATE TABLE IF NOT EXISTS option_chain_snapshots (
    venue               String,
    underlying          String,
    instrument          String,            -- contract symbol

    expiry              Date,
    strike              Decimal(38, 18),
    option_right        FixedString(1),    -- 'C' | 'P'

    event_time          DateTime64(9, 'UTC'),   -- the snapshot instant
    venue_ts            Nullable(DateTime64(9, 'UTC')),
    ingest_time         DateTime64(9, 'UTC'),
    knowledge_time      DateTime64(9, 'UTC'),

    bid                 Nullable(Decimal(38, 18)),
    ask                 Nullable(Decimal(38, 18)),
    mark                Nullable(Decimal(38, 18)),
    underlying_price    Nullable(Decimal(38, 18)),
    volume              Nullable(Int64),
    open_interest       Nullable(Int64),

    -- Implied volatility and the inputs it was computed from. No greeks (INV-07).
    iv                  Nullable(Float64),
    iv_model            Nullable(String),
    iv_rate             Nullable(Float64),
    iv_div              Nullable(Float64),

    moneyness           Float64,
    dte                 Int32,
    in_liquid_universe  UInt8,             -- ADR-P2-26 universe_id = 1
    snapshot_cadence    String,            -- 'hourly' | 'daily'

    quality_flags       UInt32,
    source_id           Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (venue, underlying, expiry, option_right, strike, event_time, knowledge_time)
PARTITION BY (toYYYYMM(expiry), toYYYYMM(event_time));

-- ── The forward clock (P-0.6 item 2) ──────────────────────────────────────────
-- Last-seen row per forward series, so the silence alarm has one cheap place to
-- look. Silence is this phase's real failure mode: a collector that dies quietly
-- costs exactly what never collecting it would have, and it costs it invisibly.
CREATE TABLE IF NOT EXISTS forward_series_heartbeat (
    series              String,            -- 'funding_rates' | 'open_interest' | 'option_chain_snapshots'
    venue               String,
    instrument          String,
    last_event_time     DateTime64(9, 'UTC'),
    last_ingest_time    DateTime64(9, 'UTC'),
    rows_written        UInt64
)
ENGINE = ReplacingMergeTree(last_ingest_time)
ORDER BY (series, venue, instrument);
