# Runbook: deploy one hand-authored strategy and start the forward clock

Set T, task T-0.4 (the operations half). Written 2026-10-04 by track F-DATA-T.

**Why this exists.** Gate 15's evidence is a calendar that cannot be compressed: every
day with no deployed strategy is a day with no forward observations. Engineering is
done (the recorder, the shadow run that supplies the expected series, the observation
store, the ramp review, the monitor). What is left is a person starting the clock. The
deploy needs a running platform and a real strategy, which is why the last step below
is `BLOCKED:HUMAN`.

Nothing here moves money. The only venue is the platform's own paper venue
(`platform_paper`, ADR-0048); `deployment_proposal.venue_class` is a CHECK of one value.

## 0. Preconditions (check, do not assume)

| Check | How | Expect |
|---|---|---|
| Platform is up on port 7080 and migrations are applied through 0190 | the platform answers on :7080; `select max(version) from _sqlx_migrations` | 190 or later |
| The monitor lease is renewing | `GET /api/monitor/coverage`; the `tbot-monitor` service is running | a lease row, not expired. With no lease the order gate **refuses every order** (COMP-007 §4) |
| Fleet ceilings are declared | `GET /api/deployment-ceilings` | three rows from migration 0190 (ADR-T2-04), or your own |
| `ramp_policy` has a row | `select * from current_ramp_policy` | 14 days / 20 bars (ADR-T3-04) |
| Live bars are flowing for the instrument | `GET /api/data/coverage`, the instrument's 1m row is not `not accruing` | fresh |
| A registered trial for the strategy | `POST /api/backtest/experiments`, then run a Study (the Experiment path), note the `trial_id` | a UUID |

## 1. Choose the strategy

Pick the simplest hand-authored v3 strategy that has already been through an Experiment
(an EMA cross on BTC-USD 1h is the reference). It does not need to be good; it needs to
produce observations. Record, in the table at the bottom of this file:

* the strategy slug and its revision hash,
* the instrument and `bar_seconds` (use 3600 for the first one: a 14-day period then
  holds 336 bars against the 20-bar floor),
* the declared divergence thresholds and kill criteria (step 3).

## 2. Propose, then have a human decide (T-1.1)

The proposal is the only door once the gate is armed (see "Arming" below); use it now so
the first deployment is already on the path rather than a `legacy_ungated` one.

```
POST /api/deployment-proposals
{ "strategy_name": "<slug>", "instrument_id": "BTC-USD", "profile_id": "paper_v1",
  "starting_capital": "10000", "review_period_days": 14, ... }
```

The proposal body is `ledger::deployment_path::ProposalDraft` (all sixteen gate verdicts,
the thresholds, capital, period). It is created `pending`. **A person decides it**:

```
POST /api/deployment-proposals/{id}/decision
{ "decision": "approved", "note": "<why>" }
```

An agent token cannot decide (`auth::scopes::NEVER_FOR_AGENTS`; the test
`no_scope_an_agent_can_hold_decides_or_authorises_a_deployment` holds it). Approval is
refused if it would exceed a fleet ceiling and the response names the binding one.

## 3. Deploy with the forward block

```
POST /api/deployments
{ "strategy_ref": "<slug>", "instrument_id": "BTC-USD", "starting_capital": "10000",
  "proposal_id": "<approved proposal id>",
  "forward": { "trial_id": "<trial uuid>", "numeraire": "usd",
               "max_abs_return_divergence": 0.02, "max_cum_return_divergence": 0.05,
               "max_position_divergence": "0.5", "min_signal_match": 0.95,
               "kill_criteria": "<the condition under which you stop it, in words>" } }
```

The `forward` block is what makes the deployment *measurable*: it writes
`mlops.paper_deployment` and the runner attaches a `ForwardRecorder`. **Without it the
deployment is unmeasured**: the ramp pass flags it every hour and does not step it
down (ADR-T3-04), and Gate 15 has nothing to read. Thresholds are never edited
afterwards.

The first numbers above are starting points, not recommendations: `0.02`/`0.05` are
per-bar and cumulative return divergence as fractions, `0.95` the minimum fraction of
bars on which the shadow run and the live run took the same decision.

## 4. Confirm the clock started (day 1)

* `select count(*) from mlops.forward_observation where deployment_id = '<forward id>'`
  grows by one row per closed bar (24 a day at 1h).
* `GET /api/deployment-overview`: the row shows its profile, the
  period countdown and a realised-vs-expected series, and no `unmeasured` flag.
* `GET /api/reconciliation/halts` is empty; a halt here is the detector working.
* The ramp pass opened period 1 (`select * from ramp_period where deployment_id = ...`)
  with `ends_at` fourteen days out.

Record the deployment date: it is **the date the forward clock started**.

## 5. Known limitation: restarts

Fills are not yet persisted for resume (`ForwardRecorders::resume_deploy` exists and is
tested, but nothing journals `ResumeFill`; the fill-persistence migration is 0186 and its
runner call is Set U's). Until it lands, a platform restart opens a fresh recorder and
the cumulative divergence state resets; the gap reads as missing bars, which the
observation floor counts honestly. Avoid restarting the platform during the first review
period when you can.

## Arming the proposal requirement (irreversible: `BLOCKED:HUMAN`)

Until armed, `POST /api/deployments` accepts a deployment with no `proposal_id` (and the
hourly `legacy_marker_pass` marks it `legacy_ungated`). Arming makes the proposal the
only door, forever: the table refuses UPDATE and DELETE and `app_role` cannot insert, so
it is one-way by construction (ADR-T1-01). Do it once the deployments route in your UI
sends `proposal_id`, as the owner, from psql on the platform database:

```sql
INSERT INTO deployment_gate_switch (name, armed_by, note)
VALUES ('deployments_require_a_proposal', '<your name>',
        'every POST /api/deployments carries an approved proposal_id; legacy sweep has marked all running deployments');
```

Verify: `select * from deployment_gate_switch;` shows the row, and a
`POST /api/deployments` without `proposal_id` now answers 4xx from the trigger.

The second switch retires `legacy_unlogged` trials (T-1.5). **Do not arm it yet**: the
`POST /api/backtests` route still registers under that policy for the Back Testing tile
and for Set U's v3 backtest-over-HTTP path, so arming it now would break both (see the
T-1.5 note in `docs/plans/plan-sets/set-T/phase-1-deployment-path.md`). When the route is
gone:

```sql
INSERT INTO deployment_gate_switch (name, armed_by, note)
VALUES ('legacy_unlogged_retired', '<your name>',
        'POST /api/backtests is retired; no new trial registers under legacy_unlogged');
```

## The result (fill in; this is the T-0.4 acceptance record)

| | |
|---|---|
| Strategy / revision | |
| Instrument, bar size | |
| Declared thresholds and kill criteria | |
| Deployment date (forward clock start) | |
| First week: realised vs expected | |

**T-0.4 status:** engineering done; day-1 action `BLOCKED:HUMAN` (needs the platform
running, the monitor lease live, and a person to decide the proposal).

## Starting the monitor (T-2.3, the operations half)

Written 2026-10-04 by track T-P. Everything the code can do for T-2.3 is done; what is left
is one process running. Without it the order gate has no lease and **refuses every order**
(COMP-007 §4), so this comes before the deployment in step 3, not after.

The monitor needs only the platform's Postgres (it must not need the platform: it keeps
recording when the platform is wedged). Pick the way that matches how the platform runs:

* **Compose** (the default stack, not behind a profile):
  `docker compose up -d tbot-monitor`. The service is `restart: unless-stopped`, cadence
  `MONITOR_CADENCE_MS=60000`.
* **On the host** (when Docker is off, see the phantom-socket note): from the repo root,
  with the platform's own `DATABASE_URL` (the database the platform migrations ran on, **not**
  whatever answers on 5432):
  `DATABASE_URL=<platform db url> MONITOR_CADENCE_MS=60000 cargo run -p monitor --bin tbot-monitor -j 2`.
  Leave it running in its own terminal; it holds the lease and renews it every cadence.

Confirm: `GET /api/monitor/coverage` shows the lease as not expired and
`orders_permitted: true`, and each running deployment lists its limit families. Then stop the
monitor for one cadence and confirm the gate refuses with `monitor_unavailable`; that is
drill `monitor_lost` run by hand (the platform also runs it weekly against the real lease
row and files a `platform_defect` if it ever passes with the lease dead).
