-- 0113 — the evidence the COMPARE phase judges, and the verdict it reaches
-- (SPEC §10, §11.5; ADR-P2-04, ADR-P2-12, ADR-P2-31, ADR-P2-34, ADR-P2-35).
--
-- Two tables, and the split between them is the point.
--
-- `replicate_measurement` is what the EXPERIMENT phase's `Study` worker
-- *measured*: one arm, one replicate of the §11.5 nuisance grid, one number,
-- and the counted trial that produced it. Every row names a `trial_id`, so a
-- measurement that entered a comparison without a ledger row is not
-- representable (INV-16 read from the other end: no evidence without a look).
--
-- `comparison` is what the COMPARE phase's `EvalTask` worker *concluded*, and
-- it is the serialized form of the sealed `Comparison` that
-- `ComparisonPlan::judge` returns — never a bare verdict. The profile, N_eff,
-- delta_practical, both trial counts and every qualifying flag travel with the
-- number, because a verdict without them is the thing the sealed type exists to
-- prevent from travelling.
--
-- ## Why `comparison` is unique on (campaign_id, campaign_seq)
--
-- A campaign's state is a fold over `mlops.campaign_event` and each phase's
-- work is a child job keyed by `(campaign job, phase, seq)` (ADR-P2-04). The
-- child job is idempotent at that key; this unique index is what makes the
-- child's *side effect* idempotent at the same key. A COMPARE worker that is
-- re-queued after a lost lease re-judges and writes nothing new, so a campaign
-- resumed mid-phase reaches the same state with no duplicated effect — which is
-- AT-59 stated about a write rather than about a fold.
--
-- Both tables are append-only, like everything else in `mlops`: a measurement
-- that can be edited is an opinion, and a verdict that can be edited is a
-- verdict nobody can audit.

-- ── the evidence ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS mlops.replicate_measurement (
  measurement_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id      TEXT NOT NULL,
  campaign_id    UUID NOT NULL REFERENCES mlops.campaign(campaign_id),
  -- Which side of the comparison this measurement belongs to. `incumbent` is
  -- the campaign's declared benchmark (or whatever last won); a challenger is
  -- named by the candidate it came from.
  arm            TEXT NOT NULL CHECK (length(arm) > 0),
  -- The §11.5 replicate: which window of history, which cost perturbation,
  -- which stochastic-fill seed. The same triple is applied to both arms, and
  -- the primary key below is what enforces that a pair exists rather than two
  -- unpaired samples.
  window_index   INT    NOT NULL CHECK (window_index >= 0),
  cost_bp        INT    NOT NULL,
  seed           BIGINT NOT NULL,
  -- The objective the campaign declared it would be judged on. Stored per row
  -- so a campaign that maximizes two things can hold both without one silently
  -- overwriting the other.
  metric         TEXT NOT NULL CHECK (length(metric) > 0),
  value          DOUBLE PRECISION NOT NULL CHECK (value = value),  -- NaN is not an observation
  -- The counted look that produced it. NOT NULL and a foreign key: a
  -- measurement whose trial does not exist is a number nobody paid for.
  trial_id       UUID NOT NULL REFERENCES mlops.trial(trial_id),
  produced_by    TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  knowledge_time TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- One measurement per (arm, replicate, metric). A re-run of the same study at
  -- the same position writes nothing, which is what makes the EXPERIMENT
  -- phase's side effect idempotent under a re-queue.
  UNIQUE (campaign_id, arm, window_index, cost_bp, seed, metric)
);

CREATE INDEX IF NOT EXISTS idx_replicate_measurement_lookup
  ON mlops.replicate_measurement (tenant_id, campaign_id, metric, arm, window_index, cost_bp, seed);

DROP TRIGGER IF EXISTS trg_replicate_measurement_immutable ON mlops.replicate_measurement;
CREATE TRIGGER trg_replicate_measurement_immutable
  BEFORE UPDATE OR DELETE ON mlops.replicate_measurement
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.replicate_measurement ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.replicate_measurement FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.replicate_measurement;
CREATE POLICY tenant_isolation ON mlops.replicate_measurement
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The platform writes; the agent reads. An agent that could write a measurement
-- could hand the comparison protocol its own replicates, which is §12.7's
-- gate-hacking one level up from the gates.
GRANT SELECT, INSERT ON mlops.replicate_measurement TO app_role;
GRANT SELECT ON mlops.replicate_measurement TO agent_role, internal_ml_role, backtest_role;

-- ── the verdict ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS mlops.comparison (
  comparison_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       TEXT NOT NULL,
  campaign_id     UUID NOT NULL REFERENCES mlops.campaign(campaign_id),
  -- The fold position that dispatched the COMPARE phase. The unique index on
  -- (campaign_id, campaign_seq) is the idempotency of the side effect.
  campaign_seq    BIGINT NOT NULL CHECK (campaign_seq >= 0),
  arm_a           TEXT NOT NULL CHECK (length(arm_a) > 0),
  arm_b           TEXT NOT NULL CHECK (length(arm_b) > 0),
  metric          TEXT NOT NULL CHECK (length(metric) > 0),

  verdict         TEXT NOT NULL CHECK (verdict IN
                    ('promote','reject','keep_sampling','practically_equivalent')),

  -- The confidence sequence, on the difference scale. `cs_upper` is what the
  -- §10 DIMINISHING_RETURNS rule reads: an anytime-valid upper bound below
  -- delta_practical *is* "P(this candidate beats the incumbent by δ) < α".
  cs_centre       DOUBLE PRECISION NOT NULL CHECK (cs_centre = cs_centre),
  cs_lower        DOUBLE PRECISION NOT NULL CHECK (cs_lower = cs_lower),
  cs_upper        DOUBLE PRECISION NOT NULL CHECK (cs_upper = cs_upper),
  alpha           DOUBLE PRECISION NOT NULL CHECK (alpha > 0.0 AND alpha < 1.0),
  CONSTRAINT chk_comparison_interval_ordered CHECK (cs_lower <= cs_upper),

  mean_difference    DOUBLE PRECISION NOT NULL CHECK (mean_difference = mean_difference),
  p_a_better         DOUBLE PRECISION NOT NULL CHECK (p_a_better >= 0.0 AND p_a_better <= 1.0),
  p_a_better_lower   DOUBLE PRECISION NOT NULL CHECK (p_a_better_lower = p_a_better_lower),

  achieved_k      INT NOT NULL CHECK (achieved_k > 0),
  wins            INT NOT NULL CHECK (wins >= 0),
  ties            INT NOT NULL CHECK (ties >= 0),
  losses          INT NOT NULL CHECK (losses >= 0),
  CONSTRAINT chk_comparison_record_totals CHECK (wins + ties + losses = achieved_k),

  -- The context the sealed type refuses to be built without. Every one of these
  -- is NOT NULL for the same reason the struct has no `Option`: a verdict that
  -- arrives without its profile, its effective trial count or its declared
  -- effect size is a bare number, and a bare number is what gets deployed on.
  profile_id      TEXT NOT NULL REFERENCES mlops.gate_profile(profile_id),
  n_eff           DOUBLE PRECISION NOT NULL CHECK (n_eff >= 0.0),
  delta_practical DOUBLE PRECISION NOT NULL CHECK (delta_practical > 0.0),
  trials_a        BIGINT NOT NULL CHECK (trials_a >= 0),
  trials_b        BIGINT NOT NULL CHECK (trials_b >= 0),
  flags           JSONB NOT NULL DEFAULT '{}'::jsonb,
  -- The one-line form the sealed type produces. Stored so a reader of the table
  -- sees the same qualified sentence a reader of the type would.
  headline        TEXT NOT NULL CHECK (length(headline) > 0),

  recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  knowledge_time  TIMESTAMPTZ NOT NULL DEFAULT now(),

  UNIQUE (campaign_id, campaign_seq)
);

CREATE INDEX IF NOT EXISTS idx_comparison_campaign
  ON mlops.comparison (tenant_id, campaign_id, recorded_at);

DROP TRIGGER IF EXISTS trg_comparison_immutable ON mlops.comparison;
CREATE TRIGGER trg_comparison_immutable BEFORE UPDATE OR DELETE ON mlops.comparison
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.comparison ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.comparison FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.comparison;
CREATE POLICY tenant_isolation ON mlops.comparison
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON mlops.comparison TO app_role;
GRANT SELECT ON mlops.comparison TO agent_role, internal_ml_role, backtest_role;
