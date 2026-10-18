-- 0070 — the token estimator, calibrated (AGENT-009 §5; AGENT-005 §7;
-- Set S task S-4.12).
--
-- `context.rs` estimates tokens at a flat 4.0 bytes each. The constant is not
-- wrong so much as unmeasured: it was chosen because it over-estimates dense
-- English and JSON, and **an over-estimate is the only kind of estimate that
-- enforces a budget.** A budget enforced with an optimistic number is not
-- enforced — the prompt is built, the provider left-truncates it, and what it
-- drops is the head, which is the charter and the tool schemas. It reads as the
-- model having become stupid.
--
-- What this table adds is the measurement: the median ratio of the provider's
-- own `prompt_tokens` to the harness estimate, over the last 50 runs per model.
--
-- ── Why it is clamped ──────────────────────────────────────────────────────
--
-- To [2.5, 5.0], with 4.0 for an unseen model. A calibration is a fitted
-- constant, and a fitted constant from a handful of unusual runs — six runs of
-- one enormous CSV, say — would push the ratio somewhere that silently disables
-- the budget for every later run. The clamp is what stops a calibration job from
-- being a way to turn the budget off.
--
-- The `budget` suite reports the over-estimate rate and it must stay >= 95%
-- (AGENT-009 §8.5). A calibration that made the estimate *accurate* would fail
-- that, correctly: accurate is the wrong target here, conservative is.

CREATE TABLE IF NOT EXISTS agent_token_calibration (
  model             TEXT PRIMARY KEY,
  provider          TEXT NOT NULL DEFAULT '',
  bytes_per_token   DOUBLE PRECISION NOT NULL
                      CHECK (bytes_per_token BETWEEN 2.5 AND 5.0),
  -- How many runs the median was taken over. Below the floor the row exists but
  -- is not used: §8.5 asks for >= 10 runs, and a median of three is a guess with
  -- a decimal point.
  sample_n          INT NOT NULL DEFAULT 0 CHECK (sample_n >= 0),
  -- The share of calls where the harness estimate was >= the provider's count.
  -- This is the number that licenses the constant, not the constant itself.
  over_estimate_rate DOUBLE PRECISION
                      CHECK (over_estimate_rate IS NULL
                             OR over_estimate_rate BETWEEN 0 AND 1),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The floor at which a row may be used, stated once here rather than in every
-- caller. AGENT-009 §8.5.
CREATE OR REPLACE FUNCTION agent_calibrated_bytes_per_token(p_model TEXT)
RETURNS DOUBLE PRECISION AS $$
  SELECT COALESCE(
    (SELECT bytes_per_token FROM agent_token_calibration
      WHERE model = p_model AND sample_n >= 10),
    4.0  -- the prior for an unseen or under-sampled model
  );
$$ LANGUAGE sql STABLE;

GRANT SELECT, INSERT, UPDATE ON agent_token_calibration TO app_role;
GRANT SELECT ON agent_token_calibration TO agent_role;
GRANT EXECUTE ON FUNCTION agent_calibrated_bytes_per_token(TEXT) TO app_role, agent_role;

-- The per-call observations the median is taken over. Kept rather than folded in
-- on write, because a median needs the distribution and because "the ratio moved"
-- is a question about which runs moved it.
CREATE TABLE IF NOT EXISTS agent_token_observation (
  observation_id  BIGSERIAL PRIMARY KEY,
  run_id          UUID REFERENCES agent_runs(run_id) ON DELETE CASCADE,
  model           TEXT NOT NULL,
  step            INT NOT NULL CHECK (step >= 0),
  estimated       INT NOT NULL CHECK (estimated >= 0),
  -- The provider's own count. NULL when the provider did not report one — which
  -- is itself worth recording: a provider that reports nothing cannot calibrate
  -- anything, and the row says so rather than being absent.
  reported        INT CHECK (reported IS NULL OR reported >= 0),
  observed_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_token_observation_model
  ON agent_token_observation (model, observed_at DESC);

GRANT SELECT, INSERT ON agent_token_observation TO app_role;
GRANT USAGE, SELECT ON SEQUENCE agent_token_observation_observation_id_seq TO app_role;
