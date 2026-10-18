# The capital gate

**Status:** in force from 2026-10-03 (Set T T-3.4). **Owner:** the named human in §3.
**Applies to:** any deployment that trades real money, on any venue, in any amount.

This is a document and not a code path, on purpose. No task in Set T moves money,
and ring 3 has no tool and never will (AGENT-013 §4, D-12, ADR-0025). What the code
does is the opposite of what this page does: it **refuses**. Every deployment this
platform can authorise today is paper, on the platform's own engine (ADR-0048), and
`crates/api/tests/no_capital_path.rs` fails if that stops being true. This page
says what must be true, in writing, before anyone changes that on purpose.

## 1. What closes today

| Door | Why it is shut | Where it is held |
|---|---|---|
| A capital venue class | `ledger::deployment_path::VenueClass` has one variant, `Paper` | the type; `no_capital_path.rs` holds an exhaustive `match` on it |
| A capital deployment row | `CHECK (venue IN ('platform_paper'))`, `CHECK (environment IN ('paper'))` | migration 0142 |
| A capital proposal | `CHECK (venue_class = 'paper')` | migration 0180 |
| A capital authorisation | `CHECK (venue_class = 'paper')` | migration 0180 |
| Capital from a `paper_v1` pass | `capital::authorise` refuses a profile with `authorises_capital = FALSE`; the ladder's raise trigger refuses it too | `ledger::capital`; migration 0053 |
| A capital rung without a reviewed period | a raise must cite exactly one passed `step_up` review | migration 0181 |
| A broker on the paper path | paper runs in the app's own engine, never Alpaca paper (ADR-0048) | `crates/execution` |

To open any of these, someone edits the type, the migration and the test in one
reviewed change. A configuration value, an environment variable or a feature flag
is not a way to do it, and the test enumerating the entry points is written so that
it will say so.

## 2. The checklist

Nothing below is a build task. All of it is evidence that has to exist first, and
a person confirms each line in writing, dated, in the change that opens the door.

1. **H-4 is answered in writing.** The roadmap's gate H-4 (capital and liability):
   - **whose broker** — the account holder and the broker, by name;
   - **whose money** — the funds, who owns them, and how much may ever be exposed;
   - **who is liable** — for a loss, for a regulatory question, and for an outage.
   Q-4 in the open-questions list is closed with these three answers, not with a
   decision to proceed.
2. **A `strict_v1` pass, with its full verdict.** All sixteen gates, each with its
   statistic, threshold and evidence, for the exact revision that would trade. Not
   `paper_v1`: that profile cannot authorise capital and its pass is forward-test
   evidence only (ADR-P2-14, AT-65). Gate 16's own verdict is recorded from a
   completed review period (migration 0181), not from a profile flag.
3. **The fleet's divergence record.** The paper fleet's reconciliation history
   (T-2.2) over at least the stated observation window: every divergence halt, who
   cleared it and why, and the fleet-level finding on whether the simulator is
   systematically wrong. A chronic halt is a finding about the simulator and
   blocks this step until it is understood.
4. **The drill log.** The monitor's fail-closed drills (T-2.4) run on schedule and
   all passed in the period; the kill-switch drill stopped order flow within one
   cadence with the monitor as an independent process (COMP-007 §8). A drill that
   has not run recently counts as failed.
5. **A forward record that exists.** Gate 15 is a real verdict from observed rows
   only (D-T1), not a `NotComputable` and not a backfill. A `legacy_ungated`
   deployment's record counts from its marker date and carries the marker beside
   its numbers.
6. **The ramp starts at 10%.** The first capital authorisation is the 10% rung and
   climbs one rung per **passed, completed** review period under Gate 16
   (10 / 25 / 50 / 100). It steps down 50% per period to zero on a failing or
   unmeasurable period (ADR-P2-27). Nothing starts above 10%, nothing skips a rung,
   and nothing sits at its size on the strength of never having been measured.
7. **The fleet ceilings are declared.** `max_concurrent`, `max_aggregate_capital`
   and `max_instrument_exposure` (migration 0182), with the per-instrument
   aggregate computed across instances of a revision. An undeclared ceiling refuses
   every deployment.
8. **The named human decides.** One person, named in §3, makes the decision in a
   web session. An agent can prepare the proposal; it can never decide, and its
   token cannot hold the scope that decides (`auth::scopes::NEVER_FOR_AGENTS`).

## 3. Who decides

**Decider:** _not yet named._ This line is filled in by the account holder when H-4
is answered, in the same change as the checklist confirmation. Until it names a
person, there is no capital deployment to decide and the code will not make one.

## 4. How the door is opened, once the checklist holds

In one reviewed change, and in this order:

1. Record the checklist confirmation and the decider in §3 of this page.
2. Add the capital venue class to `VenueClass`, and a migration that widens the
   `CHECK` constraints in §1 (a new migration; never an edit to 0142 or 0180).
3. Update `crates/api/tests/no_capital_path.rs` — it will not compile until the
   new variant is handled, which is the point.
4. Give the capital path **its own** deployment entry point with its own authority
   check. Do not widen the paper route.
5. Start the ramp at the 10% rung, never the paper floor's successor by default.

If any step is missing, the change is not reviewable. If a step is skipped, the
tests fail.
