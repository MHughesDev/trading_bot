-- Set P P-4.3 and P-4.5: as-of universes, and the free-only fence as a column.
--
-- Two unrelated things, both small, both the storage half of a mechanism whose
-- logic already exists and is tested in `crates/dataplane`.
--
-- ## cost_class, and why it has no DEFAULT (P-4.5, DA-14, ADR-0027)
--
-- The whole mechanism is the absence of a default. `crates/dataplane` already
-- refuses to compile a `SourceDecl` without a `cost_class` (`CostClass` has no
-- `Default` and the field is required), and `CostClass::parse` refuses the empty
-- string rather than reading it as free. A DEFAULT 'free' here would undo both:
-- a source added by an INSERT that forgot the column would arrive free, silently,
-- which is the convention this check replaces.
--
-- Every shipped source is `free`, stated one row at a time rather than by a bulk
-- UPDATE, so adding a source means adding a line that says which it is.
--
-- ## universe membership, and why it is an event log (P-4.3, DA-08)
--
-- The plan sketched `(name, instrument_id, valid_from, valid_to, reason)`. That
-- table cannot satisfy its own acceptance criterion: "`valid_to` is never
-- overwritten -- a removal is a new row". An UPDATE that closes `valid_to`
-- destroys the answer a previous study got, which is the survivorship bias this
-- task exists to prevent, one level up. One row per `(universe, instrument)` also
-- cannot represent an instrument that left an index and rejoined it.
--
-- So the write model is append-only and the intervals are derived
-- (`dataplane::universe` folds the log into them, with every `valid_to`
-- computed). The insert-only trigger is what makes "append-only" a mechanism
-- rather than a comment: a rule the application layer enforces is a rule the next
-- migration forgets.
--
-- Two clocks, both required (INV-01): `effective` is when the membership change
-- took effect in the world, `recorded_at` is when this platform learned of it.
-- An index change announced on the 1st and effective on the 15th is knowable from
-- the 1st, and a backtest run on the 10th must see neither the addition nor a
-- membership list that pretends the announcement had not happened.

ALTER TABLE dataplane.source
  ADD COLUMN IF NOT EXISTS cost_class TEXT;

UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'live_aggregator' AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'coinbase_rest'   AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kraken_rest'     AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'binance_rest'    AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'alpaca_rest'     AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'unknown_legacy'  AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'tradier_rest'    AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'onchain_indexer' AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kraken_ws'       AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'synthetic'       AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'alpaca_ws'       AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'oanda_rest'      AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kalshi_rest'     AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'tradovate_rest'  AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'zerox_rest'      AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'e2e_test'        AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'tradier_chain_rest' AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kraken_futures_rest' AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kraken_futures_history' AND cost_class IS NULL;

-- Anything this migration did not name is `free` too, but say so separately: the
-- statements above are the declaration, and this one is the sweep for rows added
-- between 0046 and here. If the two ever disagree, the sweep is the half nobody
-- signed.
UPDATE dataplane.source SET cost_class = 'free' WHERE cost_class IS NULL;

ALTER TABLE dataplane.source
  ALTER COLUMN cost_class SET NOT NULL;

-- No DEFAULT. Deliberate, and the point of the task. See the header.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'source_cost_class_known'
  ) THEN
    ALTER TABLE dataplane.source
      ADD CONSTRAINT source_cost_class_known CHECK (cost_class IN ('free','paid'));
  END IF;
END $$;

COMMENT ON COLUMN dataplane.source.cost_class IS
  'free | paid. No DEFAULT, on purpose (DA-14, ADR-0027): a source added without saying which it is must fail rather than arrive free. A paid source additionally needs an entry in dataplane::source::PAID_SOURCE_ALLOWLIST naming the ADR that approved it.';

-- as-of universes (P-4.3) ----------------------------------------------------

CREATE TABLE IF NOT EXISTS dataplane.universe (
  name        TEXT PRIMARY KEY,
  description TEXT,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS dataplane.universe_membership_event (
  recorded_seq  BIGSERIAL PRIMARY KEY,
  universe      TEXT NOT NULL REFERENCES dataplane.universe(name),
  instrument_id BIGINT NOT NULL,
  -- 'added' | 'removed'. An instrument may be added, removed and added again;
  -- the derived interval model handles that and an interval table could not.
  change        TEXT NOT NULL CHECK (change IN ('added','removed')),
  -- When the change took effect in the world.
  effective     TIMESTAMPTZ NOT NULL,
  -- When this platform learned of it (INV-01). Never defaulted from `effective`:
  -- an index change is usually announced before it takes effect, and collapsing
  -- the two clocks is how an announced-but-not-yet-effective change reaches a
  -- backtest as though it had already happened.
  recorded_at   TIMESTAMPTZ NOT NULL,
  reason        TEXT
);

CREATE INDEX IF NOT EXISTS universe_membership_event_by_universe
  ON dataplane.universe_membership_event (universe, effective, recorded_seq);
CREATE INDEX IF NOT EXISTS universe_membership_event_by_instrument
  ON dataplane.universe_membership_event (instrument_id, effective);

-- Append-only, enforced. "valid_to is never overwritten" is P-4.3's own
-- acceptance criterion, and a criterion the application layer keeps is a
-- criterion the next migration drops.
CREATE OR REPLACE FUNCTION dataplane.universe_membership_is_append_only()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'dataplane.universe_membership_event is append-only (Set P P-4.3, DA-08): % is refused. A membership change is a new row. Rewriting history here destroys the answer a previous study got, which is the survivorship bias this table exists to prevent.', TG_OP;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS universe_membership_append_only ON dataplane.universe_membership_event;
CREATE TRIGGER universe_membership_append_only
  BEFORE UPDATE OR DELETE ON dataplane.universe_membership_event
  FOR EACH ROW EXECUTE FUNCTION dataplane.universe_membership_is_append_only();
