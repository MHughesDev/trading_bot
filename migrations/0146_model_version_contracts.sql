-- 0146 — contracts on model versions (Set U U-5.1; ADR-0043 decision 1).
--
-- A strategy may call a model only through a typed contract: what the model
-- takes, what it gives, how long it may take, and when — point-in-time — it may
-- answer. Until now a model version recorded none of that, so every answer was
-- forced into a forecast shape and every missing input was filled with zero.
--
-- `io_contract` is the version's `domain::model_def::contract::IoContract`.
-- `contract_source` says where it came from: `derived` from the kind and
-- definition of a model that predates contracts (filled at platform start by
-- `model_registry::contracts::backfill`), or `declared` by whoever registered
-- the version. A version with neither — a `transform`, which is custom code with
-- nothing to derive from — cannot be referenced by a strategy until one is
-- declared, and the compiler says so.
--
-- `trained_through` is the last instant the version's training data reached.
-- A backtest may ask the version only for decisions after
-- `trained_through + embargo`; with it unknown, the version answers no
-- backtest at all (FEAT-005 §4.3). It is never guessed.

ALTER TABLE model_versions
  ADD COLUMN IF NOT EXISTS io_contract     JSONB,
  ADD COLUMN IF NOT EXISTS contract_source TEXT,
  ADD COLUMN IF NOT EXISTS trained_through TIMESTAMPTZ;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'chk_model_version_contract_source') THEN
    ALTER TABLE model_versions
      ADD CONSTRAINT chk_model_version_contract_source
      CHECK (contract_source IS NULL OR contract_source IN ('derived', 'declared'));
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'chk_model_version_contract_pair') THEN
    ALTER TABLE model_versions
      ADD CONSTRAINT chk_model_version_contract_pair
      CHECK ((io_contract IS NULL) = (contract_source IS NULL));
  END IF;
END $$;
