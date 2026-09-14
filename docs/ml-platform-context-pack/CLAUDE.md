# Operating instructions for the coding agent

You are modifying an existing codebase to match `spec/SPEC.md`. Read this file completely before the first edit.

---

## 1. Before you touch anything

1. Read `decisions/INVARIANTS.md` in full. It is short and every line is load-bearing.
2. Read `spec/SPEC.md` §0 (the four irreversible decisions). If the existing codebase already violates one of these, **that is the first thing to fix** and it is probably a migration, not an edit.
3. Survey the existing code and write `decisions/CURRENT-STATE.md` — what exists, what partially exists, what is absent, and which invariants are currently violated. Do this before proposing any change.
4. Do **not** start at Phase 5 of the checklist because it is more visible. The ordering in `backlog/IMPLEMENTATION-CHECKLIST.md` is by *retrofittability*, not by value. Phase 0 items are ones where every day of delay destroys information that cannot be recovered.

## 2. How to treat the spec

**The spec is normative. The research in `reference/` is evidentiary.**

- Implement what `SPEC.md` says. Where it gives a threshold, a default, or an ordering, those are chosen values, not illustrations.
- Where the spec says **REQUIRED**, **no default**, **never**, or **enforced by**, implement it as a hard constraint in code — a schema constraint, a permission grant, a CI test, or a type — not as documentation or a code comment. The spec deliberately converts discipline into mechanism. Preserve that.
- Where the spec is silent, consult `reference/` and then record your choice in `decisions/ADR-INDEX.md` with the reasoning.
- If you believe the spec is wrong, **stop and write it into `decisions/OPEN-QUESTIONS.md`.** Do not implement your correction and do not implement the spec version you disagree with. Sixteen spec decisions already reverse an earlier, more obvious position — the spec is likely to have seen the argument you are about to make.

## 3. Things you must not do

These are the ways this system fails quietly. Each is spelled out in `INVARIANTS.md`; they are repeated here because they look reasonable in a diff.

- **Do not add a non-PIT read path**, not even "for speed" or "for development." §1.3 makes the PIT path fast enough that there is no excuse. A dev-only fast path becomes the production path.
- **Do not add a way to run a trial without a ledger row.** No `--no-log`, no `skip_registry`, no admin bypass, no test fixture that writes directly to the executor. If a side door exists, an audit will find it was used.
- **Do not store adjusted prices.** Store unadjusted + factors, compose at read time.
- **Do not store greeks.** Store implied volatility with its model, rate and dividend inputs.
- **Do not let a strategy read smoothed regime probabilities.** Enforce with `GRANT`, not review.
- **Do not scalarize a multi-objective selection** with a hidden default weighting. The outcome type has no `score` column on purpose.
- **Do not lower the exploration floor**, expose it to the agent, or "optimize it away" because it looks like waste.
- **Do not expose raw point estimates** from ledger-derived recommendations. LCB-ranked and shrunk, always.
- **Do not edit a gate profile in place.** Create a new version.
- **Do not put per-instrument results in metric series.** Parquet artifacts.
- **Do not use `asof` joins with `strategy='nearest'`.** Wrap the library so it is impossible.
- **Do not put `tenant_id` as a one-hot feature in a global model.** Use hierarchical partial pooling.

## 4. Conventions

- **Timestamps:** UTC always, `TIMESTAMP(9)`, bar timestamps are the bar **open**.
- **Prices:** `DECIMAL(38,18)`, never floating point. Crypto spans 10⁻⁸ to 10⁵ and doubles will not round-trip.
- **Identity:** `instrument_id` surrogate keys. `symbol` is a bitemporal attribute, never a key.
- **Hashing:** blake3 over canonical JSON. Same function everywhere.
- **IDs in agent-facing APIs:** human-readable slugs, not UUIDs.
- **Every table that records a fact about the world carries `knowledge_time`.**

## 5. Definition of done, per work item

A checklist item is done when all five hold:

1. The behavior matches the spec section it cites.
2. The corresponding entry in `backlog/ACCEPTANCE-TESTS.md` exists and passes.
3. Any spec constraint marked **enforced by** is enforced by the stated mechanism — schema, grant, type, or CI — and there is a test proving the *violation* is rejected, not only that the happy path works.
4. No invariant regressed. Run the invariant test suite.
5. If you made a judgment call, it is recorded in `decisions/ADR-INDEX.md`.

## 6. Migration guidance

Phase 0 items are frequently migrations of existing data. Rules:

- **`knowledge_time` cannot be reconstructed for historical rows.** Do not fabricate it. Backfill with a sentinel (`knowledge_time = event_time + declared_vendor_lag`), mark those rows `quality_flags |= BACKFILLED_KNOWLEDGE_TIME`, and make the sentinel visible in every dataset spec that touches them. An honest hole is worth more than a fabricated column. Datasets spanning the sentinel boundary are flagged, not blocked.
- **Existing adjusted-price history cannot be un-adjusted.** Re-ingest from source where possible; where it is not, quarantine the adjusted series under a distinct `source_id` flagged `NON_REPRODUCIBLE` and let dataset specs opt in explicitly.
- **Existing trial history probably has no propensities.** Do not impute them. Mark those rows `policy_id = 'legacy_unlogged'`, `propensity = NULL`, and exclude them from every off-policy estimator by construction. They still count toward N_eff — trial accounting is about how many times you looked, and a legacy look still happened.
- **Set Iceberg retention before the first migration writes**, not after.

## 7. When you are unsure

Order of resort:

1. `spec/SPEC.md` — search it; it is more specific than it looks
2. `reference/` — the research file named in `decisions/SOURCE-MAP` for that topic
3. `decisions/OPEN-QUESTIONS.md` — write the question, with the options you see and what each would cost, then continue on other work

Do not guess on anything in `INVARIANTS.md`. Do not guess on anything involving money, gates, holdouts, or tenant boundaries.

## 8. Reporting

When you finish a phase, write a short entry in `decisions/PROGRESS.md`:
- what was implemented, with spec section references
- which acceptance tests now pass
- which invariants are now enforced by mechanism, and by which mechanism
- what remains violated and why
- any new entries in `OPEN-QUESTIONS.md`

Keep it factual. Do not describe a partially-enforced invariant as enforced.
