-- 0122 — the semantic memory store (AGENT-007 §2.2; Set S task S-4.3).
--
-- The keystone the rest of phase 4 waits on. `crates/harness/src/memory/` is
-- about 6,800 lines over 98 passing tests — it computes conflicts, derives
-- negative results, assembles the brief, decides retention — and **none of it
-- persists anything**, because no table exists. The harness has been deriving
-- knowledge and dropping it at the end of every turn.
--
-- ## Numbered 0122 and not 0066
--
-- The phase plan names `0066_semantic_memory.sql`. 0066 was taken long ago and
-- the tree is at 0121, which is the same collision ADR-S3-08 records for S-3.7's
-- `0079` and ADR-P2 records for `agent_events`. Set S's declared migration
-- ranges were allocated against a tree that has since moved; the number is not
-- the contract, the schema is.
--
-- ## This is a mirror of `Entry::validate`, not a second opinion
--
-- Every constraint below already exists in Rust. That is deliberate and it is
-- not redundancy: the harness validates what *it* writes, and the acceptance for
-- this task is that an entry without provenance **cannot be inserted** — which
-- is a claim about the database, not about one caller. A rule enforced only in
-- the writer is a rule that holds until the second writer.
--
-- So the pairs are exact, and a change to one without the other should fail a
-- test rather than drift:
--
--   `EntryError::NoProvenance`  -> four NOT NULL provenance columns, each with a
--                                  non-empty CHECK. All four, because
--                                  `Provenance::is_complete` requires all four:
--                                  a run id with no tool is not provenance.
--   `EntryError::EmptyClaim`    -> CHECK (btrim(claim) <> '')
--   `EntryError::ClaimTooLong`  -> CHECK (char_length(claim) <= 300)
--   `EntryError::EmptySubject`  -> CHECK (btrim(subject) <> '')
--   `EntryError::NoWindow`      -> both window ends NOT NULL
--
-- ## Seven kinds, not six
--
-- The plan says "the six entry kinds", which is stale: it counts §2's summary
-- row, written before `open_question` was added by AGENT-013 §9. The spec's own
-- §2.2 table and `memory::Kind` both carry seven, and they agree with each
-- other. This follows them.
--
-- ## Recomputable, like the rest of 0055
--
-- No WORM, no hash chain, no retention guarantee. This is what the platform has
-- *learned*, and every entry carries the provenance to re-derive it. The ledger
-- is the record; this is an index over it. The one thing that is not free to
-- delete is an entry an evidence chain cites — `cited_by_chain` — because
-- deleting it would break a sealed ticket's citation, and §2.4 says nothing
-- cited is ever hard-deleted.

CREATE SCHEMA IF NOT EXISTS agent_memory;

CREATE TABLE IF NOT EXISTS agent_memory.entry (
  entry_id        TEXT PRIMARY KEY,
  user_id         UUID NOT NULL,

  -- `memory::Kind`. A CHECK rather than a Postgres enum, following the
  -- convention in 0036: adding a kind is then a migration and not an ALTER TYPE
  -- that cannot run inside a transaction on older servers.
  kind            TEXT NOT NULL CHECK (kind IN (
                    'finding',
                    'negative_result',
                    'instrument_profile',
                    'signal_registry',
                    'platform_defect',
                    'user_preference',
                    'open_question'
                  )),

  -- Instrument, hypothesis family, strategy, or platform component.
  subject         TEXT NOT NULL CHECK (btrim(subject) <> ''),
  -- 300 chars, counted in characters and not bytes, because `Entry::validate`
  -- counts `chars()`. A byte limit would reject a claim the harness accepted.
  claim           TEXT NOT NULL
                    CHECK (btrim(claim) <> '' AND char_length(claim) <= 300),

  -- `{"experiment_id": "exp_…"}` — the ids the claim rests on, by role.
  evidence_ids    JSONB NOT NULL DEFAULT '{}'::jsonb
                    CHECK (jsonb_typeof(evidence_ids) = 'object'),

  -- `memory::Measurement`, flattened. Null together or present together: a
  -- direction without a value is not a measurement.
  quantity        TEXT,
  -- `memory::Direction`. `null` is a value here and not an absence: "measured
  -- and indistinguishable from zero" is a result, which is why the column is
  -- text and the string is spelled out.
  direction       TEXT CHECK (direction IS NULL OR direction IN ('positive', 'negative', 'null')),
  value           DOUBLE PRECISION,
  se              DOUBLE PRECISION CHECK (se IS NULL OR se >= 0),
  n               BIGINT CHECK (n IS NULL OR n >= 0),
  CONSTRAINT measurement_is_whole_or_absent CHECK (
    (quantity IS NULL AND direction IS NULL AND value IS NULL)
    OR (quantity IS NOT NULL AND btrim(quantity) <> '' AND direction IS NOT NULL AND value IS NOT NULL)
  ),

  confidence      TEXT NOT NULL
                    CHECK (confidence IN ('established', 'suggestive', 'refuted')),
  evidence_class  TEXT NOT NULL
                    CHECK (evidence_class IN ('direct_stat', 'derived', 'model_summary', 'human')),

  -- §2.3: a model's summary of something it read is never `established`. The
  -- harness decides this and the database refuses the combination, so the rule
  -- survives a second writer.
  CONSTRAINT a_model_summary_is_never_established CHECK (
    NOT (evidence_class = 'model_summary' AND confidence = 'established')
  ),

  -- `memory::DataWindow`. Both ends required: a finding is a claim about a
  -- window, never about a market in general.
  window_start    DATE NOT NULL,
  window_end      DATE NOT NULL,
  CONSTRAINT the_window_runs_forwards CHECK (window_end >= window_start),

  -- Null for the kinds that never expire on a timer.
  valid_until     DATE,

  -- `memory::Provenance`, all four parts, all required. This is the acceptance.
  prov_run_id         TEXT NOT NULL CHECK (btrim(prov_run_id) <> ''),
  prov_tool           TEXT NOT NULL CHECK (btrim(prov_tool) <> ''),
  prov_tool_version   TEXT NOT NULL CHECK (btrim(prov_tool_version) <> ''),
  prov_template_hash  TEXT NOT NULL CHECK (btrim(prov_template_hash) <> ''),

  created_by      TEXT NOT NULL CHECK (btrim(created_by) <> ''),

  -- Supersession points forward to the entry that replaced this one. Not a
  -- foreign key to itself: the replacement is written first in some paths and
  -- last in others, and a constraint that forced an order would be a constraint
  -- about the writer rather than about the data.
  superseded_by   TEXT,
  -- §2.2.1: conflicts are **surfaced and never merged**, so this is a list and
  -- there is no resolution column. A schema with a `winner` would invite one.
  conflicts_with  TEXT[] NOT NULL DEFAULT '{}',

  stale           BOOLEAN NOT NULL DEFAULT false,
  archived        BOOLEAN NOT NULL DEFAULT false,

  -- §2.5. Starts at 1 — the run that wrote it — so a zero is a bug rather than
  -- a fresh entry.
  corroborations  INTEGER NOT NULL DEFAULT 1 CHECK (corroborations >= 1),
  corroborated_by TEXT[] NOT NULL DEFAULT '{}',

  -- §2.4: nothing an evidence chain cites is ever hard-deleted. Retention reads
  -- this before it deletes anything.
  cited_by_chain  BOOLEAN NOT NULL DEFAULT false,

  -- Kind-specific fields — §2.2's right-hand column: `failure_class`,
  -- `coverage`, `scope`, `why_it_matters`, and the rest. JSONB rather than
  -- seven sparse column sets, because they share nothing and the shared fields
  -- above are the ones anything queries on.
  extra           JSONB NOT NULL DEFAULT '{}'::jsonb
                    CHECK (jsonb_typeof(extra) = 'object'),

  -- pgvector, as 0055 established. Nullable: an entry is usable before it has
  -- been embedded, and the brief's retrieval is lexical first (§4).
  embedding       vector(768),

  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The brief's retrieval path: this user's live entries for a subject, newest
-- first. Archived rows are excluded from the index rather than filtered at read
-- time, because the live set is the one every query wants.
CREATE INDEX IF NOT EXISTS entry_live_by_subject
  ON agent_memory.entry (user_id, subject, created_at DESC)
  WHERE NOT archived;

CREATE INDEX IF NOT EXISTS entry_by_kind
  ON agent_memory.entry (user_id, kind, created_at DESC)
  WHERE NOT archived;

-- Retention (§2.4) scans by expiry, and the conflict pass scans for entries
-- that already carry one.
CREATE INDEX IF NOT EXISTS entry_expiring
  ON agent_memory.entry (valid_until)
  WHERE valid_until IS NOT NULL AND NOT archived;

CREATE INDEX IF NOT EXISTS entry_conflicted
  ON agent_memory.entry USING GIN (conflicts_with);

-- The near-duplicate check (§2.2.1) is "same subject, same measured quantity,
-- overlapping window" — so it looks up exactly this.
CREATE INDEX IF NOT EXISTS entry_by_quantity
  ON agent_memory.entry (user_id, subject, quantity)
  WHERE quantity IS NOT NULL AND NOT archived;
