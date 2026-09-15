-- The evaluator ignored declared exits: the defect, and what happens to the
-- evidence computed under it (ADR-P6-11, ADR-P6-12, ADR-P6-13).
--
-- `ExitTracker` was built, tested and never called. A strategy could declare a
-- stop-loss, a take-profit, a trailing stop or a time exit as typed fields at
-- `definition_version` 1.3; the authoring surface accepted them; and
-- `crates/backtest/src/sim.rs` ignored every one. So a definition's exits
-- changed live behaviour and not backtest results, which is the more dangerous
-- direction of the two: every backtest of an exit-declaring strategy is
-- evidence about a strategy nobody declared. Usually a worse one, sometimes a
-- much better one, always the wrong one.
--
-- The same reading found a second one. The SDK's instrument constructors leave
-- maker and taker unset, and the engine's default fee model reads that as zero
-- commission on every fill, so **every** run this executor has produced —
-- exits or not — reported a gross number and labelled it net.
--
-- ## Why this is filed rather than noted
--
-- ADR-P6-06 put `report_evaluator_defect` in the platform precisely so that
-- "the simulator is wrong" is worth more than any result obtained from the
-- simulator being wrong, and encoded it as `mlops.falsification_score()`:
-- evaluator_defect > promotion > kill > zero. A mechanism nobody files into
-- when it plainly applies is decoration, and the next agent reading the table
-- would reasonably conclude that filing is not what one actually does here.
--
-- It is filed **verified and discharged**: the reproduction is a test that
-- fails on the old engine and passes on the new one, and it is named in
-- `regression_test`. That is a maintainer filing a defect together with its fix
-- — not the self-verification the GRANTs in 0059 forbid, which is about
-- `agent_role` having INSERT and no UPDATE. A reviewer can re-run the named
-- test and disagree.
--
-- ## Why the affected runs are not marked
--
-- They cannot be. `backtest_runs` is append-only by trigger (0026): UPDATE and
-- DELETE both raise. That is correct and is not worked around here. The mark
-- goes on the **engine version** instead, which is the only thing that
-- separates the two populations — `EXECUTOR_TAG` gained a `+exits` suffix in
-- the same change — and a view joins it back at read time.
--
-- Nothing is deleted and nothing is re-scored. A run computed under a defective
-- engine is still a record of a trial that happened; what it is not is evidence
-- about the strategy it names.

-- ── the engine versions known to be defective ───────────────────────────────
CREATE TABLE IF NOT EXISTS mlops.defective_engine_version (
  engine_version  TEXT PRIMARY KEY,
  -- Which part of the engine was wrong, in one line, for a reader looking at a
  -- flagged run rather than at this file.
  defect          TEXT NOT NULL CHECK (length(trim(defect)) > 0),
  -- The ADR that decided what to do about it.
  adr             TEXT NOT NULL,
  recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE mlops.defective_engine_version IS
  'Engine versions whose results are known to be wrong. Append-only in practice: a version is '
  'never un-marked, because the runs it produced do not become correct later. Joined to '
  'backtest_runs by engine_version through the backtest_runs_integrity view — the runs table '
  'itself is immutable by trigger and must stay that way.';

INSERT INTO mlops.defective_engine_version (engine_version, defect, adr) VALUES
  ('SimRunExecutor@market_simulator',
   'Ignored the declared exit policy (ExitTracker was never called), so a stop-loss, take-profit, '
   'trailing stop or time exit declared at definition_version 1.3 had no effect on the result; '
   'and charged zero commission on every fill, because the simulated instrument carried no '
   'maker/taker rate.',
   'ADR-P6-11, ADR-P6-12, ADR-P6-13'),
  ('backtest@0.1.0+sim-sdk',
   'The generic engine tag in use while the simulator ignored declared exits and charged zero '
   'commission. Results produced under it share the defect of '
   'SimRunExecutor@market_simulator.',
   'ADR-P6-11, ADR-P6-12, ADR-P6-13')
ON CONFLICT (engine_version) DO NOTHING;

-- ── the read-time join ──────────────────────────────────────────────────────
--
-- Every consumer of stored runs should read this instead of backtest_runs. It
-- adds two columns and removes nothing.
CREATE OR REPLACE VIEW backtest_runs_integrity AS
SELECT
  r.*,
  (d.engine_version IS NOT NULL)          AS computed_by_defective_engine,
  d.defect                                AS engine_defect
FROM backtest_runs r
LEFT JOIN mlops.defective_engine_version d
  ON d.engine_version = r.engine_version;

COMMENT ON VIEW backtest_runs_integrity IS
  'backtest_runs with the engine-defect mark attached. `computed_by_defective_engine` true means '
  'the row is a record that a trial happened and is not evidence about the strategy it names.';

-- ── the defect report itself ────────────────────────────────────────────────
--
-- Filed once per tenant that actually holds affected runs. mlops.falsification
-- is under row-level security keyed on tenant_id (0059), so a single row under
-- a synthetic 'platform' tenant would be invisible to every real one — which
-- would file the report and still leave nobody able to read it.
--
-- A tenant with no stored runs gets no row: a defect report about evidence
-- nobody holds is noise in the one table that must stay worth reading.
DO $$
DECLARE
  v_tenant        TEXT;
  v_runs          BIGINT;
  v_total         BIGINT := 0;
  v_tenants       INT := 0;
BEGIN
  FOR v_tenant, v_runs IN
    SELECT COALESCE(r.created_by, 'unknown'), count(*)
    FROM backtest_runs r
    JOIN mlops.defective_engine_version d ON d.engine_version = r.engine_version
    GROUP BY 1
  LOOP
    v_total   := v_total + v_runs;
    v_tenants := v_tenants + 1;

    -- 0059 sets FORCE ROW LEVEL SECURITY, so the policy applies to the table
    -- owner too and an INSERT with `app.tenant_id` unset fails its WITH CHECK
    -- (`tenant_id = NULL` is NULL, and NULL is not true). Set it per tenant,
    -- transaction-locally, rather than disabling the policy for the migration:
    -- a migration that turns tenant isolation off to write one row is a
    -- migration that teaches the next one to do the same.
    PERFORM set_config('app.tenant_id', v_tenant, true);

    INSERT INTO mlops.falsification (
      tenant_id, report_kind, component, claimed_guarantee,
      artifact_ref, seed, expected, observed, steps,
      verification, verified_by, verified_at, regression_test,
      reported_by, reporter_kind
    ) VALUES (
      v_tenant,
      'evaluator_defect',
      'backtest simulator (crates/backtest/src/sim.rs)',
      -- The claim. A defect is a gap between a claim and a behaviour; without
      -- the claim there is only a behaviour, and behaviours are not defects.
      'A Run is a faithful evaluation of the strategy definition it names. '
      'domain::strategy_def::exits: exits are "typed fields, evaluated by the runtime against '
      'position state", REQUIRED at definition_version 1.3. '
      'crates/backtest/src/lib.rs: the simulator "simulates fills per asset class" and returns '
      'results; RunResult.metrics are net of costs.',
      'crates/backtest/src/sim.rs @ engine SimRunExecutor@market_simulator',
      0,
      'A strategy declaring exits: {"mode":"rules","rules":[{"rule":"stop_loss","fraction":"0.02"}]} '
      'entered long at 100 is closed at 98 on the first bar whose low breaches 98, and every fill '
      'is charged the venue fee schedule.',
      'The position was never closed by the stop: ExitTracker existed and sim.rs never constructed '
      'it, so the declared policy had no effect on any backtest. On a bar that traded down to 97 '
      'and closed at 100.80 the position stayed open, and on a bar breaching both a 2% stop and a '
      '5% target the run booked the target. Every fill was also charged zero commission, because '
      'the simulated instrument carried no maker/taker rate.',
      ARRAY[
        'Store a definition at definition_version 1.3 whose exits declare stop_loss 0.02.',
        'Run it over bars: bar 0 flat at 100 (entry fills at the close), bar 1 h=101 l=99.5 '
          'c=100.5, bar 2 h=101 l=97 c=100.8.',
        'Observe on the defective engine: zero closed positions, and the position still open at '
          'the end of the run.',
        'Expected: one closed position at ts(bar 2) with exit price 98.00 and a negative P&L.',
        'Regression: cargo test -p backtest --test declared_exits -j 2',
        'The intrabar half: a_bar_that_breached_both_books_the_stop_and_never_the_target fails on '
          'the defective engine with a +2.76 profit where a loss is required.'
      ],
      'verified',
      'track-15 (ADR-P6-11); reproduced by the named regression test, which fails on '
      'SimRunExecutor@market_simulator and passes on SimRunExecutor@market_simulator+exits',
      now(),
      'crates/backtest/tests/declared_exits.rs::a_declared_stop_loss_fires_on_the_breaching_bar_and_fills_at_its_level',
      'migration 0120 (ADR-P6-13)',
      'scheduler'
    );
  END LOOP;

  PERFORM set_config('app.tenant_id', '', true);

  -- The count is emitted rather than stored: it is a property of the database
  -- this ran against, and writing it down would make it look like a property of
  -- the defect.
  RAISE NOTICE
    'ADR-P6-13: % stored run(s) across % tenant(s) were computed by an engine that ignored '
    'declared exits and charged no commission. They are marked through '
    'backtest_runs_integrity.computed_by_defective_engine, not rewritten: backtest_runs is '
    'append-only.',
    v_total, v_tenants;

  IF v_total = 0 THEN
    RAISE NOTICE
      'ADR-P6-13: no affected runs in this database. The defect report is still worth filing '
      'against any database that has them, and the engine-version mark is in place for runs '
      'restored from a backup.';
  END IF;
END $$;

GRANT SELECT ON mlops.defective_engine_version TO app_role, agent_role, internal_ml_role;
GRANT SELECT ON backtest_runs_integrity TO app_role, agent_role, internal_ml_role;
