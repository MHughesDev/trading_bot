# Issue #002 — Binary Envelope with rkyv: Design Specification

**Status:** DESIGN — ready for review
**Author:** Set-C latency program
**Date:** 2026-06-11
**Supersedes:** implementation plan in `issue-002.md`
**Related:** ADR-0003 (SPSC ring pipeline), `issue-001.md` (JetStream tee)

---

## 1. Current State (What Is Already Done)

A significant portion of issue #002 has already landed. This spec is grounded
in the code as of branch `claude/keen-planck-jmfw95`:

| Step (from issue-002.md) | Status | Evidence |
|---|---|---|
| rkyv in workspace | ✅ DONE | `Cargo.toml:72` — `rkyv = { version = "0.8", features = ["std"] }` |
| `unsafe_code` relaxed to `deny` | ✅ DONE | `Cargo.toml:126` (per-use `#[allow(unsafe_code)]` opt-in) |
| Interned ID newtypes | ✅ DONE | `domain/src/instrument.rs` — `InstrumentId(u32)`, `VenueId(u32)`, `SourceId(u32)`, all rkyv-derived |
| Intern table | ⚠️ PARTIAL | `domain/src/interned.rs` — self-seeding, **process-local only** (gap G3 below) |
| Compact envelope | ✅ DONE | `domain/src/envelope.rs` — non-generic, 6 fields, `size_of ≤ 96` const-asserted, no `event_type`/`schema_version`/`lane` |
| Lane carried by NATS subject | ✅ DONE | `event-bus/src/lanes.rs::subject_for(lane, instrument)` |
| rkyv on payload types | ⚠️ PARTIAL | `TradePayload`, `BarPayload` only — 7 types remain (gap G1) |
| Decimal-over-rkyv | ✅ DONE | `money::AsDecimalBytes` wrapper (`#[rkyv(with = AsDecimalBytes)]`) |
| Binary publish path | ✅ DONE | `event-bus/src/publish.rs` — `rkyv::to_bytes`, serde_json banned by comment |
| JSON quarantined | ⚠️ PARTIAL | quarantine lane is JSON (intended); 5 collectors still JSON-encode payloads (gap G2) |
| CI lint for serde_json | ❌ NOT DONE | no check in `.github/workflows/ci.yml` or xtask (gap G5) |

## 2. Remaining Gaps

- **G1 — Payload coverage.** `quote.rs`, `orderbook.rs`, `funding_rate.rs`,
  `dex_quote.rs`, `prediction_price.rs`, `social_post.rs`,
  `web_page_snapshot.rs` have no rkyv derives.
- **G2 — Collector encode paths.** Tradier (`options/tradier.rs:129`), Kalshi
  (`prediction/kalshi.rs:111`), 0x (`dex/zerox.rs:113`), Reddit
  (`social/reddit.rs:133`), and the web scraper (`web/scraper.rs:527`) write
  `serde_json::to_vec(&payload)` into `EventEnvelope.payload`. Their consumers
  symmetrically `serde_json::from_slice`.
- **G3 — Cross-process ID consistency.** Interned u32 IDs travel inside NATS
  payloads, but the intern table is self-seeding per process. Two processes
  that intern names in different orders assign **different IDs to the same
  instrument** — a silent data-corruption hazard. The module docs acknowledge
  this ("cross-process consistency is achieved by reading the same config /
  Postgres rows at startup") but nothing enforces it.
- **G4 — Unchecked access on network bytes.**
  `EventEnvelope::decode_payload()` uses `rkyv::access_unchecked` with a
  SAFETY comment claiming bytes "were produced by rkyv::to_bytes **in this
  process**". That holds for the in-process SPSC ring, but the same method is
  called on bytes received from JetStream — a cross-process, network boundary.
  Corrupt or malicious bytes are undefined behavior.
- **G5 — No CI enforcement.** Nothing prevents serde_json from creeping back
  into hot paths.
- **G6 — `schema_version: String` inside payloads.** `TradePayload` (and
  others) carry a heap-allocated `schema_version: String` per event — exactly
  the per-event String the issue set out to remove. It is derivable from the
  type (`Payload::schema_version()` is already a `&'static str`).

## 3. Design

### 3.1 Wire format (unchanged, formalized here)

```
NATS subject:   <lane>.<instrument_name>          e.g. market.trades.BTC-USD
NATS payload:   rkyv bytes of EventEnvelope:
                ┌──────────────────────────────────────────────┐
                │ instrument_id: u32   (interned)               │
                │ venue_id:      u32   (interned)               │
                │ source_id:     u32   (interned)               │
                │ sequence:      u64   (per-source monotonic)   │
                │ timestamp_ns:  i64   (exchange event time)    │
                │ payload:       [u8]  (rkyv bytes of payload)  │
                └──────────────────────────────────────────────┘
```

- `event_type` / payload schema is derived from the **lane** (subject), which
  maps 1:1 to a payload type via `lane_payload_type()` (new, §3.5).
- `schema_version` is derived from the payload type's `Payload::schema_version()`
  associated constant. Version bumps create a **new lane suffix**
  (`market.trades.v2.>`), never an in-place field change — same policy the
  strategy-definition format already follows.
- Header target stays ≤ 96 bytes (`const _: () = assert!(...)` already in place).

### 3.2 G1+G6 — Payload types: rkyv everywhere, no per-event version String

For each of the 7 remaining payload types:

1. Add `rkyv::Archive, rkyv::Serialize, rkyv::Deserialize` derives, using
   `#[rkyv(with = AsDecimalBytes)]` on every `Price`/`Size`/`Decimal` field
   (pattern proven by `TradePayload`).
2. **Remove `schema_version: String` from every payload struct.** The value is
   already available statically via the `Payload` trait. JSON consumers at the
   REST boundary that need it in the body get it from a serde
   `#[serde(serialize_with)]` shim or the API layer injecting it — not from a
   per-event heap String.

String-bearing payloads get a tiering policy:

| Tier | Lanes | Payloads | Encoding |
|---|---|---|---|
| Hot | `market.trades`, `market.quotes`, `market.bars.*`, `market.orderbook.l2`, `market.funding_rate`, `dex.quote`, `prediction.price` | Trade, Quote, Bar, Orderbook, FundingRate, DexQuote, PredictionPrice | rkyv **required** |
| Cold | `social.post`, `web.page_snapshot`, `news.article` | SocialPost, WebPageSnapshot | rkyv **preferred**; large `String` bodies are fine (rkyv stores them inline, still one decode pass, no JSON parse) |
| Boundary | `quarantine`, REST request/response, Parquet raw archive | n/a | serde_json **allowed** |

Orderbook note: `OrderbookPayload` levels become fixed-point arrays under
`AsDecimalBytes`; depth is bounded (top-N), so archived size is predictable.

### 3.3 G3 — Intern table: deterministic epoch + fail-fast handshake

The core problem: u32 IDs cross process boundaries, so all processes must
agree on the name→ID mapping. Three options were considered:

- **(a) Central allocator (Postgres sequence / NATS KV):** every intern is a
  network round-trip or a cache with invalidation — complexity and a runtime
  dependency in the hot path's startup. Rejected.
- **(b) Carry names on the wire (no shared table):** defeats the point of
  interning (per-event strings return). Rejected.
- **(c) Deterministic seed + epoch hash (CHOSEN):** all processes derive the
  same table from the same input, and prove it to each other cheaply.

Mechanism:

1. **Seed source.** At startup, every process loads the instrument/venue/source
   registry from the same place (config file today; the `instruments` Postgres
   table when it lands), sorts entries lexicographically, and interns them in
   that order. Deterministic input + deterministic order ⇒ identical IDs.
2. **Epoch hash.** After seeding, compute `table_epoch: u64 = xxh3(sorted
   names joined with '\0')` per table kind, folded into one u64. Expose it via
   `interned::epoch()`.
3. **Handshake.** Producers stamp the epoch into a NATS **header**
   (`X-Intern-Epoch: <u64 hex>`) — headers are outside the rkyv payload, so
   the envelope stays ≤ 96 bytes. Consumers check the header against their own
   epoch **once per (subject, epoch) pair** (cached), not per message. On
   mismatch: route the message to quarantine and log loudly. Fail-fast, never
   misattribute data.
4. **Late additions.** `intern_*` calls for names outside the seed remain legal
   but the returned ID is **process-local**: publishing an envelope whose IDs
   exceed the seeded range is a bug, enforced by a debug assertion in
   `Publisher::publish` (`id < seeded_len`). New instruments enter via registry
   update + rolling restart (instruments are already a startup-time concern).

This adds zero per-event cost: one header comparison amortized to once per
consumer per epoch.

### 3.4 G4 — Checked access at trust boundaries

Two distinct byte provenances require two APIs:

```rust
impl EventEnvelope {
    /// Zero-copy view; bytes MUST originate from this process (SPSC ring path).
    /// unsafe is contained here; callers in the tick hot path use this.
    pub fn access_payload_trusted<T: rkyv::Archive>(&self) -> &T::Archived;

    /// Validated zero-copy view for bytes that crossed a process/network
    /// boundary (JetStream consumers, replay). Uses `rkyv::access` with
    /// bytecheck — rejects corrupt/malicious bytes with an error instead of UB.
    pub fn access_payload<T>(&self) -> Result<&T::Archived, rkyv::rancor::Error>
    where T: rkyv::Archive, T::Archived: for<'a> rkyv::bytecheck::CheckBytes<...>;

    /// Owned deserialization (existing decode_payload), reimplemented over
    /// access_payload (checked) — used off the hot path.
    pub fn decode_payload<T>(&self) -> Result<T, rkyv::rancor::Error>;
}
```

- Workspace `rkyv` gains the `bytecheck` feature (default in 0.8; confirm it
  is not disabled).
- Rule of thumb, enforced in review + lint comment: **ring ⇒ trusted, NATS ⇒
  checked.** The strategy tick path receives envelopes from the in-process
  ring (issue #1's architecture), so the p99 tick-to-intent budget never pays
  validation; only JetStream tee consumers (storage writers, UI gateway,
  replay) pay the bytecheck walk, and they are not latency-critical.
- Benchmark gate: bytecheck validation of a `TradePayload` envelope must stay
  < 200 ns (it is a linear walk of a ~100-byte buffer; expected ~tens of ns).

### 3.5 G2 — Collector and consumer migration

Per collector (tradier, kalshi, zerox, reddit, scraper):

```rust
// before
let payload_bytes = serde_json::to_vec(&payload)?;
// after
let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)?.into_vec();
```

Symmetric change in each consumer/test (`serde_json::from_slice::<P>(&env.payload)`
→ `env.access_payload::<P>()` or `decode_payload`).

REST/WS **ingestion** parsing (`serde_json::from_slice::<QuotesResponse>` on
exchange API responses) is a boundary and stays JSON — only the
**envelope payload encoding** changes.

Add `lane_payload_type(lane: &str) -> PayloadKind` (enum) in `domain::lanes` so
generic consumers (storage writer, UI gateway) can dispatch decode without a
type table of their own.

### 3.6 G5 — CI enforcement

Add an xtask lint (runs in the existing Clippy CI job, no new workflow):

```
cargo xtask lint-no-json-hotpath
```

Implementation: deny-list grep for `serde_json::` in:
- `crates/event-bus/src/` — except `quarantine.rs`
- `crates/collectors/src/` — except lines/regions tagged `// json-boundary:`
  (REST/WS ingestion parse sites) — the tag makes every exception auditable
- `crates/strategy-runtime/src/` — no exceptions

The check fails CI with the offending file:line list. (Grep-based, zero new
dependencies, < 50 LOC in xtask.)

## 4. What This Buys (Expected Impact)

| Metric | Before (JSON payloads) | After |
|---|---|---|
| Allocs per event (encode) | 8+ (String fields × JSON) | 1 (the payload byte Vec; 0 with §6 arena follow-up) |
| Allocs per event (decode, hot path) | full JSON parse tree | **0** (zero-copy archived view) |
| Payload size | 3–5× binary | ~1× (rkyv is near-struct-layout) |
| schema_version String per event | 1 heap String | 0 (static) |
| Decode CPU | serde_json parse | pointer cast (+ optional bytecheck walk off hot path) |

These close the remaining gap toward the Set-C targets: tick-to-intent
p99 < 50 µs and < 5 allocs/tick.

## 5. Rollout Plan

Ordered to keep every commit green and shippable:

| Phase | Work | Effort | Risk |
|---|---|---|---|
| 1 | rkyv derives + drop `schema_version` field on 7 payload types; fix REST serializers | 6–10 h | Low — compile-driven |
| 2 | Checked-access API (`access_payload`, bytecheck); migrate JetStream consumers | 4–6 h | Low |
| 3 | Collector encode/decode flips (5 collectors), `lane_payload_type` | 6–8 h | Medium — per-collector tests gate each |
| 4 | Intern epoch: deterministic seed, `epoch()`, NATS header stamp + consumer check + quarantine routing | 8–12 h | Medium — touches publish/subscribe |
| 5 | xtask `lint-no-json-hotpath` + tag boundary exceptions | 2–3 h | Low |
| 6 | Benchmarks: encode/decode criterion bench, alloc-count test, size assertions per payload | 4–6 h | Low |

**Total: 30–45 hours** (down from the original 40–60 estimate because the
envelope, interning primitives, and publisher already landed).

Compatibility: lanes are versioned by subject; during rollout, producers and
consumers deploy together per lane (the deployment is a single binary set —
no long-lived mixed-version window). The Parquet raw archive and quarantine
lane keep JSON, so historical replay tooling is unaffected.

## 6. Explicit Non-Goals / Follow-ups

- **Arena/buffer reuse for `payload: Vec<u8>`** — reusing a per-ring scratch
  buffer would reach 0 allocs/event encode; deferred until after benchmarks
  prove the remaining Vec matters.
- **NATS KV–driven live intern updates** — rolling restart on registry change
  is acceptable at current scale; revisit if instrument churn becomes frequent.
- **Replacing serde on the REST/UI boundary** — JSON stays the public API
  format by design.

## 7. Acceptance Criteria (Restated Against This Design)

- [ ] All 9 payload types derive rkyv; none carries a `schema_version` String
- [ ] `cargo xtask lint-no-json-hotpath` passes and runs in CI
- [ ] All 5 JSON-encoding collectors emit rkyv payload bytes
- [ ] JetStream consumers use checked `access_payload`; ring consumers may use trusted access
- [ ] Intern epoch header stamped by producers, verified (cached) by consumers; mismatch routes to quarantine
- [ ] `size_of::<EventEnvelope>() ≤ 96` still holds (existing const assert)
- [ ] Criterion bench: encode+decode of TradePayload envelope ≤ 1 µs round-trip, 0 decode allocs
- [ ] JSON regression tests pass at REST API, quarantine, Parquet writer
