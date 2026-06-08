-- Global and per-user risk configuration.
-- Includes the global kill switch (trading_enabled).
-- All size/rate columns use NUMERIC — never FLOAT.

CREATE TABLE IF NOT EXISTS global_risk_config (
    config_id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Master kill switch.  Setting this to false halts ALL order flow immediately.
    trading_enabled     BOOLEAN NOT NULL DEFAULT true,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by          UUID REFERENCES users(user_id)
);

-- Seed the single-row global config.
INSERT INTO global_risk_config (trading_enabled) VALUES (true)
ON CONFLICT DO NOTHING;

CREATE TABLE IF NOT EXISTS user_risk_limits (
    limit_id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id                     UUID NOT NULL REFERENCES users(user_id) UNIQUE,
    max_position                NUMERIC(30, 10),    -- NULL = no limit; never FLOAT
    max_order_rate_per_minute   INTEGER,
    max_order_rate_per_second   INTEGER,
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now()
);
