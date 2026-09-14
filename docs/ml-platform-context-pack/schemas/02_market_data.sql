-- §1.2–1.9 Market data. Four timestamps everywhere. [INV-01/05/06]
-- Iceberg-backed in production; expressed here for readability.

CREATE TABLE bar_1m (
  instrument_id  BIGINT NOT NULL,
  venue_id       INT    NOT NULL,
  event_time     TIMESTAMP(9) NOT NULL,        -- bar OPEN, UTC [INV-06]
  venue_ts       TIMESTAMP(9),
  ingest_time    TIMESTAMP(9) NOT NULL,
  knowledge_time TIMESTAMP(9) NOT NULL,        -- [INV-01]
  open   DECIMAL(38,18), high DECIMAL(38,18),  -- [INV-05]
  low    DECIMAL(38,18), close DECIMAL(38,18),
  volume DECIMAL(38,18),
  trade_count INT,
  vwap   DECIMAL(38,18),
  bid_close DECIMAL(38,18), ask_close DECIMAL(38,18),
  bipower_var DOUBLE PRECISION, n_updates INT,
  quality_flags INT NOT NULL DEFAULT 0,        -- §1.9 bitmask
  revision_seq  INT NOT NULL DEFAULT 0,
  source_id     INT NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, event_time, knowledge_time, revision_seq)
);
-- Partition: asset_class / venue_id / event_date   Sort: instrument_id, event_time
-- SPARSE: absence means no trades. Densification happens in the reader. [INV-11]

-- §1.3 Makes PIT reads cheap. >99% of partitions skip resolution entirely. [INV-02]
CREATE TABLE restatement_index (
  instrument_id       BIGINT NOT NULL,
  event_date          DATE   NOT NULL,
  n_revisions         INT    NOT NULL,
  max_knowledge_time  TIMESTAMP(9) NOT NULL,
  PRIMARY KEY (instrument_id, event_date)
);

-- §1.4 Equities/ETFs. Store unadjusted; compose factors at read time. [INV-03]
CREATE TABLE corporate_action (
  action_id        BIGINT PRIMARY KEY,
  instrument_id    BIGINT NOT NULL,
  action_type      TEXT NOT NULL CHECK (action_type IN
                     ('split','dividend','spinoff','merger','symbol_change','delist')),
  announcement_time TIMESTAMP(9) NOT NULL,     -- when the market learned
  ex_date          DATE NOT NULL,
  effective_time   TIMESTAMP(9) NOT NULL,
  price_factor     DECIMAL(38,18),
  volume_factor    DECIMAL(38,18),
  cash_amount      DECIMAL(38,18),
  currency         TEXT,
  knowledge_time   TIMESTAMP(9) NOT NULL,
  revision_seq     INT NOT NULL DEFAULT 0
);

CREATE TABLE index_membership (
  index_id          INT NOT NULL,
  instrument_id     BIGINT NOT NULL,
  announcement_time TIMESTAMP(9),               -- reconstitution announced BEFORE effect
  effective_from    TIMESTAMPTZ NOT NULL,
  effective_to      TIMESTAMPTZ,
  weight            DECIMAL(18,10),
  knowledge_time    TIMESTAMP(9) NOT NULL,
  PRIMARY KEY (index_id, instrument_id, effective_from, knowledge_time)
);

CREATE TABLE session_event (
  instrument_id BIGINT, venue_id INT,
  event_time TIMESTAMP(9), event_type TEXT,    -- halt|luld|auction|resume
  detail JSONB, knowledge_time TIMESTAMP(9) NOT NULL
);

-- §1.5 Futures. Continuous series are VIEWS, never facts. [INV-08]
CREATE TABLE future_contract (
  instrument_id  BIGINT PRIMARY KEY,
  root           TEXT NOT NULL,
  expiry         DATE NOT NULL,
  first_notice   DATE,
  last_trade     DATE,
  contract_size  DECIMAL(38,18),
  tick_value     DECIMAL(38,18)
);

CREATE TABLE roll_schedule (
  root                TEXT NOT NULL,
  roll_rule_id        INT  NOT NULL,            -- calendar|oi_crossover|volume_crossover
  from_instrument_id  BIGINT NOT NULL,
  to_instrument_id    BIGINT NOT NULL,
  roll_event_time     TIMESTAMP(9) NOT NULL,
  decision_time       TIMESTAMP(9) NOT NULL,    -- when the rule COULD fire [INV-08]
  ratio_factor        DECIMAL(38,18),
  knowledge_time      TIMESTAMP(9) NOT NULL,
  CHECK (decision_time <= roll_event_time),
  PRIMARY KEY (root, roll_rule_id, roll_event_time, knowledge_time)
);

-- §1.6 Options. Store IV, never greeks. Long and sparse. [INV-07]
CREATE TABLE option_contract (
  instrument_id  BIGINT PRIMARY KEY,
  underlying_id  BIGINT NOT NULL,
  expiry         DATE NOT NULL,
  strike         DECIMAL(38,18) NOT NULL,
  right          CHAR(1) NOT NULL CHECK (right IN ('C','P')),
  exercise_style CHAR(1) NOT NULL CHECK (exercise_style IN ('A','E')),
  multiplier     INT NOT NULL,
  occ_symbol     TEXT,
  adjusted_flag  BOOLEAN NOT NULL DEFAULT FALSE  -- nonstandard deliverable
);

CREATE TABLE option_bar_1m (
  instrument_id    BIGINT NOT NULL,
  event_time       TIMESTAMP(9) NOT NULL,
  ingest_time      TIMESTAMP(9) NOT NULL,
  knowledge_time   TIMESTAMP(9) NOT NULL,
  open DECIMAL(38,18), high DECIMAL(38,18),
  low  DECIMAL(38,18), close DECIMAL(38,18),
  volume BIGINT,
  open_interest BIGINT,          -- T+1; knowledge_time proves it
  bid_close DECIMAL(38,18), ask_close DECIMAL(38,18),
  underlying_close DECIMAL(38,18),   -- denormalized: joins at this cardinality are ruinous
  iv_close  DOUBLE PRECISION,        -- STORE IV [INV-07]
  iv_model  TEXT, iv_rate DOUBLE PRECISION, iv_div DOUBLE PRECISION,
  moneyness DOUBLE PRECISION, dte INT,   -- materialized for partition pruning
  quality_flags INT NOT NULL DEFAULT 0,
  PRIMARY KEY (instrument_id, event_time, knowledge_time)
);
-- Partition: underlying_id / expiry_month / event_date   Sort: dte, moneyness, event_time

CREATE TABLE option_universe_membership (
  universe_id    INT NOT NULL,
  instrument_id  BIGINT NOT NULL,
  valid_from     TIMESTAMP(9) NOT NULL,
  valid_to       TIMESTAMP(9),
  knowledge_time TIMESTAMP(9) NOT NULL,
  PRIMARY KEY (universe_id, instrument_id, valid_from, knowledge_time)
);

-- §1.7 Crypto
CREATE TABLE funding_rate (
  instrument_id BIGINT, venue_id INT,
  event_time TIMESTAMP(9),
  predicted_rate DOUBLE PRECISION,   -- tradeable information
  realized_rate  DOUBLE PRECISION,   -- different knowledge_time
  knowledge_time TIMESTAMP(9) NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, event_time, knowledge_time)
);

CREATE TABLE intraday_seasonal_profile (   -- deflate BEFORE any vol feature [R-02/§5.2]
  venue_id INT, asset_class TEXT,
  dow SMALLINT, minute_of_week INT,
  vol_multiplier DOUBLE PRECISION,
  fitted_from DATE, fitted_to DATE, profile_version TEXT,
  PRIMARY KEY (venue_id, asset_class, minute_of_week, profile_version)
);

-- §1.8 DeFi. Key on block_hash; block_number alone is not a key. [INV-09]
CREATE TABLE chain_block (
  chain_id      INT NOT NULL,
  block_number  BIGINT NOT NULL,
  block_hash    BYTEA NOT NULL,
  parent_hash   BYTEA NOT NULL,
  block_time    TIMESTAMP(9) NOT NULL,
  finalized_at  TIMESTAMP(9),
  orphaned_at   TIMESTAMP(9),
  PRIMARY KEY (chain_id, block_number, block_hash)
);

CREATE TABLE pool_observation (
  chain_id INT NOT NULL, block_number BIGINT NOT NULL, block_hash BYTEA NOT NULL,
  pool_id  BIGINT NOT NULL,
  reserve0 DECIMAL(38,18), reserve1 DECIMAL(38,18),
  tvl_usd  DECIMAL(38,18), fee_bps INT,
  sqrt_price DECIMAL(38,18), liquidity DECIMAL(38,18),
  knowledge_time TIMESTAMP(9) NOT NULL,
  quality_flags INT NOT NULL DEFAULT 0,
  PRIMARY KEY (chain_id, block_number, block_hash, pool_id)
);

/*  §1.9 quality_flags bitmask
    0x0001 STALE_QUOTE       0x0020 SUSPECT_VOLUME   0x0400 REORG_PENDING
    0x0002 CROSSED_BOOK      0x0040 HALTED           0x0800 SYNTHETIC_ROLL
    0x0004 WIDE_SPREAD       0x0080 AUCTION_ONLY     0x1000 VENDOR_REVISED
    0x0008 LOW_UPDATE_COUNT  0x0100 CORP_ACTION_ADJ  0x2000 INTERPOLATED
    0x0010 VENUE_OUTAGE      0x0200 EXPIRY_WEEK      0x4000 QUALITY_TIER_3
    0x8000 BACKFILLED_KNOWLEDGE_TIME   (migration sentinel, CLAUDE.md §6)
*/
