-- 0071 — the per-tier scorecard (AGENT-009 §4, P4; Set S task S-4.11).
--
-- One row per (harness version, profile). **Per profile is the whole design.**
-- An average across tiers is how a local tier quietly dies: a frontier gain of
-- six points and a 7B loss of four averages to an improvement, ships, and nobody
-- finds out until the local tier is unusable.
--
-- ── Why `hardware` is a column and not a note ──────────────────────────────
--
-- `evals/gates.yaml` records the failure it prevents, in both directions. A
-- local_mid run compared against a frontier baseline fails every margin, and the
-- obvious fix — widening the margins — would weaken the gate for frontier too.
-- The other direction passes *silently*: a local_mid run compared against a
-- local_mid baseline earned on DIFFERENT hardware measures the box and reports it
-- as a harness change. A faster box is not a better harness.

CREATE TABLE IF NOT EXISTS agent_scorecard (
  scorecard_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  harness_version  TEXT NOT NULL CHECK (length(harness_version) > 0),
  profile_id       TEXT NOT NULL CHECK (length(profile_id) > 0),
  tier             TEXT NOT NULL CHECK (length(tier) > 0),
  hardware         TEXT NOT NULL DEFAULT '',
  -- baseline | candidate | current. `current` is the promoted version; there is
  -- at most one per profile and the partial unique index below says so.
  role             TEXT NOT NULL DEFAULT 'candidate'
                     CHECK (role IN ('baseline','candidate','current')),

  cost_usd         DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
  cost_tokens      BIGINT NOT NULL DEFAULT 0 CHECK (cost_tokens >= 0),
  median_steps     DOUBLE PRECISION NOT NULL DEFAULT 0,

  -- NULL means `unmeasured`, and that is a different statement from 0.
  -- AGENT-009 P3: every profile claim has an eval that licenses it, and a
  -- profile with none says so rather than carrying a number nobody earned.
  cache_hit_rate   DOUBLE PRECISION CHECK (cache_hit_rate IS NULL
                                            OR cache_hit_rate BETWEEN 0 AND 1),
  memory_recall    DOUBLE PRECISION CHECK (memory_recall IS NULL
                                            OR memory_recall BETWEEN 0 AND 1),
  hitl_precision   DOUBLE PRECISION CHECK (hitl_precision IS NULL
                                            OR hitl_precision BETWEEN 0 AND 1),

  -- How many times the gate has been evaluated for this harness version. The
  -- platform's own multiple-testing exposure, recorded rather than ignored — the
  -- discipline the trial ledger applies to research, applied to the harness that
  -- judges it.
  evaluations      INT NOT NULL DEFAULT 0 CHECK (evaluations >= 0),

  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (harness_version, profile_id, role)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_scorecard_one_current_per_profile
  ON agent_scorecard (profile_id) WHERE role = 'current';
CREATE INDEX IF NOT EXISTS idx_scorecard_profile
  ON agent_scorecard (profile_id, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_scorecard_suite (
  scorecard_id     UUID NOT NULL REFERENCES agent_scorecard(scorecard_id) ON DELETE CASCADE,
  suite            TEXT NOT NULL,
  -- replay: exact, no sampling error. live: sampled, needs an interval.
  kind             TEXT NOT NULL CHECK (kind IN ('replay','live')),
  -- §4.2: the sealed 20%. Never populated by a PR run.
  sealed           BOOLEAN NOT NULL DEFAULT FALSE,

  passed           INT NOT NULL CHECK (passed >= 0),
  n                INT NOT NULL CHECK (n >= 0),
  CONSTRAINT passed_cannot_exceed_n CHECK (passed <= n),

  -- §4.1. The spread across the k = 3 repeats. A suite above the ceiling is
  -- excluded from gating: a gate that fires at random spends the team's trust,
  -- and the response to a random gate is to stop believing all of them.
  flakiness        DOUBLE PRECISION NOT NULL DEFAULT 0
                     CHECK (flakiness BETWEEN 0 AND 1),
  -- A replay suite whose repeats disagree is the harness having become
  -- non-deterministic, which is itself the finding — so it is refused outright
  -- rather than merely excluded.
  CONSTRAINT a_replay_suite_is_deterministic CHECK (
    kind <> 'replay' OR flakiness = 0
  ),

  -- P2. Failures by rung: routing / exposure / decode / validation / tool /
  -- memory / model_choice. "The agent failed" is not a finding.
  attribution      JSONB NOT NULL DEFAULT '{}'::jsonb,
  -- Which cases failed. A pass rate without these is a number nobody can act on,
  -- and the replay gate compares these lists rather than the rates.
  failed_cases     TEXT[] NOT NULL DEFAULT '{}'::text[],

  PRIMARY KEY (scorecard_id, suite, sealed)
);

GRANT SELECT, INSERT, UPDATE ON agent_scorecard, agent_scorecard_suite TO app_role;
GRANT SELECT ON agent_scorecard, agent_scorecard_suite TO agent_role, internal_ml_role;
