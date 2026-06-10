-- Phase 3 (P3-T02): Compiled capability manifests for every saved strategy.
-- This table is written at strategy-save time by the manifest compiler and
-- read by the apply-list endpoint for compatibility filtering.

CREATE TABLE IF NOT EXISTS strategy_manifests (
    strategy_id         UUID        PRIMARY KEY,
    required_lanes      JSONB       NOT NULL DEFAULT '[]',
    required_primitives JSONB       NOT NULL DEFAULT '[]',
    required_features   JSONB       NOT NULL DEFAULT '[]',
    evaluation_trigger  TEXT        NOT NULL
        CHECK (evaluation_trigger IN ('bar_close', 'tick', 'quote', 'event', 'scheduled')),
    strategy_kind       TEXT        NOT NULL
        CHECK (strategy_kind IN ('discovery', 'execution')),
    compiled_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
