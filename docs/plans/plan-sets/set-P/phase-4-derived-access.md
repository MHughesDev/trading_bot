# Phase 4 — Derived access and the source fence

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** DA-08, DA-10, DA-14.
**Spec:** [DATA-005 §8 (universes), §11 (sources), §12](../../../specs/DATA-005-data-api-v2.md);
ADR-0027.
**Depends on:** Phase 1 (bars to resample), Phase 2 (grades, so a derived series inherits one).
**Blocks:** nothing in this set.
**Migration range:** 0104.

Two unrelated things, both small, both belonging at the end: making stored bars reachable at
whatever resolution a study needs, and making the free-only policy a mechanism instead of a
habit.

---

### P-4.1 Server-side resampling — M
`NOT STARTED`

Any stored timeframe resampled to any coarser timeframe at read time, computed once in the
data layer rather than by every caller. The resampled series inherits the **worst** grade and
the **largest** declared lag of its inputs — a derived series is never cleaner than what it was
derived from, and the arithmetic must be that way round or resampling becomes a laundering step.

Resampling is bar-convention-sensitive. The canonical timestamp rule (DATA-005 §3) applies, and
a test asserts a 1m→1h resample agrees bar-for-bar with the venue's own 1h series where both
exist.

**Files:** `crates/dataplane/src/resample.rs`, `crates/api/src/routes/data.rs`.
**Acceptance:** 1m→1h resampling matches the venue's own 1h series on an overlapping window; the
derived series carries the worst input grade and largest input lag; resampling to a *finer*
timeframe is refused rather than interpolated.
**Traces:** DA-10; DATA-005 §3, §7.

### P-4.2 Information-driven bars — S
`NOT STARTED`

Volume, dollar and tick bars where the inputs exist. These are sampled on information rather
than on the clock, which is the point — clock bars oversample quiet periods and undersample
the ones that matter.

Each generator declares its threshold and records it with the series, so a bar set is
reproducible. They are read-time derivations like P-4.1, not a second stored bar table.

**Files:** `crates/dataplane/src/bars/information_driven.rs`.
**Acceptance:** volume, dollar and tick bars are produced from stored trades or bars with the
threshold recorded; regenerating with the same threshold is byte-identical; a request for a
generator whose inputs are absent returns a stated reason rather than an empty series.
**Traces:** DA-10; 06_DATA §2.

### P-4.3 As-of universes — M
`NOT STARTED`

`universes` and `universe_membership` (migration 0104: name, instrument_id, valid_from,
valid_to, reason) and `GET /api/data/universe/{name}?as_of=` returning membership valid at
`as_of`. Universe-based Experiments declare a universe and are evaluated on as-of membership.

**Files:** `crates/dataplane/src/universe.rs`, `crates/api/src/routes/data.rs`, migration 0104.
**Acceptance:** membership as of a past date excludes instruments added later and includes ones
since removed; an Experiment declaring a universe is evaluated on as-of membership, proven by a
test where the two answers differ; `valid_to` is never overwritten — a removal is a new row.
**Traces:** DA-08; DATA-005 §8.

### P-4.4 Delisted instruments are retained, not deleted — M
`NOT STARTED`

Dead and delisted instruments stay in `instruments` with their full history and
`status='delisted'`. This is the only defence against survivorship bias that actually works,
and it is a deletion policy, not an analysis technique: once the row is gone the bias is
unrecoverable.

Pairs with P-2.5, whose survivorship check reads exactly this history.

**Files:** `crates/storage/src/postgres/instruments.rs`, migration 0104.
**Acceptance:** delisting sets `status` and retains every bar; a query for a historical
universe returns instruments now delisted; a test asserts no code path deletes an instrument
row, and the static invariant suite fails the build if one is added.
**Traces:** DATA-005 §8; DA-08; P-2.5.

### P-4.5 The free-only fence, as a CI check — M
`NOT STARTED`

DA-14 says no paid source may be added without revisiting ADR-0027. Today that is a convention.
This makes it a mechanism: every source is declared in `dataplane.source` with a `cost_class`
that has **no default**, and an `xtask` check fails the build when a source declares anything
but `free` unless an allowlist entry names the ADR that approved it.

The point is not to forbid paid data — it is to make adding it a decision someone signs rather
than a dependency that appears.

**Files:** `xtask/src/check_free_sources.rs`, `crates/dataplane/src/source.rs`, migration 0104,
CI workflow.
**Acceptance:** adding a source with `cost_class = 'paid'` fails CI; adding one with an
allowlist entry naming an ADR passes; a source with no `cost_class` fails to compile or fails
the migration's CHECK, not the review; the check runs in the same CI job set as the other
`xtask` guards.
**Traces:** DA-14; ADR-0027; D-P1.

---

## Phase exit

- Any study can get the resolution it needs without a caller reimplementing resampling, and a
  derived series never looks cleaner than its inputs.
- Universe membership is point-in-time and delisted instruments are retained forever.
- A paid source cannot enter the platform without an ADR, enforced by CI.
