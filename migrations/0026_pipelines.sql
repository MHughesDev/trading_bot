-- Pipeline factory schema (Set I, Phase 5).
--
-- The PipelineManager (crates/model-registry/src/pipeline_manager.rs) was
-- shipped using runtime sqlx queries against these tables, but no migration
-- ever created them. This migration backfills the schema so the MLOps
-- Automation surface (declarative training/inference pipelines, fan-out, run
-- history) is actually runnable.
--
-- Idempotent (IF NOT EXISTS) so it is safe to apply on top of databases where
-- the tables may have been created out-of-band during development.

-- Pipeline definitions.
CREATE TABLE IF NOT EXISTS pipelines (
    id              TEXT PRIMARY KEY,
    name            TEXT        NOT NULL,
    kind            TEXT        NOT NULL,
    created_by      TEXT        NOT NULL,
    definition_json JSONB       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pipelines_created_by_idx ON pipelines (created_by);

-- Pipeline runs (one row per matrix cell; fan-out parents link children via
-- parent_run_id).
CREATE TABLE IF NOT EXISTS pipeline_runs (
    id            TEXT PRIMARY KEY,
    pipeline_id   TEXT        NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    parent_run_id TEXT,
    cell_label    TEXT        NOT NULL DEFAULT 'default',
    status        TEXT        NOT NULL,
    cached        BOOLEAN     NOT NULL DEFAULT false,
    cell_json     JSONB,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at   TIMESTAMPTZ,
    error         TEXT
);

CREATE INDEX IF NOT EXISTS pipeline_runs_pipeline_id_idx ON pipeline_runs (pipeline_id);
CREATE INDEX IF NOT EXISTS pipeline_runs_parent_idx ON pipeline_runs (parent_run_id);

-- Per-node execution records within a run.
CREATE TABLE IF NOT EXISTS pipeline_node_runs (
    id          TEXT PRIMARY KEY,
    run_id      TEXT        NOT NULL REFERENCES pipeline_runs (id) ON DELETE CASCADE,
    node_id     TEXT        NOT NULL,
    op          TEXT        NOT NULL,
    status      TEXT        NOT NULL,
    started_at  TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    error       TEXT
);

CREATE INDEX IF NOT EXISTS pipeline_node_runs_run_id_idx ON pipeline_node_runs (run_id);
