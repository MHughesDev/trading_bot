# ADR-0050: Hot-Path JSON Is Marked at the Boundary, Not Guessed From Source Text

**Status:** Rejected — superseded before adoption by Set C ADR-C-LINT-1
**Note (merge of claude/cranky-hofstadter-b055d3):** written as ADR-0039 on a branch;
main's 0039 is a different decision, so it is filed here as 0050. Main fixed the same
broken lint differently — refuse JSON *encoding* in a hot-path crate, allow boundary
*decoding*, no call-site marker (`docs/plans/plan-sets/set-C/MASTER.md`, ADR-C-LINT-1).
The marker design below is kept as the record of the alternative: it is stricter (it
would also catch a hot-path decode of the platform's own envelopes) and costs a marker
at every venue-boundary parse.
**Date:** 2026-09-22
**Deciders:** Platform team
**Amends:** ADR-0003 (the rkyv hot path) — records how its JSON ban is actually enforced
**Traces:** Set C issue-002 acceptance criterion "`cargo xtask lint-no-json-hotpath` passes
and runs in CI"; EXECUTION-BOARD §2.6; Set S phase-0 ground rules

---

## Context

ADR-0003 moved the market lane in-process and gave it rkyv-encoded envelopes. The rule
that followed — *no `serde_json` on the market-data hot path* — was given a guard,
`cargo xtask lint-no-json-hotpath`, and the guard was never able to hold it.

The guard matched substrings of **line text**. Two consequences, both fatal:

**It flagged things that encode nothing.** `Option<serde_json::Number>` in a venue DTO,
`&serde_json::value::RawValue` in a signature, `use serde_json::value::RawValue;` — these
describe the shape of bytes a venue sent us. Eleven of the fifteen standing violations
were of this kind. `RawValue` in particular exists in `backfill/` *because* it does not
decode: reading a venue's numbers through `serde_json::Value` would round-trip them
through `f64`, which is the money-precision failure `check-money-f64` exists to prevent.
The guard was calling the fix a violation.

**Its escape hatch could not be used.** The documented exception for "parsing raw bytes
from external APIs" was `line.contains("&raw") || "&text" || "&body" || "&buf"`. For a
`&str` body the only spelling that satisfied it was:

```rust
serde_json::from_str::<T>(&body[..])
```

which is `clippy::redundant_slicing`, which `cargo clippy --workspace --all-targets --
-D warnings` — a CI job — rejects. The lint gate and the clippy gate were **mutually
unsatisfiable**. No code could pass both.

So the check sat red. Being red, it was never wired into CI. Being unwired, it was never
fixed, and legitimate boundary parses accumulated under it for months, each one adding a
line to a failure report nobody could act on. Set S phase 0 recorded the situation
correctly and left it, reasonably, for whoever owned "the line" — of which there was no
such thing.

The deeper problem is that the rule is not decidable from source text. `serde_json::from_str`
against a Kraken WebSocket frame is correct and necessary; `serde_json::from_str` against an
`EventEnvelope` payload is the thing ADR-0003 forbids. The two calls are spelled
identically. Only the author knows which one they wrote.

## Decision

**A `serde_json` codec call in a hot-path crate is a violation unless the author marks it
at the call site.**

The marker is a comment, trailing on the line or alone on the line directly above it:

```rust
// json-at-boundary: Kraken's OHLC REST response, not one of our envelopes
let env: OhlcEnvelope = serde_json::from_slice(body)
    .map_err(|e| FetchError::Integrity(format!("kraken ohlc: {e}")))?;
```

A standalone marker covers the **statement** below it, not the line below it, capped at
eight code lines. That distinction is load-bearing rather than lenient: `cargo fmt`
breaks a long `let` after the `=`, which moves the call one line further down than the
author left it. The workspace-wide `cargo fmt --all` of 2026-09-22 (`2ca5ba3`) did
exactly that to both `serde_json::from_str` calls in `derivs/funding.rs`, and under a
next-line rule each became two violations — a stale marker and an unmarked call — from a
commit that changed no code. A check whose answer depends on whether the formatter has
run is a check nobody can keep green. The eight-line cap is what stops "the statement
below" decaying into "the rest of the function".

Three further properties make it a mechanism rather than a comment:

1. **The reason is mandatory.** `// json-at-boundary:` with nothing after the colon is
   itself a violation. An exemption nobody had to justify is an exemption nobody reviews.
2. **A marker covering nothing is a violation.** When the call a marker exempted is
   deleted or moved, the marker is reported rather than silently widening to whatever
   lands under it next. This is the rot the old type-name allowlist died of.
3. **Codec items may not be imported.** `use serde_json::{json, Value};` is refused, so
   every call keeps its `serde_json::` prefix and stays visible to the check — and to a
   reader.

**What the check flags** is narrowed to what the rule is about: calls into serde_json's
codec API (`to_vec`, `to_string`, `to_writer`, `from_slice`, `from_str`, `from_reader`,
`from_value`, `to_value`, `json!`) and the dynamic `serde_json::Value`/`Map` document.
Path segments are matched whole, so `serde_json::value::RawValue` is not
`serde_json::Value`.

**What it no longer flags:** `RawValue`, `Number` and `Error` in any position, and imports
of them. Those name someone else's bytes.

**Both halves run in CI.** `.github/workflows/ci.yml` gains a `lint-no-json-hotpath` job
that runs the check and `cargo test -p xtask`. The second step matters: `cargo test
--workspace --lib` does not reach a binary crate, so the classifiers behind this check —
and behind `check-bars-v1-frozen` and `check-bars-v2-frozen` — had no run in CI at all.

## Rationale

A check whose exemption mechanism cannot be satisfied is worse than no check. It produces
a standing red that teaches everyone to ignore it, and it produces comments like the ones
Set P phase 3's collectors carried — *"the documented exception in `cargo xtask
lint-no-json-hotpath`"* — pointing at an exception that does not work.

Between "guess from the text" and "ask the author", only the second can decide the
question the rule actually asks. The cost is one comment per venue-boundary call: 28 of
them across the collectors today. That cost is the point. Each one names a venue and a
response, which is exactly the thing a reviewer needs in order to tell a legal decode from
an illegal one, and which no amount of pattern matching could have recovered.

## Consequences

- 28 call sites in `crates/collectors` now carry a `json-at-boundary:` marker naming the
  venue payload they decode. Three statements in `backfill/` were reflowed so the call
  sits on the first line of its statement and the marker has something to sit above.
- `crates/collectors/src/derivs/funding.rs`'s nine `serde_json::Number` fields and
  `backfill/`'s `RawValue` signatures are no longer violations and carry no marker.
- A new `serde_json` call on the market lane now fails CI on the PR that adds it, rather
  than joining a backlog.
- Set P phase 3's five new collectors (`macro/fred.rs`, `fundamentals/edgar.rs`,
  `news/alpaca.rs`, `corp_actions/alpaca.rs`, `prediction/history.rs`) landed on `main` in
  `3c352af` while this was being written. They were authored against the broken escape
  hatch and each carried a comment saying so — `fred.rs`'s spelled out the whole
  contradiction and concluded the code was "written to be correct rather than to satisfy
  the grep", which was the right call. Their six call sites now carry markers and those
  comments are gone. That is the check's first real test: it named all six by file and
  line the moment `main` merged, which is what it is for.
- The marker is formatter-stable. That was not true of the first version and was found
  the same week, by merging `2ca5ba3` rather than by reasoning about it — which is the
  argument for landing a guard and then running the workspace through it, instead of
  declaring it correct.
- The exemption list is now distributed across the call sites instead of centralised in
  `xtask`. A reviewer reads the reason next to the code, and `git grep json-at-boundary:`
  enumerates every exemption in the workspace.

## Alternatives Considered

**Retire the check and record that ADR-0003's JSON ban is unenforced.** The ban is real —
JSON on the tick path is #2 on the latency table — and the market lane is exactly where a
convenience `serde_json::to_vec` gets added under time pressure. An unenforced rule in a
document is how the rule was lost the first time.

**A per-file `#![allow]`-style marker.** Cheaper: six files would have carried one line
each instead of 28 markers. Rejected because almost every collector file is a venue
boundary, so file-level opt-out would have exempted essentially all of `crates/collectors`
— including a future `envelope.payload = serde_json::to_vec(&trade)` added inside an
already-exempt file, which is precisely the regression the check exists to catch.

**Keep substring matching and add the missing spellings.** This is what produced the
allowlist of nine venue type names (`AlpacaMessage`, `ChartResponse`, `RedditListing`, …)
already in the check. Each new collector adds a name; none of them is checkable; and the
list says nothing about whether the call was legal.

**Silence clippy so `&body[..]` works again.** An `#[allow(clippy::redundant_slicing)]`
per call site, to satisfy a check by writing code a different check calls wrong. Two
guards disagreeing is a design fault, not something to annotate past.

## References

- ADR-0003 — NATS JetStream as the tail; the in-process rkyv hot path
- `xtask/src/main.rs` — `lint_no_json_hotpath`, `scan_json_hotpath_file`, and their tests
- `.github/workflows/ci.yml` — the `lint-no-json-hotpath` job
- `docs/architecture/latency-table.md` §2 — JSON on every hot path
- `docs/plans/plan-sets/set-C/issue-002-DESIGN.md` §7 — the acceptance criterion this closes
