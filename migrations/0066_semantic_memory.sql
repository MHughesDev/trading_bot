-- 0066 — semantic memory (AGENT-007 §2.2, §3.1, §3.2; Set S tasks S-4.3, S-4.4).
--
-- `knowledge.insight` (0055) is the storage base and stays: it is the pack's
-- insight table, keyed to trial ids, with evidence enforced by a CHECK. What it
-- cannot hold is a claim about a *window* with a *direction* and a *confidence*
-- — and without those three, §2.2.1's contradiction check has nothing to read
-- and P5's `conflicts_with` is a column that is always empty.
--
-- So this is an extension, not a replacement. An entry here cites trials the
-- same way; what it adds is everything a later run needs in order to decide
-- whether to believe it.
--
-- ── The two rules that are structural rather than conventional ──────────────
--
--  1. **Provenance is required.** AGENT-007 P2: an entry without it is a rumour.
--     Enforced here rather than in the writer, because a writer is one code path
--     and a table is every code path.
--
--  2. **`failure_class` is a closed enum, and the agent role cannot write one.**
--     §3.2. The behaviour being stopped is specific: an agent that can choose its
--     own failure class will write `capacity_insufficient` on an idea that died
--     of weak effect, because that class comes with a retry condition and weak
--     effect does not. The idea then stays alive across every later session,
--     spending budget on a condition that will never be met — and the
--     negative-result ledger, the entry §2.4 calls the likeliest to save a
--     budget, quietly becomes the opposite. The GRANT at the bottom is what makes
--     that unreachable rather than discouraged.

CREATE TABLE IF NOT EXISTS knowledge.memory_entry (
  entry_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       TEXT NOT NULL,
  user_id         UUID REFERENCES users(user_id) ON DELETE CASCADE,

  kind            TEXT NOT NULL CHECK (kind IN (
                    'finding','negative_result','instrument_profile',
                    'signal_registry','platform_defect','user_preference',
                    'open_question')),
  subject         TEXT NOT NULL CHECK (length(subject) > 0),
  claim           TEXT NOT NULL CHECK (length(claim) > 0 AND length(claim) <= 300),

  -- {"experiment_id": "exp_…", "ticket_id": "tkt_…"}. The handles a later claim
  -- may cite.
  evidence_ids    JSONB NOT NULL DEFAULT '{}'::jsonb,
  -- Trials, when there are any. `cardinality(...)`, never `array_length(x,1)`:
  -- for an empty array `array_length` returns NULL, `NULL >= 0` is NULL, and a
  -- CHECK that evaluates to NULL **passes**. 0055's comment records the same
  -- trap; this is the same spelling for the same reason.
  evidence_trials UUID[] NOT NULL DEFAULT '{}'::uuid[]
                    CHECK (cardinality(evidence_trials) >= 0),

  -- The structured measurement §2.2.1 step 2 reads. Direction comes from here,
  -- never from parsing the claim's prose: "does not beat" and "beats" differ by
  -- one word a parser gets wrong about as often as it matters.
  quantity        TEXT,
  direction       TEXT CHECK (direction IS NULL OR direction IN ('positive','negative','null')),
  value           DOUBLE PRECISION,
  se              DOUBLE PRECISION CHECK (se IS NULL OR se >= 0),
  n               BIGINT CHECK (n IS NULL OR n >= 0),

  confidence      TEXT NOT NULL DEFAULT 'suggestive'
                    CHECK (confidence IN ('established','suggestive','refuted')),
  evidence_class  TEXT NOT NULL DEFAULT 'derived'
                    CHECK (evidence_class IN ('direct_stat','derived','model_summary','human')),

  window_start    DATE NOT NULL,
  window_end      DATE NOT NULL,
  CONSTRAINT window_is_ordered CHECK (window_end >= window_start),
  -- NULL means "does not expire on a timer": a platform defect is true until
  -- someone fixes it, and a negative result is the cheapest row in the store.
  valid_until     DATE,

  -- P2. Every field of it, and the two that identify the producer are required.
  provenance      JSONB NOT NULL,
  CONSTRAINT provenance_is_complete CHECK (
    provenance ? 'run_id' AND provenance ? 'tool'
    AND length(provenance->>'run_id') > 0
    AND length(provenance->>'tool') > 0
  ),
  created_by      TEXT NOT NULL DEFAULT '',

  superseded_by   UUID REFERENCES knowledge.memory_entry(entry_id),
  stale           BOOLEAN NOT NULL DEFAULT FALSE,
  -- Out of retrieval, still addressable (§2.4). There is no DELETE path in the
  -- application for this table, and that is deliberate: an old dossier's
  -- citations must still resolve.
  archived        BOOLEAN NOT NULL DEFAULT FALSE,

  -- §2.5. A second independent measurement is evidence, not a duplicate, and a
  -- finding corroborated across three windows should outrank a singleton.
  corroborations  INT NOT NULL DEFAULT 1 CHECK (corroborations >= 1),
  corroborated_by TEXT[] NOT NULL DEFAULT '{}'::text[],

  -- Kind-specific: failure_class, retry_condition, coverage, scope, blocking…
  extra           JSONB NOT NULL DEFAULT '{}'::jsonb,
  -- Over the claim TEXT only, never over numbers (0055 §5.6). An embedding of a
  -- metric is a similarity metric nobody designed.
  embedding       vector(768),

  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- §3.2, as a constraint rather than a convention.
  CONSTRAINT negative_result_has_a_closed_class CHECK (
    kind <> 'negative_result' OR (
      extra ? 'failure_class'
      AND extra->>'failure_class' IN (
        'weak_effect','capacity_insufficient','costs_dominate','replication_failed',
        'insufficient_data','no_better_than_null','leakage','protocol_unstable',
        'fragile_parameters','regime_specific')
      -- And it names the ticket the funnel minted. A negative result whose
      -- ticket does not resolve is a claim, and this store does not hold claims.
      AND evidence_ids ? 'ticket_id'
    )
  ),
  -- §3.1 step 2, one layer down from the verifier. `model_summary` is what a
  -- claim with no cited result gets, and it can never be `established`.
  CONSTRAINT a_model_summary_is_never_established CHECK (
    NOT (evidence_class = 'model_summary' AND confidence = 'established')
  )
);

CREATE INDEX IF NOT EXISTS idx_memory_entry_retrieval
  ON knowledge.memory_entry (tenant_id, kind, subject)
  WHERE NOT archived;
CREATE INDEX IF NOT EXISTS idx_memory_entry_expiry
  ON knowledge.memory_entry (valid_until)
  WHERE valid_until IS NOT NULL AND NOT archived;
CREATE INDEX IF NOT EXISTS idx_memory_entry_subject_quantity
  ON knowledge.memory_entry (tenant_id, subject, quantity)
  WHERE kind = 'finding' AND NOT archived AND NOT stale;

ALTER TABLE knowledge.memory_entry ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.memory_entry FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.memory_entry;
CREATE POLICY tenant_isolation ON knowledge.memory_entry
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- ── The grant that is the design (§3.2) ─────────────────────────────────────
--
-- The agent reads everything and writes findings. It does NOT write negative
-- results: those are derived by the platform from a gate refusal, a failed study
-- objective or a `reject` verdict. `memory.record_negative_result` reaches an
-- annotation path on `app_role` and nothing else.
GRANT SELECT ON knowledge.memory_entry TO app_role, agent_role;
GRANT INSERT, UPDATE ON knowledge.memory_entry TO app_role;

-- `agent_role` may insert, but only a kind it is allowed to produce. A row-level
-- WITH CHECK is the enforcement, because a grant alone would let the agent write
-- any kind it liked.
DROP POLICY IF EXISTS agent_writes_findings_only ON knowledge.memory_entry;
CREATE POLICY agent_writes_findings_only ON knowledge.memory_entry
  FOR INSERT TO agent_role
  WITH CHECK (
    tenant_id = current_setting('app.tenant_id', true)
    AND kind IN ('finding','open_question')
  );
GRANT INSERT ON knowledge.memory_entry TO agent_role;
