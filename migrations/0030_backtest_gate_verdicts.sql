-- Set-J Phase 4: the staged-gate funnel ledger (spec §2.2 / §2.3).
--
-- Each gate's entry requires the prior gate's PASSING verdict, which only exists
-- if its Studies ran (D-8). Persisting verdicts here makes the funnel auditable
-- and lets the workbench (Phase 5) render the gate board with each gate locked
-- until its prerequisite has a passing row.

CREATE TABLE IF NOT EXISTS backtest_gate_verdicts (
    experiment_id   TEXT NOT NULL REFERENCES backtest_experiments(experiment_id),
    gate            TEXT NOT NULL,        -- integrity | single_path | robustness | significance | vault
    passed          BOOLEAN NOT NULL,
    summary         TEXT NOT NULL,
    evidence        TEXT[] NOT NULL DEFAULT '{}',  -- study/run ids constituting the evidence
    -- Gate 3 only: the inseparable significance triple + corroborators (INV-3).
    p_value             DOUBLE PRECISION,  -- selection-bias-corrected
    raw_p_value         DOUBLE PRECISION,
    null_ref            TEXT,
    trial_count_at_eval BIGINT,
    deflated_sharpe     DOUBLE PRECISION,
    pbo                 DOUBLE PRECISION,
    corroborators_agree BOOLEAN,
    decided_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (experiment_id, gate, decided_at),
    -- A significance verdict must carry its null AND its trial count, or neither
    -- (INV-3: significance is never naked).
    CONSTRAINT chk_significance_never_naked CHECK (
        gate <> 'significance'
        OR (p_value IS NOT NULL AND null_ref IS NOT NULL AND trial_count_at_eval IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_backtest_gate_verdicts_exp ON backtest_gate_verdicts(experiment_id, gate);
