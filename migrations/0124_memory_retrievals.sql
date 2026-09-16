-- 0124 — the retrieval log (AGENT-007 §4; AGENT-009 §4; Set S task S-4.5).
--
-- ## The one place two specs meet
--
-- `memory_retrievals.cited` is the memory-recall signal AGENT-009 §4 scores.
-- The memory half writes a row for every entry it put in front of the model;
-- the evaluation half reads the share of them the model went on to name. It is
-- the only field either spec takes from the other, and both sides were already
-- built around it: `memory::brief::Retrieval` produces the row and
-- `eval::scorecard`'s `memory_recall` consumes the share. Nothing stored it.
--
-- Without the table the signal is not merely absent, it is **unfalsifiable**:
-- a brief that retrieved nothing useful and a brief that retrieved perfectly
-- both score `None`, and the retrieval policy could drift for months with the
-- scorecard reporting the same blank.
--
-- ## Numbered 0124, not the plan's 0067
--
-- The ADR-S3-08 collision again. 0122 is the semantic store, 0123 the turn
-- summaries, this follows.
--
-- ## `cited` is set later, by a different act
--
-- A row is written when the brief is assembled and updated when a later
-- `record_finding` or `finish_task` names the entry. So the table is written
-- twice per row, by two different parts of the run, and the second write is the
-- measurement. That is why `cited` is a column with a default rather than
-- something derived at read time: derived, it would be a join against whatever
-- the citation story looks like on the day somebody asks, and the score would
-- change retroactively when that story changed.
--
-- ## No foreign key to the entry
--
-- Deliberate, and it is the retention rule (§2.4) that forces it. An entry that
-- nothing cites may be hard-deleted; a retrieval row recording that it was once
-- shown and not cited is exactly the evidence the score is computed from. A
-- cascade would delete the misses and leave the hits, which would move
-- `memory_recall` upward every time the store was tidied — a metric that
-- improves when you delete data is worse than no metric.

CREATE TABLE IF NOT EXISTS memory_retrievals (
  id          BIGSERIAL PRIMARY KEY,
  run_id      TEXT NOT NULL CHECK (btrim(run_id) <> ''),
  -- Not a foreign key; see the header.
  entry_id    TEXT NOT NULL CHECK (btrim(entry_id) <> ''),

  -- Where it sat in the brief. Rank 1 is the top item of its slot, so a recall
  -- share can be read against position — an entry cited from rank 12 and one
  -- cited from rank 1 say different things about the scorer.
  rank        INTEGER NOT NULL CHECK (rank >= 1),

  -- `memory::brief::Slot`, serde `snake_case`. The names are the enum's, not a
  -- shorthand: `crates/invariants/tests/schema_mirrors_memory_types.rs` compares
  -- the two, and a readable abbreviation here would be a second vocabulary.
  slot        TEXT NOT NULL CHECK (slot IN (
                'user_preference',
                'instrument_profile',
                'finding',
                'negative_result',
                'open_question',
                'conflict'
              )),

  -- P4: why it was retrieved. Required and non-empty, because a retrieval
  -- nobody can explain is a retrieval nobody can debug — which is the whole
  -- reason §4 asks for it.
  why         TEXT NOT NULL CHECK (btrim(why) <> ''),

  -- The measurement. False until a later act names the entry.
  cited       BOOLEAN NOT NULL DEFAULT false,
  cited_at    TIMESTAMPTZ,
  -- A citation that claims no time is a citation nobody can order against the
  -- run, so the two move together.
  CONSTRAINT cited_carries_its_moment CHECK (cited = (cited_at IS NOT NULL)),

  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- One row per entry per run. A brief assembled twice in one run — a re-plan,
  -- a resumed step — must not count the same entry twice, or a run that
  -- re-planned would score a different recall from one that did not for reasons
  -- having nothing to do with memory.
  UNIQUE (run_id, entry_id)
);

-- The score: the cited share for a run, or across runs.
CREATE INDEX IF NOT EXISTS memory_retrievals_by_run
  ON memory_retrievals (run_id, cited);

-- The other question worth asking of this table: is this entry ever useful?
-- An entry retrieved forty times and never cited is a retrieval-policy bug or a
-- worthless entry, and the two are distinguishable only by looking.
CREATE INDEX IF NOT EXISTS memory_retrievals_by_entry
  ON memory_retrievals (entry_id, cited);
