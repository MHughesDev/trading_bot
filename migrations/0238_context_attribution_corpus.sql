-- 0238 -- the attribution corpus (S-9.10, S-9.12).
--
-- `harness::context::utility` attributes a run's rendered records against what
-- the model did next, and `harness::context::fit` fits the selection weights to
-- the result. Both were complete and had no store: the attribution lived and
-- died in memory, so `Scorecard::utility_rate` had nothing to report and a fit
-- had no corpus to read. This is the corpus.
--
-- Two tables, append-only, written once per run when it ends:
--
--   * `context_attribution_run`    one row per run: its profile, tier and policy
--                                  version, and the token sums the rate is made of.
--   * `context_attribution_record` one row per rendered record: the nine selection
--                                  signals it was scored on and what became of it.
--
-- Sums, not rates, are stored. A rate cannot be re-aggregated across runs without
-- the denominators, and `utility_by_tier` sums tokens so that the answer does not
-- depend on corpus order -- the aggregation stays reproducible from the rows.
--
-- No content is stored: signals are numbers and `record_kind` is a name. The
-- corpus holds nothing a tenant wrote, so it carries no user_id and no RLS; it is
-- platform evaluation data, like `eval_holdout_draw`.
--
-- No foreign key to `agent_runs`: a run swept by retention must not delete the
-- evidence a policy was fitted on (the same rule 0124 states for retrievals).

CREATE TABLE IF NOT EXISTS context_attribution_run (
  run_id             UUID PRIMARY KEY,
  profile_id         TEXT NOT NULL CHECK (btrim(profile_id) <> ''),
  tier               TEXT NOT NULL CHECK (btrim(tier) <> ''),
  policy_version     TEXT NOT NULL CHECK (btrim(policy_version) <> ''),
  rendered_tokens    BIGINT NOT NULL CHECK (rendered_tokens >= 0),
  used_tokens        BIGINT NOT NULL CHECK (used_tokens >= 0 AND used_tokens <= rendered_tokens),
  indeterminate_share REAL NOT NULL DEFAULT 0
                     CHECK (indeterminate_share >= 0 AND indeterminate_share <= 1),
  recorded_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS context_attribution_run_policy_idx
  ON context_attribution_run (policy_version, profile_id);

CREATE TABLE IF NOT EXISTS context_attribution_record (
  id          BIGSERIAL PRIMARY KEY,
  run_id      UUID NOT NULL REFERENCES context_attribution_run (run_id),
  record_kind TEXT NOT NULL CHECK (btrim(record_kind) <> ''),
  signals     JSONB NOT NULL CHECK (jsonb_typeof(signals) = 'object'),
  utility     TEXT NOT NULL CHECK (utility IN ('used','wanted','unused','indeterminate'))
);

CREATE INDEX IF NOT EXISTS context_attribution_record_run_idx
  ON context_attribution_record (run_id);

-- Append-only: a corpus a fit was drawn from must not change underneath the
-- fingerprint recorded with that fit (`context_policy_promotion`). Reuses 0177's
-- immutability function.
DROP TRIGGER IF EXISTS trg_context_attribution_run_immutable ON context_attribution_run;
CREATE TRIGGER trg_context_attribution_run_immutable
  BEFORE UPDATE OR DELETE ON context_attribution_run
  FOR EACH ROW EXECUTE FUNCTION eval_holdout_draw_immutable();

DROP TRIGGER IF EXISTS trg_context_attribution_record_immutable ON context_attribution_record;
CREATE TRIGGER trg_context_attribution_record_immutable
  BEFORE UPDATE OR DELETE ON context_attribution_record
  FOR EACH ROW EXECUTE FUNCTION eval_holdout_draw_immutable();

GRANT SELECT, INSERT ON context_attribution_run TO app_role;
GRANT SELECT, INSERT ON context_attribution_record TO app_role;
GRANT USAGE, SELECT ON SEQUENCE context_attribution_record_id_seq TO app_role;
