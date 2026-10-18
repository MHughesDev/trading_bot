-- A definition names the revision it serves, or it is not a definition.
--
-- 0058 made `strategy_id` a pointer to a content-addressed, append-only
-- revision, and deliberately left `head_revision` nullable for two states:
-- rows written before 0058, and the window inside 0058's own backfill. Both
-- are spent. What kept using the hatch afterwards was
-- `crates/ledger/tests/pg_strategy_revision.rs`, which wrote against
-- `DATABASE_URL` — the database the platform serves from — and committed. Six
-- runs of it left 18 rows shaped
--
--     strategy_id     s_<32 hex>      (a bare UUID, not a slug anyone chose)
--     definition_json {}
--     head_revision   NULL
--
-- `list_strategies` selects `strategy_id` with no predicate, so all 18 were in
-- the list the UI and the MCP catalogue read. `get_strategy` then deserialises
-- `definition_json` into `domain::strategy_def::StrategyDefinition`, which has
-- no defaults for `strategy_id`, `inputs`, `nodes` or `actions` — so `{}` is a
-- 500, not a 404, and `sim_executor`, `backtests` and `automations` read the
-- same column. Of the 32 rows in the table, 14 deserialised.
--
-- Two halves, and they have to be in this order: remove the rows, then remove
-- the shape.

-- ── the residue ──────────────────────────────────────────────────────────────
--
-- Four conditions, all of them together. Any real definition fails at least one
-- of the first three: the slug pattern is a bare `Uuid::new_v4().simple()` that
-- no human names, `{}` is not a document, and the platform's own write path
-- sets a head in the same transaction that appends the revision. The
-- `NOT EXISTS` is the one that would save us if that reasoning were wrong — a
-- slug something is running is not residue whatever else it looks like.
--
-- `created_by IS NULL` is NOT one of the conditions, though it is true of all 18
-- and reads like the obvious fifth. It is true of every other row too:
-- `routes/strategies.rs` does not bind `created_by` at all, so all 14 survivors
-- have a NULL author as well. A predicate that matches everything is not
-- evidence, and including it here would have implied a check that was not
-- happening. (The column being dead is a separate finding, recorded against
-- Set S Phase 5 with the `actor_kind` one it belongs with.)
DELETE FROM strategy_definitions d
 WHERE d.head_revision IS NULL
   AND d.definition_json = '{}'::jsonb
   AND d.strategy_id ~ '^s_[0-9a-f]{32}$'
   AND NOT EXISTS (
         SELECT 1 FROM strategy_instances i WHERE i.strategy_id = d.strategy_id
       );

-- The revisions those same runs wrote are NOT removed, and cannot be: 40 rows
-- with `actor_kind = 'agent'`, holding seq 14..53 of a hash chain whose whole
-- point is that nothing can be taken out of the middle of it. They are
-- unreachable rather than wrong — `strategy_revision` is only ever read by
-- primary key (`lifecycle.rs`), and with the definitions gone nothing holds
-- those keys. An append-only table keeps its mistakes; that is the trade it
-- makes, and the fix for it is upstream, in the test that no longer commits.

-- ── the shape ────────────────────────────────────────────────────────────────
--
-- SET NOT NULL reports the column and not the rows, which is the wrong end of
-- the problem to be handed at migration time. Name them.
DO $$
DECLARE offenders TEXT;
BEGIN
  SELECT string_agg(strategy_id, ', ' ORDER BY strategy_id) INTO offenders
    FROM strategy_definitions WHERE head_revision IS NULL;
  IF offenders IS NOT NULL THEN
    RAISE EXCEPTION
      'these definitions have no head revision and are not the test residue 0140 removes: % — '
      'each one is a slug whose document cannot be reconstructed from any immutable record. '
      'Give it a revision (POST /api/strategies re-saves it) or delete it, then re-run.',
      offenders;
  END IF;
END $$;

ALTER TABLE strategy_definitions
  ALTER COLUMN head_revision SET NOT NULL;

-- `strategy_head_must_match_revision()` (0058) still opens with an early return
-- on a NULL head. That branch is now unreachable and is left alone on purpose:
-- editing an applied migration changes its checksum, and a guard that has
-- become redundant is not a guard that has become wrong.
--
-- Not done here, deliberately: the same treatment for
-- `strategy_instances.revision_hash`. That one is nullable because instances
-- predating 0058 were bound to a mutable name, and backfilling them would
-- assert something untrue — we know what the definition says now, not what it
-- said when the instance started. There are no rows to clean and no claim we
-- can honestly make, so the column stays as it is.
