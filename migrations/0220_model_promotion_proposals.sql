-- 0220 -- alias promotion is a human decision (Set N N-4.3, MD-13, AGENT-013 ring 1).
--
-- `promote_model_version` moves the `production` alias, which is ring 1. An agent
-- can only *propose* it; a person decides, and the alias write happens at that
-- decision, as that person. Three facts are kept apart, as for every proposal:
-- decided (a person said yes or no), performed (the platform then moved the
-- alias) and what it did (`effect`).

CREATE TABLE IF NOT EXISTS model_promotion_proposals (
    proposal_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id       TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,
    environment    TEXT NOT NULL DEFAULT 'paper',
    rationale      TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending'
                   CHECK (status IN ('pending', 'approved', 'rejected')),
    evidence       JSONB NOT NULL DEFAULT '{}'::jsonb,
    proposed_by    UUID NOT NULL,
    proposed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by     UUID,
    decided_at     TIMESTAMPTZ,
    decision_note  TEXT,
    performed_at   TIMESTAMPTZ,
    effect         JSONB,
    FOREIGN KEY (model_id, version) REFERENCES model_versions(model_id, version) ON DELETE CASCADE,
    -- A decision names who made it and when; pending names neither.
    CONSTRAINT model_promotion_decided_is_complete CHECK (
      (status = 'pending' AND decided_by IS NULL AND decided_at IS NULL)
      OR (status <> 'pending' AND decided_by IS NOT NULL AND decided_at IS NOT NULL)
    ),
    -- Nothing is performed that was not approved.
    CONSTRAINT model_promotion_performed_needs_approval CHECK (
      performed_at IS NULL OR status = 'approved'
    )
);
CREATE INDEX IF NOT EXISTS idx_model_promotion_proposals_model
    ON model_promotion_proposals (model_id, proposed_at DESC);
-- One open proposal per (model, version, environment).
CREATE UNIQUE INDEX IF NOT EXISTS model_promotion_one_pending
    ON model_promotion_proposals (model_id, version, environment) WHERE status = 'pending';

GRANT SELECT, INSERT, UPDATE ON model_promotion_proposals TO app_role;
