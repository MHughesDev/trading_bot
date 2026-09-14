-- L0/L1 data plane in Postgres (SPEC §1–§3, §7.3). High-volume series (bars, option
-- bars, pool observations) live in ClickHouse (clickhouse/07_canonical_market.sql);
-- this schema holds identity, bitemporal reference data, and the L1 specs.
--
-- INV-04: symbols are bitemporal attributes, never keys.
-- INV-01: every fact about the world carries knowledge_time.
-- INV-05: prices are NUMERIC(38,18).

CREATE SCHEMA IF NOT EXISTS dataplane;

-- ── sources and their declared vendor lag (OQ-01) ─────────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.source (
  source_id              INT PRIMARY KEY,
  name                   TEXT NOT NULL UNIQUE,
  kind                   TEXT NOT NULL CHECK (kind IN ('live_stream','rest_history','vendor_file','derived')),
  declared_vendor_lag_ms BIGINT NOT NULL CHECK (declared_vendor_lag_ms >= 0),
  notes                  TEXT
);
INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes) VALUES
  (1, 'live_aggregator',  'live_stream',  0,       'trades aggregated to bars in-process; knowledge_time observed at close'),
  (2, 'coinbase_rest',    'rest_history', 60000,   'candles published after the minute closes'),
  (3, 'kraken_rest',      'rest_history', 60000,   NULL),
  (4, 'binance_rest',     'rest_history', 60000,   NULL),
  (5, 'alpaca_rest',      'rest_history', 900000,  'free-tier SIP data is 15-minute delayed'),
  (6, 'unknown_legacy',   'rest_history', 60000,   'pre-ledger rows with no recorded source'),
  (7, 'tradier_rest',     'rest_history', 900000,  'delayed options quotes'),
  (8, 'onchain_indexer',  'live_stream',  0,       'knowledge_time is when the indexer saw the block'),
  (9, 'kraken_ws',        'live_stream',  0,       'bars aggregated from the live Kraken trade stream'),
  (10, 'synthetic',       'derived',      0,       'generated test series; never tradeable'),
  (11, 'alpaca_ws',       'live_stream',  0,       'live Alpaca bar/trade stream'),
  (12, 'oanda_rest',      'rest_history', 5000,    NULL),
  (13, 'kalshi_rest',     'rest_history', 60000,   NULL),
  (14, 'tradovate_rest',  'rest_history', 60000,   NULL),
  (15, 'zerox_rest',      'rest_history', 30000,   'DEX quotes via 0x'),
  (16, 'e2e_test',        'derived',      0,       'integration-test fixture source')
ON CONFLICT (source_id) DO NOTHING;

-- ── identity (§1.1) ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.venue (
  venue_id         INT PRIMARY KEY,
  name             TEXT NOT NULL UNIQUE,
  chain_id         INT,
  session_model    TEXT NOT NULL CHECK (session_model IN ('continuous_24_7','rth_plus_ext','chain_block')),
  calendar_id      TEXT NOT NULL,
  calendar_version TEXT NOT NULL,
  quality_tier     SMALLINT NOT NULL CHECK (quality_tier BETWEEN 1 AND 3)
);
INSERT INTO dataplane.venue (venue_id, name, chain_id, session_model, calendar_id, calendar_version, quality_tier) VALUES
  (1,  'coinbase', NULL, 'continuous_24_7', 'continuous_24_7', '1', 1),
  (2,  'kraken',   NULL, 'continuous_24_7', 'continuous_24_7', '1', 1),
  (3,  'binance',  NULL, 'continuous_24_7', 'continuous_24_7', '1', 2),
  (4,  'alpaca',   NULL, 'rth_plus_ext',    'XNYS',            '2026.1', 1),
  (5,  'oanda',    NULL, 'continuous_24_7', 'fx_24_5',         '1', 2),
  (6,  'cme',      NULL, 'rth_plus_ext',    'CMES',            '2026.1', 1),
  (7,  'opra',     NULL, 'rth_plus_ext',    'XNYS',            '2026.1', 1),
  (8,  'kalshi',   NULL, 'continuous_24_7', 'continuous_24_7', '1', 2),
  (9,  'uniswap_v3_ethereum', 1, 'chain_block', 'chain_block', '1', 2),
  (10, 'unknown',  NULL, 'continuous_24_7', 'continuous_24_7', '1', 3),
  (11, 'synthetic', NULL, 'continuous_24_7', 'continuous_24_7', '1', 3)
ON CONFLICT (venue_id) DO NOTHING;

CREATE SEQUENCE IF NOT EXISTS dataplane.instrument_id_seq START 1000;

CREATE TABLE IF NOT EXISTS dataplane.instrument (
  instrument_id      BIGINT PRIMARY KEY DEFAULT nextval('dataplane.instrument_id_seq'),
  asset_class        TEXT NOT NULL CHECK (asset_class IN ('equity','etf','future','option','crypto','pool','fx','prediction_market')),
  base_instrument_id BIGINT REFERENCES dataplane.instrument(instrument_id),
  first_seen         TIMESTAMPTZ NOT NULL,
  last_seen          TIMESTAMPTZ,
  static_attrs       JSONB NOT NULL DEFAULT '{}'::jsonb
);
-- An identity is never reused and never removed.
CREATE OR REPLACE FUNCTION dataplane.refuse_delete()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% rows are never deleted (identity is never reused)', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_instrument_no_delete ON dataplane.instrument;
CREATE TRIGGER trg_instrument_no_delete BEFORE DELETE ON dataplane.instrument FOR EACH ROW EXECUTE FUNCTION dataplane.refuse_delete();

CREATE TABLE IF NOT EXISTS dataplane.instrument_symbol (
  instrument_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  venue_id       INT    NOT NULL REFERENCES dataplane.venue(venue_id),
  symbol         TEXT   NOT NULL,
  valid_from     TIMESTAMPTZ NOT NULL,
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  backfilled_knowledge_time BOOLEAN NOT NULL DEFAULT FALSE,
  PRIMARY KEY (instrument_id, venue_id, valid_from, knowledge_time),
  CHECK (valid_to IS NULL OR valid_to > valid_from)
);
CREATE INDEX IF NOT EXISTS idx_instrument_symbol_lookup ON dataplane.instrument_symbol (venue_id, symbol, valid_from);

CREATE TABLE IF NOT EXISTS dataplane.instrument_trading_rule (
  instrument_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  venue_id       INT NOT NULL REFERENCES dataplane.venue(venue_id),
  tick_size      NUMERIC(38,18),
  lot_size       NUMERIC(38,18),
  valid_from     TIMESTAMPTZ NOT NULL,
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, valid_from, knowledge_time)
);

-- Resolve (venue, symbol) at an event time, as known by as_of. Mirrors
-- dataplane::identity::resolve_symbol: the latest version of each fact wins.
CREATE OR REPLACE FUNCTION dataplane.resolve_symbol(p_venue INT, p_symbol TEXT, p_at TIMESTAMPTZ, p_as_of TIMESTAMPTZ)
RETURNS BIGINT AS $$
  WITH current_versions AS (
    SELECT DISTINCT ON (instrument_id, venue_id, symbol, valid_from) *
      FROM dataplane.instrument_symbol
     WHERE venue_id = p_venue AND symbol = p_symbol AND knowledge_time <= p_as_of
     ORDER BY instrument_id, venue_id, symbol, valid_from, knowledge_time DESC
  )
  SELECT instrument_id FROM current_versions
   WHERE valid_from <= p_at AND (valid_to IS NULL OR p_at < valid_to)
   ORDER BY valid_from DESC, knowledge_time DESC
   LIMIT 1;
$$ LANGUAGE sql STABLE;

-- Backfill identity for the platform's existing catalog. Their true knowledge time
-- was never recorded. Using the catalog row's creation time would make every
-- historical PIT read fail to resolve the symbol, so the sentinel is the start of
-- validity (CLAUDE.md §6: event_time + declared lag), and the row is flagged.
DO $$
DECLARE r RECORD; new_id BIGINT; v INT; cls TEXT;
BEGIN
  FOR r IN SELECT instrument_id AS sym, asset_class, venue_id AS venue_name, created_at FROM public.instruments LOOP
    SELECT venue_id INTO v FROM dataplane.venue WHERE name = r.venue_name;
    IF v IS NULL THEN v := 10; END IF;
    IF EXISTS (SELECT 1 FROM dataplane.instrument_symbol WHERE venue_id = v AND symbol = r.sym) THEN
      CONTINUE;
    END IF;
    cls := CASE r.asset_class
      WHEN 'equity' THEN 'equity' WHEN 'etf' THEN 'etf'
      WHEN 'futures_expiring' THEN 'future' WHEN 'option' THEN 'option'
      WHEN 'crypto_spot_cex' THEN 'crypto' WHEN 'perpetual_swap' THEN 'crypto'
      WHEN 'fx' THEN 'fx' WHEN 'prediction_market' THEN 'prediction_market'
      ELSE 'crypto' END;
    INSERT INTO dataplane.instrument (asset_class, first_seen, static_attrs)
      VALUES (cls, r.created_at, jsonb_build_object('backfilled_identity', true, 'platform_asset_class', r.asset_class))
      RETURNING instrument_id INTO new_id;
    INSERT INTO dataplane.instrument_symbol (instrument_id, venue_id, symbol, valid_from, knowledge_time, backfilled_knowledge_time)
      VALUES (new_id, v, r.sym, '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', TRUE);
  END LOOP;
END $$;

-- The trading engine keeps its symbol-keyed catalog (out of the pack's scope); it
-- links to the surrogate identity rather than defining one.
ALTER TABLE public.instruments ADD COLUMN IF NOT EXISTS instrument_key BIGINT REFERENCES dataplane.instrument(instrument_id);
UPDATE public.instruments i
   SET instrument_key = s.instrument_id
  FROM dataplane.instrument_symbol s
  JOIN dataplane.venue v ON v.venue_id = s.venue_id
 WHERE s.symbol = i.instrument_id AND v.name = i.venue_id AND i.instrument_key IS NULL;

-- ── equities / ETFs (§1.4) ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.corporate_action (
  action_id         BIGSERIAL PRIMARY KEY,
  instrument_id     BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  action_type       TEXT NOT NULL CHECK (action_type IN ('split','dividend','spinoff','merger','symbol_change','delist')),
  announcement_time TIMESTAMPTZ NOT NULL,
  ex_date           DATE NOT NULL,
  effective_time    TIMESTAMPTZ NOT NULL,
  price_factor      NUMERIC(38,18),
  volume_factor     NUMERIC(38,18),
  cash_amount       NUMERIC(38,18),
  currency          TEXT,
  knowledge_time    TIMESTAMPTZ NOT NULL,
  revision_seq      INT NOT NULL DEFAULT 0,
  CHECK (announcement_time <= effective_time),
  UNIQUE (instrument_id, action_type, ex_date, revision_seq)
);

CREATE TABLE IF NOT EXISTS dataplane.index_membership (
  index_id          INT NOT NULL,
  instrument_id     BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  announcement_time TIMESTAMPTZ NOT NULL,
  effective_from    TIMESTAMPTZ NOT NULL,
  effective_to      TIMESTAMPTZ,
  weight            NUMERIC(38,18),
  knowledge_time    TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (index_id, instrument_id, effective_from, knowledge_time)
);

CREATE TABLE IF NOT EXISTS dataplane.session_event (
  instrument_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  venue_id       INT NOT NULL REFERENCES dataplane.venue(venue_id),
  event_time     TIMESTAMPTZ NOT NULL,
  event_type     TEXT NOT NULL CHECK (event_type IN ('halt','luld','auction','resume','odd_lot')),
  detail         JSONB NOT NULL DEFAULT '{}'::jsonb,
  knowledge_time TIMESTAMPTZ NOT NULL
);

-- ── futures (§1.5): continuous series are views, never facts ──────────────────
CREATE TABLE IF NOT EXISTS dataplane.future_contract (
  instrument_id  BIGINT PRIMARY KEY REFERENCES dataplane.instrument(instrument_id),
  root           TEXT NOT NULL,
  expiry         DATE NOT NULL,
  first_notice   DATE,
  last_trade     DATE,
  contract_size  NUMERIC(38,18),
  tick_value     NUMERIC(38,18)
);

CREATE TABLE IF NOT EXISTS dataplane.roll_schedule (
  root               TEXT NOT NULL,
  roll_rule          TEXT NOT NULL CHECK (roll_rule IN ('calendar','oi_crossover','volume_crossover')),
  from_instrument_id BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  to_instrument_id   BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  observation_time   TIMESTAMPTZ NOT NULL,
  decision_time      TIMESTAMPTZ NOT NULL,
  roll_event_time    TIMESTAMPTZ NOT NULL,
  ratio_factor       NUMERIC(38,18),
  knowledge_time     TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (root, roll_rule, roll_event_time, knowledge_time),
  CHECK (decision_time <= roll_event_time),
  CHECK (knowledge_time <= roll_event_time),
  -- Open-interest and volume are published with at least a day's lag: firing on
  -- same-day OI is look-ahead (R-09, AT-07).
  CHECK (roll_rule = 'calendar' OR decision_time >= observation_time + INTERVAL '1 day')
);

-- ── options (§1.6): contracts and universe membership here; bars in ClickHouse ─
CREATE TABLE IF NOT EXISTS dataplane.option_contract (
  instrument_id  BIGINT PRIMARY KEY REFERENCES dataplane.instrument(instrument_id),
  underlying_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  expiry         DATE NOT NULL,
  strike         NUMERIC(38,18) NOT NULL,
  option_right   CHAR(1) NOT NULL CHECK (option_right IN ('C','P')),
  exercise_style CHAR(1) NOT NULL CHECK (exercise_style IN ('A','E')),
  multiplier     INT NOT NULL,
  occ_symbol     TEXT,
  adjusted_flag  BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS dataplane.option_universe_membership (
  universe_id    INT NOT NULL,
  instrument_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  valid_from     TIMESTAMPTZ NOT NULL,
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  gate_version   TEXT NOT NULL,
  PRIMARY KEY (universe_id, instrument_id, valid_from, knowledge_time)
);

-- ── crypto (§1.7) ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.funding_rate (
  instrument_id   BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  venue_id        INT NOT NULL REFERENCES dataplane.venue(venue_id),
  kind            TEXT NOT NULL CHECK (kind IN ('predicted','realized')),
  settlement_time TIMESTAMPTZ NOT NULL,
  rate            NUMERIC(38,18) NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, kind, settlement_time, knowledge_time),
  -- A realized rate cannot be known before settlement.
  CHECK (kind = 'predicted' OR knowledge_time >= settlement_time)
);

CREATE TABLE IF NOT EXISTS dataplane.listing_membership (
  instrument_id  BIGINT NOT NULL REFERENCES dataplane.instrument(instrument_id),
  venue_id       INT NOT NULL REFERENCES dataplane.venue(venue_id),
  listed_from    TIMESTAMPTZ NOT NULL,
  delisted_at    TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, listed_from, knowledge_time)
);

CREATE TABLE IF NOT EXISTS dataplane.intraday_seasonal_profile (
  venue_id        INT NOT NULL REFERENCES dataplane.venue(venue_id),
  asset_class     TEXT NOT NULL,
  profile_version TEXT NOT NULL,
  season_slot     INT NOT NULL,
  vol_multiplier  DOUBLE PRECISION NOT NULL CHECK (vol_multiplier > 0),
  fitted_from     TIMESTAMPTZ NOT NULL,
  fitted_to       TIMESTAMPTZ NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (venue_id, asset_class, profile_version, season_slot),
  CHECK (knowledge_time >= fitted_to)
);

-- ── DeFi (§1.8): the block hash is part of every key ──────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.chain_block (
  chain_id       INT NOT NULL,
  block_number   BIGINT NOT NULL,
  block_hash     BYTEA NOT NULL,
  parent_hash    BYTEA NOT NULL,
  block_time     TIMESTAMPTZ NOT NULL,
  finalized_at   TIMESTAMPTZ,
  orphaned_at    TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (chain_id, block_number, block_hash),
  CHECK (NOT (finalized_at IS NOT NULL AND orphaned_at IS NOT NULL))
);
-- Blocks are facts: only finality and orphaning may be recorded later, once each.
CREATE OR REPLACE FUNCTION dataplane.chain_block_guard()
RETURNS TRIGGER AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN RAISE EXCEPTION 'chain blocks are never deleted; reorged blocks get orphaned_at'; END IF;
  IF NEW.block_hash <> OLD.block_hash OR NEW.parent_hash <> OLD.parent_hash OR NEW.block_time <> OLD.block_time
     OR NEW.knowledge_time <> OLD.knowledge_time
     OR (OLD.finalized_at IS NOT NULL AND NEW.finalized_at IS DISTINCT FROM OLD.finalized_at)
     OR (OLD.orphaned_at IS NOT NULL AND NEW.orphaned_at IS DISTINCT FROM OLD.orphaned_at) THEN
    RAISE EXCEPTION 'chain_block facts are immutable once recorded';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_chain_block_guard ON dataplane.chain_block;
CREATE TRIGGER trg_chain_block_guard BEFORE UPDATE OR DELETE ON dataplane.chain_block FOR EACH ROW EXECUTE FUNCTION dataplane.chain_block_guard();

-- Trials whose datasets consumed a block, so a reorg can flag them (§17, AT-09).
CREATE TABLE IF NOT EXISTS dataplane.block_consumption (
  trial_id     UUID NOT NULL,
  chain_id     INT NOT NULL,
  block_number BIGINT NOT NULL,
  block_hash   BYTEA NOT NULL,
  PRIMARY KEY (trial_id, chain_id, block_number, block_hash)
);

-- ── L1: features, labels, splits, datasets (§3) ────────────────────────────────
CREATE TABLE IF NOT EXISTS dataplane.feature_def (
  feature_id       TEXT NOT NULL,
  version          INT  NOT NULL,
  -- Every implementation that ever served a value is registered, so a logged
  -- code_hash always resolves to a definition.
  code_hash        TEXT NOT NULL,
  lookback_bars    INT  NOT NULL CHECK (lookback_bars >= 1),
  knowledge_lag_ms BIGINT NOT NULL CHECK (knowledge_lag_ms >= 0),
  output_dtype     TEXT NOT NULL,
  asset_classes    TEXT[] NOT NULL,
  deflators        TEXT[] NOT NULL DEFAULT '{}',
  info_class       TEXT NOT NULL CHECK (info_class IN
                     ('platform_physics','methodology','market_public','strategy_content','performance_conditional','tenant_operational')),
  registered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (feature_id, version, code_hash)
);
-- A registered definition never changes meaning; a change is a new feature_id/version.
DROP TRIGGER IF EXISTS trg_feature_def_immutable ON dataplane.feature_def;
CREATE TRIGGER trg_feature_def_immutable BEFORE UPDATE OR DELETE ON dataplane.feature_def FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE TABLE IF NOT EXISTS dataplane.label_spec (
  label_spec_id        TEXT PRIMARY KEY,
  kind                 TEXT NOT NULL CHECK (kind IN ('triple_barrier','horizon_return','meta_label','custom')),
  horizon_bars         INT NOT NULL CHECK (horizon_bars >= 1),
  pt_sl_multiples      DOUBLE PRECISION[],
  vol_estimator        TEXT,
  min_return_threshold DOUBLE PRECISION,
  sample_weight_method TEXT NOT NULL CHECK (sample_weight_method IN ('uniqueness','return_attribution','time_decay','none')),
  code_hash            TEXT NOT NULL,
  CHECK (kind <> 'triple_barrier' OR cardinality(pt_sl_multiples) = 2)
);

CREATE TABLE IF NOT EXISTS dataplane.split_spec (
  split_spec_id          TEXT PRIMARY KEY,
  kind                   TEXT NOT NULL CHECK (kind IN ('walk_forward','purged_kfold','cpcv','holdout','sealed')),
  n_folds                INT,
  n_test_groups          INT,
  train_window           TEXT NOT NULL,
  computed_embargo_bars  INT NOT NULL CHECK (computed_embargo_bars >= 1),
  embargo_bars           INT NOT NULL,
  embargo_override_reason TEXT,
  purge_on               TEXT NOT NULL DEFAULT 't1' CHECK (purge_on IN ('t1','t0')),
  purge_override_reason  TEXT,
  min_train_bars         INT,
  regime_stratified      BOOLEAN NOT NULL DEFAULT FALSE,
  -- Lowering the computed embargo, or purging on t0, requires a written reason (INV-15).
  CHECK (embargo_bars >= computed_embargo_bars OR length(trim(coalesce(embargo_override_reason, ''))) > 0),
  CHECK (purge_on = 't1' OR length(trim(coalesce(purge_override_reason, ''))) > 0),
  CHECK (kind <> 'cpcv' OR (n_test_groups > 0 AND n_test_groups < n_folds))
);

CREATE TABLE IF NOT EXISTS dataplane.dataset_spec (
  dataset_id             TEXT PRIMARY KEY,
  tenant_id              TEXT NOT NULL,
  spec                   JSONB NOT NULL,
  instrument_ids         BIGINT[] NOT NULL,
  date_from              TIMESTAMPTZ NOT NULL,
  date_to                TIMESTAMPTZ NOT NULL,
  frequency              TEXT NOT NULL,
  feature_set_id         TEXT NOT NULL,
  label_spec_id          TEXT REFERENCES dataplane.label_spec(label_spec_id),
  split_spec_id          TEXT REFERENCES dataplane.split_spec(split_spec_id),
  as_of_knowledge_time   TIMESTAMPTZ NOT NULL,
  quality_exclusion_mask BIGINT NOT NULL,
  calendar_versions      JSONB NOT NULL,
  adjustment_policy      TEXT NOT NULL CHECK (adjustment_policy IN ('unadjusted','splits_only','splits_and_dividends')),
  finality_policy        JSONB,
  runtime_image_digest   TEXT NOT NULL,
  non_reproducible       BOOLEAN NOT NULL DEFAULT FALSE,
  uses_backfilled_knowledge BOOLEAN NOT NULL DEFAULT FALSE,
  created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (date_from < date_to)
);
DROP TRIGGER IF EXISTS trg_dataset_spec_immutable ON dataplane.dataset_spec;
CREATE TRIGGER trg_dataset_spec_immutable BEFORE UPDATE OR DELETE ON dataplane.dataset_spec FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── one code path: serving log + nightly consistency diff (§3.3, INV-14) ──────
CREATE TABLE IF NOT EXISTS dataplane.feature_serving_log (
  serve_id       UUID PRIMARY KEY,
  tenant_id      TEXT NOT NULL,
  instrument_id  BIGINT NOT NULL,
  -- venue and bar period of the rows read: the diff job needs both to recompute.
  venue_id       INT NOT NULL,
  period_secs    INT NOT NULL CHECK (period_secs > 0),
  event_time     TIMESTAMPTZ NOT NULL,
  knowledge_time TIMESTAMPTZ NOT NULL,
  feature_set_id TEXT NOT NULL,
  code_hashes    JSONB NOT NULL,
  feature_values JSONB NOT NULL,
  served_at      TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_serving_log_served_at ON dataplane.feature_serving_log (served_at);
DROP TRIGGER IF EXISTS trg_serving_log_immutable ON dataplane.feature_serving_log;
CREATE TRIGGER trg_serving_log_immutable BEFORE UPDATE OR DELETE ON dataplane.feature_serving_log FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE TABLE IF NOT EXISTS dataplane.feature_consistency_diff (
  serve_id                  UUID NOT NULL REFERENCES dataplane.feature_serving_log(serve_id),
  feature_id                TEXT NOT NULL,
  tenant_id                 TEXT NOT NULL,
  served_value              DOUBLE PRECISION,
  recomputed_value          DOUBLE PRECISION,
  abs_diff                  DOUBLE PRECISION,
  served_knowledge_time     TIMESTAMPTZ NOT NULL,
  recomputed_knowledge_time TIMESTAMPTZ NOT NULL,
  diagnosis                 TEXT NOT NULL CHECK (diagnosis IN ('match','late_arrival','code_drift','nondeterminism','precision')),
  diffed_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (serve_id, feature_id)
);

-- ── the feature firewall (§7.3, INV-24, AT-36) ─────────────────────────────────
CREATE OR REPLACE VIEW mlops.firewall_violations WITH (security_invoker = true) AS
SELECT m.model_id, fl.feature_id, f.info_class
FROM mlops.internal_model_registry m
CROSS JOIN LATERAL unnest(m.feature_list) AS fl(feature_id)
LEFT JOIN dataplane.feature_def f ON f.feature_id = fl.feature_id
WHERE m.scope IN ('global','hierarchical')
  AND (f.feature_id IS NULL OR f.info_class NOT IN ('platform_physics','methodology','market_public'));

-- A registry row whose global feature list violates the firewall cannot be written.
CREATE OR REPLACE FUNCTION mlops.enforce_feature_firewall()
RETURNS TRIGGER AS $$
DECLARE bad TEXT;
BEGIN
  IF NEW.scope IN ('global','hierarchical') THEN
    SELECT string_agg(fl.feature_id || '(' || coalesce(f.info_class, 'unregistered') || ')', ', ') INTO bad
      FROM unnest(NEW.feature_list) AS fl(feature_id)
      LEFT JOIN dataplane.feature_def f ON f.feature_id = fl.feature_id
     WHERE f.feature_id IS NULL OR f.info_class NOT IN ('platform_physics','methodology','market_public');
    IF bad IS NOT NULL THEN
      RAISE EXCEPTION 'feature firewall violation: % model % uses %', NEW.scope, NEW.model_id, bad;
    END IF;
    -- A tenant identifier in any encoding is a one-hot invitation to memorize a tenant (ADR-018, AT-40).
    IF EXISTS (SELECT 1 FROM unnest(NEW.feature_list) fl(feature_id) WHERE fl.feature_id ~* 'tenant|user_id|account_id|created_by') THEN
      RAISE EXCEPTION 'global model % may not use a tenant identifier feature', NEW.model_id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_feature_firewall ON mlops.internal_model_registry;
CREATE TRIGGER trg_feature_firewall BEFORE INSERT OR UPDATE ON mlops.internal_model_registry FOR EACH ROW EXECUTE FUNCTION mlops.enforce_feature_firewall();

-- ── grants and tenancy ─────────────────────────────────────────────────────────
GRANT USAGE ON SCHEMA dataplane TO app_role, internal_ml_role, backtest_role, agent_role;
GRANT SELECT ON ALL TABLES IN SCHEMA dataplane TO app_role, internal_ml_role, backtest_role;
GRANT SELECT ON dataplane.venue, dataplane.instrument, dataplane.instrument_symbol, dataplane.feature_def,
  dataplane.label_spec, dataplane.split_spec TO agent_role;
GRANT INSERT ON dataplane.instrument, dataplane.instrument_symbol, dataplane.instrument_trading_rule,
  dataplane.corporate_action, dataplane.index_membership, dataplane.session_event, dataplane.future_contract,
  dataplane.roll_schedule, dataplane.option_contract, dataplane.option_universe_membership, dataplane.funding_rate,
  dataplane.listing_membership, dataplane.intraday_seasonal_profile, dataplane.chain_block,
  dataplane.block_consumption, dataplane.feature_def, dataplane.label_spec, dataplane.split_spec,
  dataplane.dataset_spec, dataplane.feature_serving_log, dataplane.feature_consistency_diff TO app_role;
GRANT UPDATE (finalized_at, orphaned_at) ON dataplane.chain_block TO app_role;
GRANT UPDATE (last_seen) ON dataplane.instrument TO app_role;
GRANT USAGE, SELECT ON SEQUENCE dataplane.instrument_id_seq, dataplane.corporate_action_action_id_seq TO app_role;
GRANT SELECT ON mlops.firewall_violations TO app_role, internal_ml_role;

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['dataset_spec','feature_serving_log','feature_consistency_diff'] LOOP
    EXECUTE format('ALTER TABLE dataplane.%I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE dataplane.%I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON dataplane.%I', t);
    EXECUTE format($p$
      CREATE POLICY tenant_isolation ON dataplane.%I
        USING (tenant_id = current_setting('app.tenant_id', true))
        WITH CHECK (tenant_id = current_setting('app.tenant_id', true))
    $p$, t);
  END LOOP;
END $$;
