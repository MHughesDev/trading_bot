-- Strategy definitions and active instances.

CREATE TABLE IF NOT EXISTS strategy_definitions (
    strategy_id         TEXT PRIMARY KEY,
    definition_version  TEXT NOT NULL DEFAULT '1.0',
    asset_class         TEXT NOT NULL,
    min_trust_tier      TEXT NOT NULL DEFAULT 'centralized_exchange',
    definition_json     JSONB NOT NULL,     -- serialized StrategyDefinition
    created_by          UUID REFERENCES users(user_id),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- An instance is one strategy_definition bound to a specific instrument.
CREATE TABLE IF NOT EXISTS strategy_instances (
    instance_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    strategy_id     TEXT NOT NULL REFERENCES strategy_definitions(strategy_id),
    account_id      UUID NOT NULL REFERENCES accounts(account_id),
    instrument_id   TEXT NOT NULL REFERENCES instruments(instrument_id),
    state           TEXT NOT NULL DEFAULT 'stopped',   -- 'running' | 'stopped' | 'error'
    started_at      TIMESTAMPTZ,
    stopped_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS strategy_instances_account
    ON strategy_instances(account_id, state);
