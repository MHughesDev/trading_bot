-- Internal agent runs: an LLM-driven loop that designs strategies, launches
-- backtests, waits on them, and iterates. Modeled on training_runs (0020).

CREATE TABLE IF NOT EXISTS agent_runs (
    run_id            UUID PRIMARY KEY,
    user_id           UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    status            TEXT NOT NULL DEFAULT 'queued',
      -- queued | running | waiting_backtest | completed | failed | cancelled
    goal              TEXT NOT NULL,
    provider          TEXT NOT NULL,
    model             TEXT NOT NULL,
    constraints_json  JSONB NOT NULL DEFAULT '{}'::jsonb,
    iterations        INT NOT NULL DEFAULT 0,
    max_iterations    INT NOT NULL DEFAULT 15,
    tokens_in         BIGINT NOT NULL DEFAULT 0,
    tokens_out        BIGINT NOT NULL DEFAULT 0,
    max_total_tokens  BIGINT,
    wallclock_budget_secs INT NOT NULL DEFAULT 14400,
    error             TEXT,
    summary           TEXT,
    final_strategy_id TEXT,
    best_backtest_id  UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at        TIMESTAMPTZ,
    finished_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS agent_runs_user_idx ON agent_runs (user_id, created_at DESC);

-- Full step-by-step transcript, polled by the UI timeline.
CREATE TABLE IF NOT EXISTS agent_messages (
    id           BIGSERIAL PRIMARY KEY,
    run_id       UUID NOT NULL REFERENCES agent_runs(run_id) ON DELETE CASCADE,
    seq          INT  NOT NULL,
    kind         TEXT NOT NULL,  -- assistant | tool_call | tool_result | status | error | final
    content_json JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, seq)
);
