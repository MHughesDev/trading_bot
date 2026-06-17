# Phase 3 — The Null Library

**Completion: 100% (9 / 9 tasks)** — the Null Library landed and unit-tested in
`crates/backtest/src/nulls/`: the `Null` contract, all 7 generators (each with a
property test on what it preserves/destroys), recommended-not-defaulted selection
with override logging, and the INV-3 `SignificanceResult` seam. Postgres-backed
null persistence is the live leg of J-3.8 (trait + in-memory reference +
`migrations/0029`).

**Summary of completed work (2026-06-17):**
- J-3.1 ADR-0020 authored and Accepted; indexed in `docs/adr/README.md`.
- J-3.2 `Null` contract + `NullGenerator` trait; non-empty `preserves`/`destroys`; content-addressed `null_id` (`nulls/mod.rs`, `nulls/generators.rs`).
- J-3.3 `signal_return_decouple` + `block_permutation` (marginal/autocorrelation-preserving property tests).
- J-3.4 `stationary_bootstrap` + `bar_permutation` (length + draw-from-source; OHLC-integrity property tests).
- J-3.5 `synthetic_garch` (pure-Rust GARCH(1,1) sim; deterministic per seed; vol-clustering signature).
- J-3.6 `regime_block` + `random_entry_matched` (within-regime / count + holding-period preservation).
- J-3.7 `recommend_null` (prompt, not default) + `NullChoice` (override requires a logged reason).
- J-3.8 `NullStore` trait + idempotent `InMemoryNullStore` + `migrations/0029_backtest_nulls.sql`.
- J-3.9 INV-3 `SignificanceResult { p_value, null_ref, trial_count_at_eval }` — no constructor omits any field (`nulls/significance.rs`).

`cargo test -p backtest` → 122 lib tests green; lib clippy clean.

**Goal:** Make the null a **first-class, selectable, parameterized, logged
object**. A permutation test's entire validity rests on the null being
appropriate to the question; the wrong null gives a confident, precise,
*meaningless* p-value. So the null is not a hidden default — it travels attached
to every significance result (INV-3), it states its hypothesis via
`preserves`/`destroys`, and it is **recommended but never defaulted**: the
recommendation is a prompt, the choice is logged, an override carries a logged
reason.

**Depends on:** Phase 0 (`DataSlice`, the bar read path), Phase 1
(`PermutationNull`/`SyntheticPaths` study kinds consume generators).
**Blocks:** Phase 4 Gate 3 (the primary significance test runs against a `Null`).

---

## Design notes

**The null is the hypothesis, rendered.** `preserves`/`destroys` are not
documentation — they are the explicit statement of what structure the null keeps
intact and what it breaks (the thing being tested), shown in every report. A null
that destroys the wrong thing is the most common silent error in this whole
domain.

**Contract (frozen shape, `crates/backtest/src/nulls/mod.rs`):**

```rust
pub struct Null {
    pub null_id: NullId,
    pub kind: NullKind,            // catalog below
    pub params: NullParams,        // e.g. block_length, n_resamples
    pub preserves: Vec<String>,    // what structure this null KEEPS intact
    pub destroys: Vec<String>,     // what it BREAKS (the thing being tested)
}

pub enum NullKind {
    SignalReturnDecouple, BlockPermutation, StationaryBootstrap,
    BarPermutation, SyntheticGarch, RegimeBlock, RandomEntryMatched,
}

pub trait NullGenerator {
    /// Produce ONE null-world dataset from the real bars + a seed.
    fn generate(&self, data: &BarSeries, seed: u64) -> BarSeries;
}
```

**Catalog (which null for which question):**

| Null kind | Preserves | Destroys | Use for |
|---|---|---|---|
| `signal_return_decouple` | marginal return dist, signal dist | the *pairing* signal→forward-return | "does the signal predict, or coincidence?" — general purpose |
| `block_permutation` | short-horizon autocorrelation (within block) | signal timing across blocks | intraday / mean-reversion where serial correlation matters |
| `stationary_bootstrap` | autocorrelation (random block lengths) | specific historical ordering | return-distribution robustness; daily trend |
| `bar_permutation` | bar-level OHLC integrity | inter-bar sequence | does *sequence* (not just bar shape) carry the edge? |
| `synthetic_garch` | volatility clustering, fat tails | the specific realized path | "would this work in markets that could have happened?" |
| `regime_block` | within-regime structure | cross-regime arrangement | suspected one-regime wonders |
| `random_entry_matched` | trade frequency, holding period, exposure | entry *timing* skill | "is the edge in timing, or just being in the market?" |

---

## Tasks

### ☑ J-3.1 Author ADR-0020 (Null Library & selection discipline) — S
Write `docs/adr/0020-null-library-and-selection-discipline.md`: the null is a
first-class object; `preserves`/`destroys` are mandatory and rendered; the null
is recommended-not-defaulted and the choice (and any override reason) is logged;
INV-3 requires the null travel with every significance result. Mark Accepted,
index in `docs/adr/README.md` + MASTER §9.
**Acceptance:** ADR-0020 exists, indexed, cited by Phase 3 + Gate 3.

### ☑ J-3.2 `Null` contract + `NullGenerator` trait — M
Add `crates/backtest/src/nulls/mod.rs` with `Null`, `NullKind`, `NullParams`,
and the `NullGenerator` trait. `preserves`/`destroys` are **non-empty** required
fields validated at construction (a null with no stated hypothesis does not
construct). `null_id` is a content hash of `kind + params`.
**Acceptance:** round-trip; a `Null` with empty `preserves` or `destroys` is
rejected; identical kind+params collide on `null_id`.

### ☑ J-3.3 `signal_return_decouple` + `block_permutation` — M
Implement the two most-used generators. `signal_return_decouple` shuffles the
pairing of signal→forward-return while preserving each marginal distribution.
`block_permutation` permutes fixed-length blocks (`params.block_length`),
preserving within-block autocorrelation and destroying cross-block timing.
**Acceptance:** property tests — `signal_return_decouple` preserves the marginal
return histogram (KS test passes) and destroys signal/return correlation;
`block_permutation` preserves lag-1 autocorrelation within `block_length`.

### ☑ J-3.4 `stationary_bootstrap` + `bar_permutation` — M
`stationary_bootstrap` resamples with geometric-random block lengths
(`params.mean_block`), preserving autocorrelation structure while destroying the
specific ordering. `bar_permutation` permutes whole OHLC bars, preserving
bar-level integrity (no OHLC violated) and destroying inter-bar sequence.
**Acceptance:** `bar_permutation` output never violates `low ≤ open,close ≤ high`
(test); `stationary_bootstrap` preserves the autocorrelation function within
tolerance over many resamples.

### ☑ J-3.5 `synthetic_garch` — M
Fit a GARCH(1,1)-t to the realized return series and simulate alternate paths
(seed-varying), preserving volatility clustering + fat tails while destroying the
specific realized path. Pure-Rust fit (method-of-moments / QMLE); no Python
sidecar (D-9). Feeds the `SyntheticPaths` study kind (Phase 1).
**Acceptance:** simulated paths reproduce the input's volatility-clustering
signature (ACF of squared returns) within tolerance; distinct seeds give distinct
paths; deterministic per seed.

### ☑ J-3.6 `regime_block` + `random_entry_matched` — M
`regime_block` permutes blocks across regime labels (preserving within-regime
structure, destroying cross-regime arrangement). `random_entry_matched` generates
random entries matched on trade frequency / holding period / exposure (preserving
activity profile, destroying entry-timing skill).
**Acceptance:** `random_entry_matched` output matches the source's trade count
and mean holding period within tolerance; `regime_block` keeps each block's
regime-internal stats intact (test).

### ☑ J-3.7 Recommended-not-defaulted selection + override log — M
Add `recommend_null(strategy_type) -> NullKind` (e.g. `BlockPermutation` for
intraday mean-reversion). The recommendation is surfaced as a **prompt** (API
returns it; the user must confirm or override). The chosen null and any override
reason are logged on the Experiment. There is **no invisible default** — a Study
of kind `PermutationNull` cannot run without an explicitly chosen `null_ref`.
**Acceptance:** the recommender returns the spec's mapping; running a permutation
study without an explicit choice is refused; an override stores its reason
(test).

### ☑ J-3.8 Null registry + persistence — S
Persist `Null` definitions via Postgres migration **0029** (`backtest_nulls`:
null_id, kind, params, preserves, destroys, created_by). Nulls are reusable
across Experiments and immutable once stored (a new params set is a new
`null_id`).
**Acceptance:** store→load round-trip; re-storing an identical null is
idempotent; `preserves`/`destroys` survive.

### ☑ J-3.9 INV-3 attach-to-result plumbing — S
Add the seam by which a significance result *must* carry its `null_ref` and the
`trial_count_at_eval` together: a `SignificanceResult { p_value, null_ref,
trial_count_at_eval }` type with **no constructor that omits any field**. Gate 3
(Phase 4) produces it; the workbench (Phase 5) renders all three or nothing.
**Acceptance:** `SignificanceResult` cannot be constructed without a `null_ref`
and a `trial_count_at_eval` (compile-fenced); a test asserts no "bare p-value"
path exists.

---

## Exit criteria

- All 7 null kinds generate null-world datasets; each declares non-empty
  `preserves`/`destroys` and is validated by a property test on what it keeps and
  breaks.
- The null is recommended-not-defaulted; the choice and any override reason are
  logged; a permutation study cannot run without an explicit null.
- `SignificanceResult` structurally cannot exist without its null and trial count
  (INV-3 seam). ADR-0020 Accepted. `cargo test -p backtest` green.
