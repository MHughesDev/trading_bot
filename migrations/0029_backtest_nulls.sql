-- Set-J Phase 3: the Null Library registry (spec §2.1).
--
-- A Null is a first-class, reusable, immutable object: kind + params + the
-- explicit preserves/destroys hypothesis. A new params set is a new null_id, so
-- rows are never updated. The per-Experiment CHOICE is logged separately, with
-- an override reason required whenever the chosen null differs from the
-- recommendation — the audit trail behind "the null was selected, not defaulted".

CREATE TABLE IF NOT EXISTS backtest_nulls (
    null_id      TEXT PRIMARY KEY,          -- null:<hex> content hash of kind+params
    kind         TEXT NOT NULL,             -- signal_return_decouple | block_permutation | ...
    params_json  JSONB NOT NULL,
    preserves    TEXT[] NOT NULL,           -- what this null KEEPS intact (non-empty)
    destroys     TEXT[] NOT NULL,           -- what it BREAKS (non-empty)
    created_by   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A null with no stated hypothesis is not a null (the most common silent error).
    CONSTRAINT chk_hypothesis_nonempty
        CHECK (cardinality(preserves) > 0 AND cardinality(destroys) > 0)
);

-- The logged null choice per Experiment. Recommended-not-defaulted: an override
-- (chosen kind <> recommended kind) MUST carry a reason.
CREATE TABLE IF NOT EXISTS backtest_null_choices (
    experiment_id   TEXT PRIMARY KEY,
    chosen_null_id  TEXT NOT NULL REFERENCES backtest_nulls(null_id),
    recommended     TEXT NOT NULL,
    was_override    BOOLEAN NOT NULL,
    override_reason TEXT,
    chosen_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_override_has_reason
        CHECK (NOT was_override OR (override_reason IS NOT NULL AND length(btrim(override_reason)) > 0))
);

CREATE INDEX IF NOT EXISTS idx_backtest_nulls_kind ON backtest_nulls(kind);

-- Null definitions are immutable once written.
CREATE OR REPLACE FUNCTION backtest_nulls_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'backtest_nulls is immutable (null_id=%)', OLD.null_id;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_backtest_nulls_immutable ON backtest_nulls;
CREATE TRIGGER trg_backtest_nulls_immutable
    BEFORE UPDATE OR DELETE ON backtest_nulls
    FOR EACH ROW EXECUTE FUNCTION backtest_nulls_immutable();
