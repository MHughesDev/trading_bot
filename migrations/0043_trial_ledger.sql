-- ML platform L2: the Trial Ledger and MLOps governance tables (SPEC §4, §9, §10,
-- §12.3, §12.7, §14, §15). Everything lives in schema `mlops` so grants can be
-- expressed per schema (migration 0044).
--
-- Shape decisions (decisions/ADR-INDEX.md ADR-P0-01, ADR-P0-05, ADR-P0-12):
--   * mlops.trial holds REGISTRATION FACTS ONLY and is immutable: no UPDATE path at
--     any level (AT-20). Lifecycle is mlops.trial_event, append-only, one row per
--     §9 transition, validated against the state machine by trigger.
--   * Both tables are hash-chained by trigger. A trial's first event chains from the
--     trial's own row_hash, so events cannot be re-parented.
--   * Deduplicated dispatches still get a trial row (a look is a look, §12.4).
--   * Propensity and delta_practical may be NULL only under policy 'legacy_unlogged'.

CREATE SCHEMA IF NOT EXISTS mlops;

-- ── outcome vector (§4.3): typed, and there is NO score column (AT-34) ───────
DO $$ BEGIN
  CREATE TYPE mlops.outcome_vector AS (
    auc DOUBLE PRECISION, logloss DOUBLE PRECISION, brier DOUBLE PRECISION,
    ece DOUBLE PRECISION, ic DOUBLE PRECISION, ic_ir DOUBLE PRECISION,
    sharpe_net DOUBLE PRECISION, sortino_net DOUBLE PRECISION, calmar DOUBLE PRECISION,
    psr DOUBLE PRECISION, dsr DOUBLE PRECISION, pbo DOUBLE PRECISION,
    max_dd DOUBLE PRECISION, dd_duration_days INT, turnover_annual DOUBLE PRECISION,
    capacity_usd DOUBLE PRECISION, breakeven_cost_multiple DOUBLE PRECISION,
    seed_sharpe_std DOUBLE PRECISION, regime_pnl_hhi DOUBLE PRECISION,
    cpcv_p05_sharpe DOUBLE PRECISION, bootstrap_p05_sharpe DOUBLE PRECISION,
    param_cliff_score DOUBLE PRECISION,
    alpha_t_stat DOUBLE PRECISION, factor_r2 DOUBLE PRECISION,
    inference_latency_p99_ms DOUBLE PRECISION, model_bytes BIGINT,
    train_gpu_seconds DOUBLE PRECISION
  );
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

-- Shared guard for append-only tables.
CREATE OR REPLACE FUNCTION mlops.refuse_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % refused (corrections append; see SPEC §4.2·8)', TG_TABLE_NAME, TG_OP;
END;
$$ LANGUAGE plpgsql;

-- ── gate profiles (§12.3): immutable and versioned (INV-23, AT-29) ────────────
CREATE TABLE IF NOT EXISTS mlops.gate_profile (
  profile_id  TEXT PRIMARY KEY,
  thresholds  JSONB NOT NULL,
  supersedes  TEXT REFERENCES mlops.gate_profile(profile_id),
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_by  TEXT NOT NULL
);
DROP TRIGGER IF EXISTS trg_gate_profile_immutable ON mlops.gate_profile;
CREATE TRIGGER trg_gate_profile_immutable BEFORE UPDATE OR DELETE ON mlops.gate_profile
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

INSERT INTO mlops.gate_profile (profile_id, thresholds, created_by) VALUES ('strict_v1', '{
  "preregistration_required": true,
  "leakage_suite_pass_rate": 1.0,
  "min_breakeven_cost_multiple": 3.0,
  "max_capacity_fraction_at_half_sharpe": 0.20,
  "adv_soft": 0.05, "adv_hard": 0.10,
  "cpcv_p05_sharpe_gt": 0.0,
  "walk_forward_sharpe_gt": 0.0, "walk_forward_min_regimes": 3,
  "pbo_lt": 0.20,
  "dsr_gte": 0.95,
  "min_track_record_years": 5.0, "min_independent_events": 300,
  "alpha_t_stat_gte": 3.0, "factor_r2_lt": 0.7,
  "max_single_regime_pnl_share": 0.50,
  "max_single_instrument_pnl_share": 0.20,
  "bootstrap_p05_sharpe_gt": 0.0,
  "romano_wolf_p_lt": 0.05,
  "paper_signal_match_gte": 0.99, "paper_slippage_ratio_lte": 1.5,
  "paper_turnover_tolerance": 0.20, "paper_model_rejects_max": 0,
  "capital_ramp": [0.10, 0.25, 0.50, 1.00]
}'::jsonb, 'platform') ON CONFLICT (profile_id) DO NOTHING;

-- ── campaigns (§10): DEFINE facts are immutable ───────────────────────────────
CREATE TABLE IF NOT EXISTS mlops.campaign (
  campaign_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  slug              TEXT NOT NULL,
  tenant_id         TEXT NOT NULL,
  hypothesis        TEXT NOT NULL,
  objective         JSONB NOT NULL,
  benchmark         JSONB NOT NULL,
  delta_practical   DOUBLE PRECISION NOT NULL CHECK (delta_practical > 0),   -- REQUIRED. no default.
  budget            JSONB NOT NULL,
  exploration_floor DOUBLE PRECISION NOT NULL CHECK (exploration_floor >= 0.05 AND exploration_floor <= 1.0),
  gates_profile     TEXT NOT NULL REFERENCES mlops.gate_profile(profile_id),
  preference_vector JSONB,
  search_space      JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_by        TEXT NOT NULL,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, slug),
  CHECK (jsonb_typeof(objective->'maximize') = 'array' AND jsonb_array_length(objective->'maximize') >= 1),
  CHECK (objective ? 'subject_to'),
  CHECK (budget ? 'max_trials' AND budget ? 'gpu_hours' AND budget ? 'usd' AND budget ? 'wall_clock_hours')
);
DROP TRIGGER IF EXISTS trg_campaign_immutable ON mlops.campaign;
CREATE TRIGGER trg_campaign_immutable BEFORE UPDATE OR DELETE ON mlops.campaign
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE TABLE IF NOT EXISTS mlops.campaign_event (
  event_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  campaign_id UUID NOT NULL REFERENCES mlops.campaign(campaign_id),
  tenant_id   TEXT NOT NULL,
  state       TEXT NOT NULL CHECK (state IN ('define','baseline','diagnose','hypothesize','experiment','compare',
                'gate','prune','reallocate','converged','budget_exhausted','diminishing_returns','halted')),
  detail      JSONB NOT NULL DEFAULT '{}'::jsonb,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
DROP TRIGGER IF EXISTS trg_campaign_event_immutable ON mlops.campaign_event;
CREATE TRIGGER trg_campaign_event_immutable BEFORE UPDATE OR DELETE ON mlops.campaign_event
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();
CREATE INDEX IF NOT EXISTS idx_campaign_event_campaign ON mlops.campaign_event(campaign_id, occurred_at);

-- ── the trial (§4.1): registration facts, immutable ───────────────────────────
CREATE TABLE IF NOT EXISTS mlops.trial (
  trial_id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id           TEXT NOT NULL,
  campaign_id         UUID REFERENCES mlops.campaign(campaign_id),
  experiment_id       TEXT,
  parent_trial_id     UUID REFERENCES mlops.trial(trial_id),
  seq                 BIGINT NOT NULL,
  prev_hash           BYTEA  NOT NULL,
  row_hash            BYTEA  NOT NULL,

  config_hash         TEXT   NOT NULL,
  config              JSONB  NOT NULL,
  dataset_id          TEXT   NOT NULL,
  split_spec_id       TEXT,
  code_hash           TEXT   NOT NULL,
  image_digest        TEXT   NOT NULL,
  seed_set            BIGINT[] NOT NULL,

  actor_kind          TEXT   NOT NULL CHECK (actor_kind IN ('human','agent','scheduler')),
  actor_id            TEXT   NOT NULL,
  on_behalf_of        TEXT,
  policy_id           TEXT   NOT NULL,
  policy_version      INT    NOT NULL,
  candidate_set_hash  TEXT,
  propensity          DOUBLE PRECISION,
  exploration_flag    BOOLEAN NOT NULL,
  hypothesis_id       UUID,

  prereg_hash         TEXT   NOT NULL,
  delta_practical     DOUBLE PRECISION,

  non_reproducible    BOOLEAN NOT NULL DEFAULT FALSE,
  overlapping_labels_unweighted BOOLEAN NOT NULL DEFAULT FALSE,
  split_overrides     JSONB  NOT NULL DEFAULT '[]'::jsonb,
  planned_steps       INT,

  registered_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  supersedes          UUID REFERENCES mlops.trial(trial_id),
  knowledge_time      TIMESTAMPTZ NOT NULL DEFAULT now(),

  CONSTRAINT chk_trial_propensity_logged CHECK (propensity IS NOT NULL OR policy_id = 'legacy_unlogged'),
  CONSTRAINT chk_trial_propensity_range  CHECK (propensity IS NULL OR (propensity > 0.0 AND propensity <= 1.0)),
  CONSTRAINT chk_trial_delta_declared    CHECK (delta_practical IS NOT NULL OR policy_id = 'legacy_unlogged'),
  CONSTRAINT chk_trial_campaign_or_legacy CHECK (campaign_id IS NOT NULL OR experiment_id IS NOT NULL OR policy_id = 'legacy_unlogged'),
  CONSTRAINT uq_trial_stream_seq UNIQUE (tenant_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_trial_config_hash ON mlops.trial(tenant_id, config_hash);
CREATE INDEX IF NOT EXISTS idx_trial_campaign    ON mlops.trial(campaign_id);
CREATE INDEX IF NOT EXISTS idx_trial_experiment  ON mlops.trial(experiment_id);

CREATE OR REPLACE FUNCTION mlops.trial_chain()
RETURNS TRIGGER AS $$
DECLARE
  tip_hash BYTEA;
  tip_seq  BIGINT;
BEGIN
  IF NEW.delta_practical IS NULL AND NEW.policy_id <> 'legacy_unlogged' THEN
    RAISE EXCEPTION 'delta_practical is REQUIRED and has no default (SPEC §4.1): declare the practically-meaningful effect size before the trial runs, or use policy_id=''legacy_unlogged'' for a path that never declared one';
  END IF;

  PERFORM pg_advisory_xact_lock(hashtext('mlops.trial:' || NEW.tenant_id));
  SELECT row_hash, seq INTO tip_hash, tip_seq
    FROM mlops.trial WHERE tenant_id = NEW.tenant_id ORDER BY seq DESC LIMIT 1;
  IF tip_hash IS NULL THEN
    tip_hash := '\x0000000000000000000000000000000000000000000000000000000000000000'::BYTEA;
    tip_seq  := -1;
  END IF;

  NEW.seq       := tip_seq + 1;
  NEW.prev_hash := tip_hash;
  NEW.row_hash  := sha256(
       tip_hash
    || convert_to(NEW.trial_id::TEXT, 'UTF8')
    || convert_to(NEW.seq::TEXT, 'UTF8')
    || convert_to(NEW.tenant_id, 'UTF8')
    || convert_to(COALESCE(NEW.campaign_id::TEXT, ''), 'UTF8')
    || convert_to(COALESCE(NEW.experiment_id, ''), 'UTF8')
    || convert_to(NEW.config_hash, 'UTF8')
    || convert_to(NEW.dataset_id, 'UTF8')
    || convert_to(NEW.code_hash, 'UTF8')
    || convert_to(NEW.image_digest, 'UTF8')
    || convert_to(array_to_string(NEW.seed_set, ','), 'UTF8')
    || convert_to(NEW.prereg_hash, 'UTF8')
    || COALESCE(float8send(NEW.delta_practical), '\x'::BYTEA)
    || convert_to(NEW.actor_kind, 'UTF8')
    || convert_to(NEW.actor_id, 'UTF8')
    || convert_to(COALESCE(NEW.on_behalf_of, ''), 'UTF8')
    || convert_to(NEW.policy_id, 'UTF8')
    || convert_to(NEW.policy_version::TEXT, 'UTF8')
    || convert_to(COALESCE(NEW.candidate_set_hash, ''), 'UTF8')
    || COALESCE(float8send(NEW.propensity), '\x'::BYTEA)
    || convert_to(NEW.exploration_flag::TEXT, 'UTF8')
    || convert_to(COALESCE(NEW.supersedes::TEXT, ''), 'UTF8')
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_trial_chain ON mlops.trial;
CREATE TRIGGER trg_trial_chain BEFORE INSERT ON mlops.trial FOR EACH ROW EXECUTE FUNCTION mlops.trial_chain();
DROP TRIGGER IF EXISTS trg_trial_immutable ON mlops.trial;
CREATE TRIGGER trg_trial_immutable BEFORE UPDATE OR DELETE ON mlops.trial FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── trial lifecycle (§9): append-only events validated by the state machine ───
CREATE TABLE IF NOT EXISTS mlops.trial_event (
  event_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  trial_id         UUID NOT NULL REFERENCES mlops.trial(trial_id),
  tenant_id        TEXT NOT NULL,
  event_seq        INT  NOT NULL,
  state            TEXT NOT NULL CHECK (state IN ('queued','rejected','deduplicated','provision','running','evaluate',
                     'paused','preempted','recovering','gated','completed','completed_pass','completed_fail','failed')),
  censoring        TEXT NOT NULL DEFAULT 'none' CHECK (censoring IN
                     ('none','right_asha','right_budget','right_preempt','right_cancel','failed')),
  terminal_reason  TEXT CHECK (terminal_reason IS NULL OR terminal_reason IN
                     ('oom','nan_divergence','data_error','timeout','leakage_detected','budget_exceeded','cancelled',
                      'dependency_failure','asha_stopped','integrity_rejected','preempted_abandoned','gate_failed')),
  censor_at_step   INT,
  run_id           TEXT,
  deduplicated_of  UUID REFERENCES mlops.trial(trial_id),
  outcome          mlops.outcome_vector,
  outcome_digest   TEXT,
  gate_profile     TEXT REFERENCES mlops.gate_profile(profile_id),
  gate_results     JSONB,
  gpu_seconds      DOUBLE PRECISION,
  cpu_seconds      DOUBLE PRECISION,
  peak_vram_bytes  BIGINT,
  usd_cost         NUMERIC(18,6),
  artifacts_uri    TEXT,
  metrics_uri      TEXT,
  predictions_uri  TEXT,
  returns_uri      TEXT,
  detail           JSONB NOT NULL DEFAULT '{}'::jsonb,
  occurred_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  prev_hash        BYTEA NOT NULL,
  row_hash         BYTEA NOT NULL,
  CONSTRAINT uq_trial_event_seq UNIQUE (trial_id, event_seq),
  CONSTRAINT chk_event_outcome_digest CHECK ((outcome IS NULL) = (outcome_digest IS NULL)),
  CONSTRAINT chk_event_dedup_target CHECK (state <> 'deduplicated' OR deduplicated_of IS NOT NULL),
  -- INV-18: a completion that reports a Sharpe produced a return series, and the
  -- series must be on record for N_eff and every later re-scoring.
  CONSTRAINT chk_event_returns_persisted CHECK (
    state NOT IN ('completed','completed_pass','completed_fail')
    OR outcome IS NULL OR (outcome).sharpe_net IS NULL OR returns_uri IS NOT NULL),
  CONSTRAINT chk_event_terminal_censoring CHECK (
    CASE state
      WHEN 'failed' THEN censoring <> 'none' AND terminal_reason IS NOT NULL
      WHEN 'completed' THEN censoring = 'none'
      WHEN 'completed_pass' THEN censoring = 'none'
      WHEN 'completed_fail' THEN censoring = 'none'
      ELSE TRUE
    END),
  CONSTRAINT chk_event_reason_censoring CHECK (
    terminal_reason IS NULL
    OR (terminal_reason = 'asha_stopped' AND censoring = 'right_asha')
    OR (terminal_reason = 'budget_exceeded' AND censoring = 'right_budget')
    OR (terminal_reason = 'cancelled' AND censoring = 'right_cancel')
    OR (terminal_reason = 'preempted_abandoned' AND censoring = 'right_preempt')
    OR (terminal_reason = 'gate_failed' AND censoring = 'none')
    OR (terminal_reason = 'integrity_rejected' AND censoring IN ('none','failed'))
    OR (terminal_reason IN ('oom','nan_divergence','data_error','timeout','leakage_detected','dependency_failure') AND censoring = 'failed'))
);
CREATE INDEX IF NOT EXISTS idx_trial_event_trial ON mlops.trial_event(trial_id, event_seq DESC);

CREATE OR REPLACE FUNCTION mlops.trial_transition_legal(from_state TEXT, to_state TEXT)
RETURNS BOOLEAN AS $$
  SELECT CASE from_state
    WHEN 'registered' THEN to_state IN ('queued','rejected','deduplicated','running','failed')
    WHEN 'queued'     THEN to_state IN ('provision','running','failed')
    WHEN 'provision'  THEN to_state IN ('running','failed')
    WHEN 'running'    THEN to_state IN ('evaluate','paused','preempted','failed','completed')
    WHEN 'paused'     THEN to_state IN ('queued','failed')
    WHEN 'preempted'  THEN to_state IN ('recovering','failed')
    WHEN 'recovering' THEN to_state IN ('running','failed')
    WHEN 'evaluate'   THEN to_state IN ('gated','completed','completed_fail','failed')
    WHEN 'gated'      THEN to_state IN ('completed_pass','completed_fail')
    ELSE FALSE
  END;
$$ LANGUAGE sql IMMUTABLE;

CREATE OR REPLACE FUNCTION mlops.trial_event_chain()
RETURNS TRIGGER AS $$
DECLARE
  cur_state TEXT;
  tip_hash  BYTEA;
  tip_seq   INT;
  trial_tenant TEXT;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('mlops.trial_event:' || NEW.trial_id::TEXT));
  SELECT tenant_id, row_hash INTO trial_tenant, tip_hash FROM mlops.trial WHERE trial_id = NEW.trial_id;
  IF trial_tenant IS NULL THEN
    RAISE EXCEPTION 'trial % does not exist', NEW.trial_id;
  END IF;
  IF trial_tenant <> NEW.tenant_id THEN
    RAISE EXCEPTION 'trial_event tenant % does not match trial tenant %', NEW.tenant_id, trial_tenant;
  END IF;

  SELECT state, row_hash, event_seq INTO cur_state, tip_hash, tip_seq
    FROM mlops.trial_event WHERE trial_id = NEW.trial_id ORDER BY event_seq DESC LIMIT 1;
  IF cur_state IS NULL THEN
    cur_state := 'registered';
    tip_seq := -1;
    SELECT row_hash INTO tip_hash FROM mlops.trial WHERE trial_id = NEW.trial_id;
  END IF;

  IF NOT mlops.trial_transition_legal(cur_state, NEW.state) THEN
    RAISE EXCEPTION 'illegal trial transition % -> % (trial %)', cur_state, NEW.state, NEW.trial_id;
  END IF;

  NEW.event_seq := tip_seq + 1;
  NEW.prev_hash := tip_hash;
  NEW.row_hash  := sha256(
       tip_hash
    || convert_to(NEW.event_id::TEXT, 'UTF8')
    || convert_to(NEW.trial_id::TEXT, 'UTF8')
    || convert_to(NEW.event_seq::TEXT, 'UTF8')
    || convert_to(NEW.state, 'UTF8')
    || convert_to(NEW.censoring, 'UTF8')
    || convert_to(COALESCE(NEW.terminal_reason, ''), 'UTF8')
    || convert_to(COALESCE(NEW.censor_at_step::TEXT, ''), 'UTF8')
    || convert_to(COALESCE(NEW.run_id, ''), 'UTF8')
    || convert_to(COALESCE(NEW.deduplicated_of::TEXT, ''), 'UTF8')
    || convert_to(COALESCE(NEW.outcome_digest, ''), 'UTF8')
    || convert_to(COALESCE(NEW.returns_uri, ''), 'UTF8')
    || convert_to(COALESCE(NEW.predictions_uri, ''), 'UTF8')
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_trial_event_chain ON mlops.trial_event;
CREATE TRIGGER trg_trial_event_chain BEFORE INSERT ON mlops.trial_event FOR EACH ROW EXECUTE FUNCTION mlops.trial_event_chain();
DROP TRIGGER IF EXISTS trg_trial_event_immutable ON mlops.trial_event;
CREATE TRIGGER trg_trial_event_immutable BEFORE UPDATE OR DELETE ON mlops.trial_event FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE OR REPLACE VIEW mlops.trial_state AS
SELECT t.trial_id, t.tenant_id, t.campaign_id, t.experiment_id, t.config_hash, t.policy_id,
       t.propensity, t.exploration_flag, t.registered_at,
       COALESCE(e.state, 'registered') AS state,
       COALESCE(e.censoring, 'none')   AS censoring,
       e.terminal_reason, e.run_id, e.deduplicated_of, e.outcome, e.gate_profile, e.gate_results,
       e.returns_uri, e.predictions_uri, e.occurred_at AS state_since,
       COALESCE(e.state IN ('rejected','deduplicated','completed','completed_pass','completed_fail','failed'), FALSE) AS terminal
FROM mlops.trial t
LEFT JOIN LATERAL (
  SELECT * FROM mlops.trial_event ev WHERE ev.trial_id = t.trial_id ORDER BY ev.event_seq DESC LIMIT 1
) e ON TRUE;

-- ── decision log (§4.4, INV-20) ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mlops.decision (
  decision_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id        TEXT NOT NULL,
  campaign_id      UUID REFERENCES mlops.campaign(campaign_id),
  experiment_id    TEXT,
  decision_kind    TEXT NOT NULL CHECK (decision_kind IN ('propose','select','prune','reallocate','stop','promote','reject')),
  actor_kind       TEXT NOT NULL CHECK (actor_kind IN ('human','agent','scheduler')),
  actor_id         TEXT NOT NULL,
  on_behalf_of     TEXT,
  context_hash     TEXT NOT NULL,
  context_uri      TEXT,
  candidate_set    JSONB NOT NULL CHECK (jsonb_typeof(candidate_set) = 'array'),
  chosen           JSONB NOT NULL,
  propensity       DOUBLE PRECISION CHECK (propensity IS NULL OR (propensity > 0.0 AND propensity <= 1.0)),
  policy_id        TEXT NOT NULL,
  policy_version   INT  NOT NULL,
  exploration_flag BOOLEAN NOT NULL,
  decision_tier    TEXT NOT NULL CHECK (decision_tier IN ('rule','shrunk','learned')),
  rationale        TEXT,
  decided_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  seq              BIGINT NOT NULL,
  prev_hash        BYTEA NOT NULL,
  row_hash         BYTEA NOT NULL,
  CONSTRAINT chk_decision_propensity CHECK (propensity IS NOT NULL OR policy_id = 'legacy_unlogged'),
  CONSTRAINT chk_decision_scope CHECK (campaign_id IS NOT NULL OR experiment_id IS NOT NULL),
  CONSTRAINT uq_decision_stream_seq UNIQUE (tenant_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_decision_campaign ON mlops.decision(campaign_id);

CREATE OR REPLACE FUNCTION mlops.decision_chain()
RETURNS TRIGGER AS $$
DECLARE tip_hash BYTEA; tip_seq BIGINT;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('mlops.decision:' || NEW.tenant_id));
  SELECT row_hash, seq INTO tip_hash, tip_seq FROM mlops.decision WHERE tenant_id = NEW.tenant_id ORDER BY seq DESC LIMIT 1;
  IF tip_hash IS NULL THEN
    tip_hash := '\x0000000000000000000000000000000000000000000000000000000000000000'::BYTEA; tip_seq := -1;
  END IF;
  NEW.seq := tip_seq + 1;
  NEW.prev_hash := tip_hash;
  NEW.row_hash := sha256(
       tip_hash
    || convert_to(NEW.decision_id::TEXT, 'UTF8')
    || convert_to(NEW.seq::TEXT, 'UTF8')
    || convert_to(NEW.tenant_id, 'UTF8')
    || convert_to(NEW.decision_kind, 'UTF8')
    || convert_to(NEW.actor_kind, 'UTF8')
    || convert_to(NEW.actor_id, 'UTF8')
    || convert_to(NEW.context_hash, 'UTF8')
    || sha256(convert_to(NEW.candidate_set::TEXT, 'UTF8'))
    || sha256(convert_to(NEW.chosen::TEXT, 'UTF8'))
    || convert_to(NEW.policy_id, 'UTF8')
    || convert_to(NEW.policy_version::TEXT, 'UTF8')
    || COALESCE(float8send(NEW.propensity), '\x'::BYTEA)
    || convert_to(NEW.exploration_flag::TEXT, 'UTF8')
    || convert_to(NEW.decision_tier, 'UTF8')
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_decision_chain ON mlops.decision;
CREATE TRIGGER trg_decision_chain BEFORE INSERT ON mlops.decision FOR EACH ROW EXECUTE FUNCTION mlops.decision_chain();
DROP TRIGGER IF EXISTS trg_decision_immutable ON mlops.decision;
CREATE TRIGGER trg_decision_immutable BEFORE UPDATE OR DELETE ON mlops.decision FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── OOS return series (§4.2·3, INV-18): the input to platform N_eff (§12.4) ────
CREATE TABLE IF NOT EXISTS mlops.trial_return_series (
  trial_id      UUID PRIMARY KEY REFERENCES mlops.trial(trial_id),
  tenant_id     TEXT NOT NULL,
  ts            TIMESTAMPTZ[] NOT NULL,
  returns       DOUBLE PRECISION[] NOT NULL,
  series_digest TEXT NOT NULL,
  recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT chk_return_series_shape CHECK (cardinality(ts) = cardinality(returns))
);
CREATE INDEX IF NOT EXISTS idx_return_series_tenant ON mlops.trial_return_series(tenant_id);
DROP TRIGGER IF EXISTS trg_trial_return_series_immutable ON mlops.trial_return_series;
CREATE TRIGGER trg_trial_return_series_immutable BEFORE UPDATE OR DELETE ON mlops.trial_return_series
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── daily signed anchors (§4.6) ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mlops.ledger_anchor (
  anchor_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id     TEXT NOT NULL,
  anchor_date   DATE NOT NULL,
  trial_max_seq BIGINT NOT NULL,
  trial_head    BYTEA NOT NULL,
  decision_max_seq BIGINT NOT NULL,
  decision_head BYTEA NOT NULL,
  signature     BYTEA NOT NULL,
  key_id        TEXT NOT NULL,
  worm_uri      TEXT NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT uq_anchor_tenant_day UNIQUE (tenant_id, anchor_date)
);
DROP TRIGGER IF EXISTS trg_ledger_anchor_immutable ON mlops.ledger_anchor;
CREATE TRIGGER trg_ledger_anchor_immutable BEFORE UPDATE OR DELETE ON mlops.ledger_anchor FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── sealed holdout (§12.7): one evaluation per lineage, ever ──────────────────
CREATE TABLE IF NOT EXISTS mlops.sealed_holdout_call (
  tenant_id           TEXT NOT NULL,
  strategy_lineage_id TEXT NOT NULL,
  trial_id            UUID NOT NULL REFERENCES mlops.trial(trial_id),
  called_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  result              JSONB NOT NULL,
  PRIMARY KEY (tenant_id, strategy_lineage_id)
);
DROP TRIGGER IF EXISTS trg_sealed_holdout_call_immutable ON mlops.sealed_holdout_call;
CREATE TRIGGER trg_sealed_holdout_call_immutable BEFORE UPDATE OR DELETE ON mlops.sealed_holdout_call FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE TABLE IF NOT EXISTS mlops.sealed_holdout_attempt (
  attempt_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id           TEXT NOT NULL,
  strategy_lineage_id TEXT NOT NULL,
  requested_by        TEXT NOT NULL,
  served_first_result BOOLEAN NOT NULL,
  attempted_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
DROP TRIGGER IF EXISTS trg_sealed_holdout_attempt_immutable ON mlops.sealed_holdout_attempt;
CREATE TRIGGER trg_sealed_holdout_attempt_immutable BEFORE UPDATE OR DELETE ON mlops.sealed_holdout_attempt FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();
-- The first attempt claims the lineage before any holdout data is read. A second
-- concurrent "first" attempt fails on this index and never reaches the data; a
-- claimed lineage whose evaluation crashed stays spent (one evaluation, ever).
CREATE UNIQUE INDEX IF NOT EXISTS uq_holdout_first_attempt
  ON mlops.sealed_holdout_attempt (tenant_id, strategy_lineage_id) WHERE NOT served_first_result;

-- ── agent trajectories (§14.6): log now, impossible to retrofit ────────────────
CREATE TABLE IF NOT EXISTS mlops.agent_trajectory (
  traj_id            UUID NOT NULL,
  step_idx           INT  NOT NULL,
  tenant_id          TEXT NOT NULL,
  campaign_id        UUID,
  tool_name          TEXT NOT NULL,
  tool_schema_hash   TEXT NOT NULL,
  tool_semver        TEXT NOT NULL,
  arguments          JSONB,
  result_summary     JSONB,
  error              JSONB,
  latency_ms         INT,
  tokens_in          INT,
  tokens_out         INT,
  critic_label       TEXT CHECK (critic_label IN ('good','unnecessary','mistake','recover')),
  propensity         DOUBLE PRECISION,
  exploration_flag   BOOLEAN,
  outcome_trial_id   UUID,
  label_available_at TIMESTAMPTZ,
  recorded_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (traj_id, step_idx)
);
DROP TRIGGER IF EXISTS trg_agent_trajectory_immutable ON mlops.agent_trajectory;
CREATE TRIGGER trg_agent_trajectory_immutable BEFORE UPDATE OR DELETE ON mlops.agent_trajectory FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();
-- Critic labels arrive later; they append here rather than mutating the step.
CREATE TABLE IF NOT EXISTS mlops.trajectory_label (
  traj_id      UUID NOT NULL,
  step_idx     INT  NOT NULL,
  tenant_id    TEXT NOT NULL,
  critic_label TEXT NOT NULL CHECK (critic_label IN ('good','unnecessary','mistake','recover')),
  labeled_by   TEXT NOT NULL,
  labeled_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  FOREIGN KEY (traj_id, step_idx) REFERENCES mlops.agent_trajectory(traj_id, step_idx)
);

-- ── audit trail for approval envelopes (§15): record_phase pre proves denials ──
CREATE TABLE IF NOT EXISTS mlops.audit_event (
  audit_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id    TEXT NOT NULL,
  seq          BIGINT NOT NULL,
  record_phase TEXT NOT NULL CHECK (record_phase IN ('pre','post')),
  action       TEXT NOT NULL,
  envelope     TEXT NOT NULL CHECK (envelope IN ('none','spend','promotion','sealed_holdout','tier3_memory')),
  actor_kind   TEXT NOT NULL,
  actor_id     TEXT NOT NULL,
  on_behalf_of TEXT,
  request      JSONB NOT NULL,
  verdict      TEXT NOT NULL CHECK (verdict IN ('allowed','denied','pending_approval','approved','executed','failed')),
  approval_id  UUID,
  pre_audit_id UUID REFERENCES mlops.audit_event(audit_id),
  occurred_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  prev_hash    BYTEA NOT NULL,
  row_hash     BYTEA NOT NULL,
  CONSTRAINT uq_audit_seq UNIQUE (tenant_id, seq),
  CONSTRAINT chk_audit_post_has_pre CHECK (record_phase = 'pre' OR pre_audit_id IS NOT NULL)
);
CREATE OR REPLACE FUNCTION mlops.audit_chain()
RETURNS TRIGGER AS $$
DECLARE tip_hash BYTEA; tip_seq BIGINT;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('mlops.audit:' || NEW.tenant_id));
  SELECT row_hash, seq INTO tip_hash, tip_seq FROM mlops.audit_event WHERE tenant_id = NEW.tenant_id ORDER BY seq DESC LIMIT 1;
  IF tip_hash IS NULL THEN
    tip_hash := '\x0000000000000000000000000000000000000000000000000000000000000000'::BYTEA; tip_seq := -1;
  END IF;
  NEW.seq := tip_seq + 1;
  NEW.prev_hash := tip_hash;
  NEW.row_hash := sha256(tip_hash
    || convert_to(NEW.audit_id::TEXT, 'UTF8') || convert_to(NEW.seq::TEXT, 'UTF8')
    || convert_to(NEW.record_phase, 'UTF8') || convert_to(NEW.action, 'UTF8')
    || convert_to(NEW.envelope, 'UTF8') || convert_to(NEW.actor_id, 'UTF8')
    || sha256(convert_to(NEW.request::TEXT, 'UTF8')) || convert_to(NEW.verdict, 'UTF8')
    || convert_to(COALESCE(NEW.pre_audit_id::TEXT, ''), 'UTF8'));
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_audit_chain ON mlops.audit_event;
CREATE TRIGGER trg_audit_chain BEFORE INSERT ON mlops.audit_event FOR EACH ROW EXECUTE FUNCTION mlops.audit_chain();
DROP TRIGGER IF EXISTS trg_audit_immutable ON mlops.audit_event;
CREATE TRIGGER trg_audit_immutable BEFORE UPDATE OR DELETE ON mlops.audit_event FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── internal model registry + the feature firewall (§7.3–7.4, §14) ────────────
CREATE TABLE IF NOT EXISTS mlops.internal_model_registry (
  model_id                 TEXT PRIMARY KEY,
  tenant_id                TEXT,
  scope                    TEXT NOT NULL CHECK (scope IN ('global','hierarchical','per_tenant')),
  tier                     CHAR(1) NOT NULL CHECK (tier IN ('A','B','C')),
  feature_list             TEXT[] NOT NULL,
  frozen_holdout_trial_ids UUID[] NOT NULL,
  entropy_floor            DOUBLE PRECISION NOT NULL,
  registered_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT m8_per_tenant CHECK (split_part(model_id, ':', 1) <> 'M8' OR scope = 'per_tenant'),
  CONSTRAINT per_tenant_has_tenant CHECK (scope <> 'per_tenant' OR tenant_id IS NOT NULL),
  CONSTRAINT no_tier_c_models CHECK (tier <> 'C')
);
-- Promotions are append-only; the champion is the latest promotion.
CREATE TABLE IF NOT EXISTS mlops.model_promotion (
  promotion_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  model_id       TEXT NOT NULL REFERENCES mlops.internal_model_registry(model_id),
  version        TEXT NOT NULL,
  action         TEXT NOT NULL CHECK (action IN ('promote','rollback','block')),
  tier           CHAR(1) NOT NULL CHECK (tier IN ('A','B')),
  promoted_by    TEXT,
  evidence       JSONB NOT NULL,
  policy_entropy DOUBLE PRECISION,
  decided_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT tier_b_requires_approver CHECK (tier = 'A' OR action <> 'promote' OR promoted_by IS NOT NULL)
);
DROP TRIGGER IF EXISTS trg_model_promotion_immutable ON mlops.model_promotion;
CREATE TRIGGER trg_model_promotion_immutable BEFORE UPDATE OR DELETE ON mlops.model_promotion FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- Global freeze switch (§14.3): append-only; the latest row wins.
CREATE TABLE IF NOT EXISTS mlops.internal_model_freeze (
  freeze_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  frozen     BOOLEAN NOT NULL,
  reason     TEXT NOT NULL,
  set_by     TEXT NOT NULL,
  set_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
DROP TRIGGER IF EXISTS trg_model_freeze_immutable ON mlops.internal_model_freeze;
CREATE TRIGGER trg_model_freeze_immutable BEFORE UPDATE OR DELETE ON mlops.internal_model_freeze FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();
