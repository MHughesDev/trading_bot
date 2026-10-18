-- 0212 — the evidence a skill is promoted, retired and re-evaluated on (Set R-b
-- RB-2.1, RB-2.4, RB-2.5, RB-2.6; AGENT-003 §6.4).
--
-- Promotion to global is the one place a single mistake is replicated into every
-- session, so each of its six conditions is a *recorded row*, and the promotion
-- reads rows rather than trusting a caller's claim. Paired evaluation outcomes live
-- here too, because the same pairs back the replay verdict (RA-3.4), the savings
-- condition (RA-3.6) and the A/B retirement rule (RB-2.4).
--
-- Granted to app_role in this file; no agent role has any access.

CREATE TABLE IF NOT EXISTS skill_registry.skill_promotion_condition (
  record_id   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  skill_id    UUID NOT NULL REFERENCES skill_registry.skill (skill_id),
  condition   TEXT NOT NULL CHECK (condition IN
              ('cross_project_evals', 'sanitiser', 'owner_consent',
               'security_scan', 'independent_reviewer', 'no_taint')),
  passed      BOOLEAN NOT NULL,
  recorded_by TEXT NOT NULL CHECK (btrim(recorded_by) <> ''),
  evidence    JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skill_promotion_condition_skill
  ON skill_registry.skill_promotion_condition (skill_id, condition, record_id DESC);

-- The findings a skill's provenance cites, so no_taint is computed from the memory
-- store's tainted mark rather than asserted by whoever asks for the promotion.
CREATE TABLE IF NOT EXISTS skill_registry.skill_provenance (
  skill_id   UUID NOT NULL REFERENCES skill_registry.skill (skill_id),
  finding_id TEXT NOT NULL CHECK (btrim(finding_id) <> ''),
  tainted    BOOLEAN NOT NULL,
  PRIMARY KEY (skill_id, finding_id)
);

-- A pair belongs to a proposal (verification before admission: RA-3.4) or to an
-- admitted skill (A/B, re-evaluation: RB-2.4), never both and never neither.
CREATE TABLE IF NOT EXISTS skill_registry.skill_eval_pair (
  pair_id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  proposal_id     UUID REFERENCES skill_registry.skill_proposal (proposal_id),
  skill_id        UUID REFERENCES skill_registry.skill (skill_id),
  version_hash    TEXT REFERENCES skill_registry.skill_version (version_hash),
  kind            TEXT NOT NULL CHECK (kind IN ('replay', 'ab', 'reeval', 'leave_one_out')),
  model           TEXT NOT NULL CHECK (btrim(model) <> ''),
  variant         TEXT NOT NULL CHECK (btrim(variant) <> ''),
  activated       BOOLEAN NOT NULL,
  with_success    BOOLEAN NOT NULL,
  without_success BOOLEAN NOT NULL,
  with_cost       DOUBLE PRECISION NOT NULL CHECK (with_cost >= 0),
  without_cost    DOUBLE PRECISION NOT NULL CHECK (without_cost >= 0),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT pair_has_one_subject CHECK (num_nonnulls(proposal_id, skill_id) = 1)
);
CREATE INDEX IF NOT EXISTS skill_eval_pair_skill ON skill_registry.skill_eval_pair (skill_id, kind, pair_id);
CREATE INDEX IF NOT EXISTS skill_eval_pair_proposal ON skill_registry.skill_eval_pair (proposal_id, kind, pair_id);

-- Every move of a skill to a wider scope, and who approved it (RB-2.2: "the owner's
-- approval is audited"). Not approval_requests: that table is project-bound with a
-- foreign key to research_projects and answered by the desk's approval inbox, which
-- does not know what a skill is.
CREATE TABLE IF NOT EXISTS skill_registry.skill_promotion_log (
  log_id      BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  skill_id    UUID NOT NULL REFERENCES skill_registry.skill (skill_id),
  from_scope  TEXT NOT NULL,
  to_scope    TEXT NOT NULL CHECK (to_scope IN ('user', 'global')),
  approved_by UUID NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A run of the whole library on one model (SK-17). A model default change reads the
-- latest completed row for the model.
CREATE TABLE IF NOT EXISTS skill_registry.skill_library_reeval (
  reeval_id    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  model        TEXT NOT NULL CHECK (btrim(model) <> ''),
  completed    BOOLEAN NOT NULL,
  failing      JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(failing) = 'array'),
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skill_library_reeval_model ON skill_registry.skill_library_reeval (model, reeval_id DESC);

GRANT SELECT, INSERT ON skill_registry.skill_promotion_condition TO app_role;
GRANT SELECT, INSERT, UPDATE ON skill_registry.skill_provenance TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_eval_pair TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_promotion_log TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_library_reeval TO app_role;
