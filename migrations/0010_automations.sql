-- Phase 3 (P3-T05): Automation plans and stateful stage membership.
-- automations holds both SingleInstrument and Pipeline plan specs (JSONB).
-- automation_stage_membership tracks which instruments are in each pipeline stage.

CREATE TABLE IF NOT EXISTS automations (
    id           UUID        PRIMARY KEY,
    user_id      UUID        NOT NULL,
    kind         TEXT        NOT NULL
        CHECK (kind IN ('single_instrument', 'pipeline')),
    account_mode TEXT        NOT NULL
        CHECK (account_mode IN ('paper', 'live')),
    spec         JSONB       NOT NULL,
    armed        BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS automations_user_idx ON automations (user_id);

CREATE TABLE IF NOT EXISTS automation_stage_membership (
    automation_id  UUID        NOT NULL REFERENCES automations (id) ON DELETE CASCADE,
    stage_id       TEXT        NOT NULL,
    instrument_id  TEXT        NOT NULL,
    entered_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (automation_id, stage_id, instrument_id)
);

CREATE INDEX IF NOT EXISTS asm_automation_idx ON automation_stage_membership (automation_id);
