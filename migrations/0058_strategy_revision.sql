-- Immutable strategy revisions (EXTERNAL-REVIEW-2026-09 §3.1, ADR-P6-04).
--
-- `strategy_definitions` was the last mutable thing in the evidence path. A
-- re-save was `ON CONFLICT (strategy_id) DO UPDATE`, which meant:
--
--   * the definition a live instance is running was not reconstructable from
--     any immutable record, and
--   * `RunConfig.strategy_version` hashed a document that could be replaced
--     underneath it, so two runs sharing a `strategy_version` were only
--     comparable if nobody had re-saved in between — a condition nothing
--     checked and nothing recorded.
--
-- The trial ledger already proves the pattern. This applies it one layer out:
-- `strategy_id` becomes a *pointer*, and everything that constitutes evidence
-- binds to a content-addressed revision that cannot be edited or deleted.
--
-- Deliberately NOT done here: dropping the `definition_json` column from
-- `strategy_definitions`. Reads all over the platform still go through it, and
-- a migration that breaks every read to enforce an invariant on writes is a
-- migration that gets reverted. It is now a cache of the head revision, and
-- `trg_strategy_definitions_head` keeps it honest.

-- ── the revision: content-addressed, append-only ─────────────────────────────
CREATE TABLE IF NOT EXISTS strategy_revision (
  -- sha256 over the canonical JSON serialization (RFC 8785 shape: sorted keys,
  -- no insignificant whitespace). The identity *is* the content.
  revision_hash       TEXT PRIMARY KEY CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
  strategy_id         TEXT NOT NULL,
  -- The revision this one was edited from, when it was edited from one. NULL
  -- for a first revision. Not a foreign key to itself on purpose: a revision
  -- imported from another tenant's export names a parent we may not hold, and
  -- refusing the import loses more than the dangling pointer costs.
  parent_hash         TEXT,
  definition_version  TEXT NOT NULL,
  asset_class         TEXT NOT NULL,
  -- The canonical form, for querying.
  definition_json     JSONB NOT NULL,
  -- The bytes exactly as authored, before canonicalisation. Kept because
  -- canonicalisation is lossy in the ways that matter here: it reorders keys and
  -- can round an exact decimal string through a binary float. Reproducing a
  -- 2026 result in 2028 means replaying what was actually sent.
  definition_bytes    BYTEA NOT NULL,
  -- Who put it there. `actor_kind` distinguishes a human author from the
  -- research agent, which is the difference between a hypothesis and a sample.
  actor_kind          TEXT NOT NULL CHECK (actor_kind IN ('human','agent','scheduler','import')),
  actor_id            TEXT NOT NULL,
  authored_at         TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- Hash chain, same construction as mlops.trial: a revision cannot be removed
  -- from the middle of the history without breaking every hash after it.
  seq                 BIGINT NOT NULL,
  prev_hash           BYTEA  NOT NULL,
  row_hash            BYTEA  NOT NULL,

  CONSTRAINT uq_strategy_revision_seq UNIQUE (seq),
  -- A revision that is its own parent is a cycle of length one, and the only
  -- way to write one is a bug in the hasher.
  CONSTRAINT chk_revision_not_its_own_parent CHECK (parent_hash IS DISTINCT FROM revision_hash)
);

CREATE INDEX IF NOT EXISTS idx_strategy_revision_strategy
  ON strategy_revision(strategy_id, authored_at DESC);
CREATE INDEX IF NOT EXISTS idx_strategy_revision_parent
  ON strategy_revision(parent_hash);

CREATE OR REPLACE FUNCTION public.strategy_revision_chain()
RETURNS TRIGGER AS $$
DECLARE
  tip_hash BYTEA;
  tip_seq  BIGINT;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('public.strategy_revision'));
  SELECT row_hash, seq INTO tip_hash, tip_seq
    FROM strategy_revision ORDER BY seq DESC LIMIT 1;
  IF tip_hash IS NULL THEN
    tip_hash := '\x0000000000000000000000000000000000000000000000000000000000000000'::BYTEA;
    tip_seq  := -1;
  END IF;

  NEW.seq       := tip_seq + 1;
  NEW.prev_hash := tip_hash;
  NEW.row_hash  := sha256(
       tip_hash
    || convert_to(NEW.revision_hash, 'UTF8')
    || convert_to(NEW.seq::TEXT, 'UTF8')
    || convert_to(NEW.strategy_id, 'UTF8')
    || convert_to(COALESCE(NEW.parent_hash, ''), 'UTF8')
    || convert_to(NEW.definition_version, 'UTF8')
    || convert_to(NEW.asset_class, 'UTF8')
    || NEW.definition_bytes
    || convert_to(NEW.actor_kind, 'UTF8')
    || convert_to(NEW.actor_id, 'UTF8')
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_revision_chain ON strategy_revision;
CREATE TRIGGER trg_strategy_revision_chain
  BEFORE INSERT ON strategy_revision
  FOR EACH ROW EXECUTE FUNCTION public.strategy_revision_chain();

-- Append-only. Reuses mlops.refuse_mutation() from 0043 rather than defining a
-- second one: two functions that must stay identical eventually do not.
DROP TRIGGER IF EXISTS trg_strategy_revision_immutable ON strategy_revision;
CREATE TRIGGER trg_strategy_revision_immutable
  BEFORE UPDATE OR DELETE ON strategy_revision
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── strategy_id becomes a pointer ────────────────────────────────────────────
ALTER TABLE strategy_definitions
  ADD COLUMN IF NOT EXISTS head_revision TEXT REFERENCES strategy_revision(revision_hash);

-- ── an instance binds to a revision, not to a name ───────────────────────────
--
-- This is the column that makes a running strategy reconstructable. Nullable
-- because instances predating this migration were bound to a mutable name and
-- backfilling them would assert something untrue: we know what the definition
-- says *now*, not what it said when the instance started.
ALTER TABLE strategy_instances
  ADD COLUMN IF NOT EXISTS revision_hash TEXT REFERENCES strategy_revision(revision_hash);

CREATE INDEX IF NOT EXISTS idx_strategy_instances_revision
  ON strategy_instances(revision_hash);

-- ── backfill: one revision per existing definition ───────────────────────────
--
-- The hash here is over `definition_json::text`, which is Postgres's own JSONB
-- text form — NOT the application's canonical form. They agree on key order
-- (both sort) and can disagree on numeric formatting. A backfilled revision is
-- therefore content-addressed under a *slightly different* function than a
-- freshly written one, and `actor_kind = 'import'` is how you tell which is
-- which. Recomputing them through the application hasher would be better and is
-- not worth taking the platform down for.
INSERT INTO strategy_revision (
  revision_hash, strategy_id, parent_hash, definition_version, asset_class,
  definition_json, definition_bytes, actor_kind, actor_id, authored_at,
  seq, prev_hash, row_hash
)
SELECT
  encode(sha256(convert_to(d.definition_json::text, 'UTF8')), 'hex'),
  d.strategy_id,
  NULL,
  d.definition_version,
  d.asset_class,
  d.definition_json,
  convert_to(d.definition_json::text, 'UTF8'),
  'import',
  COALESCE(d.created_by::TEXT, 'pre-0058'),
  d.created_at,
  0, '\x'::BYTEA, '\x'::BYTEA   -- overwritten by the chain trigger
FROM strategy_definitions d
WHERE d.head_revision IS NULL
ON CONFLICT (revision_hash) DO NOTHING;

UPDATE strategy_definitions d
SET head_revision = encode(sha256(convert_to(d.definition_json::text, 'UTF8')), 'hex')
WHERE d.head_revision IS NULL;

-- ── the cache cannot drift from the revision it points at ────────────────────
--
-- `definition_json` on `strategy_definitions` is now a denormalised copy of the
-- head revision. A copy that nothing checks is a copy that is wrong, so this
-- refuses a write where the two disagree. The check is on the pointer, not on
-- the bytes: comparing JSONB equality would reject a legitimate re-canonicalisation.
CREATE OR REPLACE FUNCTION public.strategy_head_must_match_revision()
RETURNS TRIGGER AS $$
DECLARE
  rev_strategy TEXT;
BEGIN
  IF NEW.head_revision IS NULL THEN
    RETURN NEW;   -- pre-0058 rows and the backfill's own intermediate state
  END IF;
  SELECT strategy_id INTO rev_strategy
    FROM strategy_revision WHERE revision_hash = NEW.head_revision;
  IF rev_strategy IS NULL THEN
    RAISE EXCEPTION 'head_revision % does not exist', NEW.head_revision;
  END IF;
  IF rev_strategy <> NEW.strategy_id THEN
    RAISE EXCEPTION
      'head_revision % belongs to strategy %, not % — a definition may not point at another strategy''s revision',
      NEW.head_revision, rev_strategy, NEW.strategy_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_definitions_head ON strategy_definitions;
CREATE TRIGGER trg_strategy_definitions_head
  BEFORE INSERT OR UPDATE ON strategy_definitions
  FOR EACH ROW EXECUTE FUNCTION public.strategy_head_must_match_revision();

-- Same rule for instances: an instance may not point at another strategy's
-- revision. Without this an instance could claim to be running a definition it
-- has no relationship to, and the binding would look perfectly well-formed.
CREATE OR REPLACE FUNCTION public.instance_revision_must_match_strategy()
RETURNS TRIGGER AS $$
DECLARE
  rev_strategy TEXT;
BEGIN
  IF NEW.revision_hash IS NULL THEN
    RETURN NEW;
  END IF;
  SELECT strategy_id INTO rev_strategy
    FROM strategy_revision WHERE revision_hash = NEW.revision_hash;
  IF rev_strategy IS DISTINCT FROM NEW.strategy_id THEN
    RAISE EXCEPTION
      'instance names strategy % but revision % belongs to %',
      NEW.strategy_id, NEW.revision_hash, COALESCE(rev_strategy, '(missing)');
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_instances_revision ON strategy_instances;
CREATE TRIGGER trg_strategy_instances_revision
  BEFORE INSERT OR UPDATE ON strategy_instances
  FOR EACH ROW EXECUTE FUNCTION public.instance_revision_must_match_strategy();
