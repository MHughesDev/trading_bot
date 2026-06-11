# Latency Optimization Master Plan — Set C

## Overview

This plan covers **68 identified latency issues** in the trading-bot codebase, organized into 7 phases (A through G). The program targets four core metrics:

| Metric | Current Estimate | Target |
|--------|-----------------|--------|
| Decision latency (tick-to-intent p99) | 0.5–5 ms | < 50 µs |
| Allocations per tick | ~150+ | < 5 |
| API latency (order submit-to-wire) | 100–300 ms (cold) | < 1 ms (warm) |
| WS throughput (UI frames/sec) | JSON-bound | 5× improvement |

---

## Phase Summary

| Phase | Label | Issues | Goal | Estimated Effort |
|-------|-------|--------|------|-----------------|
| A | Architectural | #1, #2 | Eliminate JetStream from hot path; replace JSON envelope with binary | 4–6 weeks |
| B | Strategy Runtime | #3, #4, #5, #12, #13, #17, #18, #21, #24, #55, #63 | Zero-alloc evaluation; O(1) dispatch | 3–4 weeks |
| C | Collector Cleanup | #6, #7, #11, #32, #51, #52, #53, #61 | Zero-alloc normalize; borrowed WS frames | 2–3 weeks |
| D | Storage & Registry | #8, #9, #22, #29, #30, #31, #34, #40, #60, #66 | Eliminate Redis RTTs; lock-free registries | 2–3 weeks |
| E | UI/API Hygiene | #10, #14–#16, #19–#20, #23, #25–#28, #33, #35–#39, #41–#50, #54, #56–#59, #62, #67–#68 | Quick wins; Arc; atomic counters; build tuning | 2–3 weeks |
| F | Data Modeling | #44, #47, #65 | Correct Alpaca side; flatten nested Vecs; URL caching | 1–2 weeks |
| G | Error Handling | #64 | Typed errors; lazy formatting | 1 week |

**Total estimated effort:** 15–22 weeks (parallel team execution compresses this significantly)

---

## All Issues — Master Table

| # | Title | Phase | Severity | Quick Win | Sub-Plan |
|---|-------|-------|----------|-----------|----------|
| 1 | JetStream in the decision path | A | High | No | [issue-001.md](issue-001.md) |
| 2 | JSON + six heap Strings per envelope | A | High | No | [issue-002.md](issue-002.md) |
| 3 | Interpreter re-parses expression strings every event | B | High | No | [issue-003.md](issue-003.md) |
| 4 | Feature map rebuilt + every key cloned per tick | B | High | No | [issue-004.md](issue-004.md) |
| 5 | Dispatch scans all instances linearly with string compares + clones event per match | B | Medium | No | [issue-005.md](issue-005.md) |
| 6 | UUID v5 (SHA-1) + two format! allocations per tick for dedup identity | C | Medium | No | [issue-006.md](issue-006.md) |
| 7 | Collector deserializes into owned Strings per trade | C | Medium | No | [issue-007.md](issue-007.md) |
| 8 | 10,000 sequential Redis round-trips per flush | D | Medium | No | [issue-008.md](issue-008.md) |
| 9 | Cold REST order egress | D | Medium | No | [issue-009.md](issue-009.md) |
| 10 | Build/runtime config leaves free speed unused | E | Medium | Yes | [issue-010.md](issue-010.md) |
| 11 | Double conversion: f64→string→Decimal→string | C | Medium | No | [issue-011.md](issue-011.md) |
| 12 | Deep clone of feature payload | B | Medium | No | [issue-012.md](issue-012.md) |
| 13 | Universe cloned across pipeline | B | Medium | No | [issue-013.md](issue-013.md) |
| 14 | WS JSON per message | E | Medium | Yes | [issue-014.md](issue-014.md) |
| 15 | Panel/instrument IDs cloned in loop | E | Low | Yes | [issue-015.md](issue-015.md) |
| 16 | Rollup: multiple HashMap rebuilds per request | E | Low | Yes | [issue-016.md](issue-016.md) |
| 17 | FeatureValue name cloned as HashMap key | B | Medium | No | [issue-017.md](issue-017.md) |
| 18 | Node ID and universe filtering with string compares | B | Low | No | [issue-018.md](issue-018.md) |
| 19 | instrument_id cloned in HashMap keys | E | Low | Yes | [issue-019.md](issue-019.md) |
| 20 | Error messages formatted unnecessarily | E | Very Low | Yes | [issue-020.md](issue-020.md) |
| 21 | Universe entry: String+HashMap fields | B | Medium | No | [issue-021.md](issue-021.md) |
| 22 | Arc<Mutex> lock contention | D | Low-Medium | No | [issue-022.md](issue-022.md) |
| 23 | Strategy manifest cloned per compile (test code) | E | Very Low | Yes | [issue-023.md](issue-023.md) |
| 24 | Expressions parsed per evaluation in universe | B | Medium | No | [issue-024.md](issue-024.md) |
| 25 | Subscription cloned on insert | E | Low | Yes | [issue-025.md](issue-025.md) |
| 26 | Subscription removal: filter+clone+iterate again | E | Low | Yes | [issue-026.md](issue-026.md) |
| 27 | Panel removal: two-pass with clones | E | Low | Yes | [issue-027.md](issue-027.md) |
| 28 | Subscription list: filter+clone+collect | E | Very Low | Yes | [issue-028.md](issue-028.md) |
| 29 | Demand registry: string clones on every add/remove | D | Medium | Yes | [issue-029.md](issue-029.md) |
| 30 | Demand registry: lock contention with nested unwraps | D | Low-Medium | No | [issue-030.md](issue-030.md) |
| 31 | FifoEngine: string clones in P&L path | D | Medium | Yes | [issue-031.md](issue-031.md) |
| 32 | Account source: credential parsing with clones | C | Low | Yes | [issue-032.md](issue-032.md) |
| 33 | Account source: .to_owned() per JSON field | E | Low | Yes | [issue-033.md](issue-033.md) |
| 34 | Venue router: triple string clone on key | D | Low | Yes | [issue-034.md](issue-034.md) |
| 35 | PnlLot: lot cloned on archive insert | E | Very Low | Yes | [issue-035.md](issue-035.md) |
| 36 | PnlLot: lot cloned again on VecDeque push | E | Very Low | Yes | [issue-036.md](issue-036.md) |
| 37 | Subscription fully cloned at insertion | E | Low | Yes | [issue-037.md](issue-037.md) |
| 38 | Debug formatting per strategy request | E | Very Low | Yes | [issue-038.md](issue-038.md) |
| 39 | Reddit: title cloned then chained | E | Very Low | Yes | [issue-039.md](issue-039.md) |
| 40 | Lock/unwrap chains creating contention | D | Low | No | [issue-040.md](issue-040.md) |
| 41 | Reconciliation: string comparison with alloc | E | Low | Yes | [issue-041.md](issue-041.md) |
| 42 | Venue-router: to_owned on parameters | E | Low | Yes | [issue-042.md](issue-042.md) |
| 43 | RateBudget: lock/unwrap on every check | E | Very Low | Yes | [issue-043.md](issue-043.md) |
| 44 | Vec<Vec<>> nested allocations | E | Low | Yes | [issue-044.md](issue-044.md) |
| 45 | Throttle: atomic-like lock per WS frame | E | Low-Medium | Yes | [issue-045.md](issue-045.md) |
| 46 | Collector: repeated as_deref+unwrap_or | E | Very Low | Yes | [issue-046.md](issue-046.md) |
| 47 | Alpaca trades: side always Unknown | F | Medium | No | [issue-047.md](issue-047.md) |
| 48 | Subscription clone in remove path | E | Very Low | Yes | [issue-048.md](issue-048.md) |
| 49 | Format errors in health checks | E | Very Low | Yes | [issue-049.md](issue-049.md) |
| 50 | Multiple iterations over lanes collection | E | Very Low | Yes | [issue-050.md](issue-050.md) |
| 51 | HashMap rebuilt per post in reddit collector | C | Medium | No | [issue-051.md](issue-051.md) |
| 52 | robots.txt parsing: Vec<String> per line without capacity hint | C | Low | Yes | [issue-052.md](issue-052.md) |
| 53 | Web scraper: .starts_with() on every filter pass | C | Low | No | [issue-053.md](issue-053.md) |
| 54 | Order intent: strategy_id cloned unnecessarily | E | Very Low | Yes | [issue-054.md](issue-054.md) |
| 55 | Intent filtering: signals.contains() is O(n) | B | Medium | Yes | [issue-055.md](issue-055.md) |
| 56 | Manifest: HashSet rebuilt on every compile | E | Low | Yes | [issue-056.md](issue-056.md) |
| 57 | Manifest: feature.clone() on insert | E | Very Low | Yes | [issue-057.md](issue-057.md) |
| 58 | Graph: serde_json::to_value() per asset class | E | Very Low | Yes | [issue-058.md](issue-058.md) |
| 59 | Graph: dt.as_key().to_owned per data type | E | Very Low | Yes | [issue-059.md](issue-059.md) |
| 60 | CollectorRegistry: async Mutex overhead | D | Low-Medium | No | [issue-060.md](issue-060.md) |
| 61 | RobotsTxt: linear search through path rules | C | Low | No | [issue-061.md](issue-061.md) |
| 62 | Milvus: .to_owned() on static strings | E | Very Low | Yes | [issue-062.md](issue-062.md) |
| 63 | Order intent filtering: O(n²) worst case | B | Medium | Yes | [issue-063.md](issue-063.md) |
| 64 | Account source: repeated map_err formatting | G | Low | Yes | [issue-064.md](issue-064.md) |
| 65 | Web scraper: multiple string-based path lookups per fetch | F | Low | No | [issue-065.md](issue-065.md) |
| 66 | Venue router: async Mutex contention on lifecycle | D | Low-Medium | No | [issue-066.md](issue-066.md) |
| 67 | Reddit: symbol lookup in HashMap per post | E | Very Low | Yes | [issue-067.md](issue-067.md) |
| 68 | Manifest: dedup work done at runtime, not parse time | E | Low | Yes | [issue-068.md](issue-068.md) |

---

## Recommended Implementation Order

### Phase A → B → C → D → E → F → G

### Why this order?

**Phase A must go first** — it is the single largest source of latency in the system. Bypassing JetStream in the decision path (#1) removes a 0.5–5 ms floor that would mask every subsequent improvement. Replacing the JSON envelope with rkyv binary (#2) introduces the interned `InstrumentId(u32)` / `VenueId(u32)` types that Phase B and Phase C depend on. Neither phase can use its optimal data structures (slot arrays, O(1) dispatch keyed on u32) until the intern table exists.

**Phase B follows** — the strategy runtime is the hot loop. Phase B delivers the most consistent throughput improvements per unit of developer effort. The bytecode compiler (#3), slot array features (#4), and O(1) dispatch (#5) compound with each other: #4 enables #3 (slot IDs are the bytecode operands), and #2 enables #5 (u32 instrument IDs are the dispatch keys). Phase B cannot be fully implemented until Phase A's intern table is available.

**Phase C follows B** — collector cleanup reduces per-event allocation in the ingestion pipeline. This matters more after Phase B because Phase B's improvements expose the collector as a proportionally larger bottleneck. The borrowed WS deserialization (#7) and xxh3 identity (#6) are independent of A/B but deliver less ROI if Phase A's JetStream bottleneck is still present (the bus transit dominates collector normalization time).

**Phase D follows C** — storage, registry, and execution cleanup. The Redis flush bottleneck (#8) is on the write path, not the read path; it doesn't block the strategy but does cause backpressure under sustained load. Cold order egress (#9) is critical for execution quality and can be done in parallel with Phase B if resources allow. Lock contention issues (#22, #30, #40, #60, #66) have lower ROI than Phase B/C hot-path work.

**Phase E is a sweep** — 34 quick-win issues that each require < 2 hours of work. The build config (#10) is actually the very first thing to do within Phase E (measure the compiler baseline before other changes), but Phase E as a whole comes after the architectural foundation. The quick wins in Phase E don't unblock anything — they are independent improvements.

**Phase F addresses data quality** — Alpaca side inference (#47) and web scraper URL caching (#65) are important for strategy correctness (#47) and collector efficiency (#65) but do not affect the hot path latency metrics. They follow the hot-path optimization phases.

**Phase G cleans up error handling** — the thiserror refactor (#64) is code hygiene with minimal runtime impact. It is last because it is safe, independent, and low-risk.

---

## Dependency Graph

```
#2 (intern IDs)
  ├── #5 (dispatch on InstrumentId)
  ├── #29 (demand registry u32 IDs)
  ├── #31 (PnlLot InstrumentId)
  └── #34 (CollectorKey u32 IDs)

#3 (bytecode compiler)
  └── requires: #4 (slot IDs as bytecode operands)
       └── requires: #2 (intern table for slot ID assignment)

#1 (remove JetStream from hot path)
  └── enables: meaningful measurement of all other improvements

#10 (build flags)
  └── should be applied before measuring any phase's impact

#53/#61 (trie for robots.txt)
  └── requires: #52 (capacity hint, feeds the trie builder)
  └── supersedes: #65 (URL caching consolidates all scraper issues)

#55 and #63 → same fix, same PR
#56, #57, #68 → same fix, same PR
#25, #26, #27, #28, #37, #40, #48 → same PR (Arc<Subscription> throughout)
#35 and #36 → same PR (Arc<PnlLot>)
#60 and #66 → same PR (DashMap for registry)
```

---

## 5 Hardest Wins

These require the most design work, have the most dependencies, or touch the most code.

| Rank | Issue | Why Hard |
|------|-------|---------|
| 1 | #1 — JetStream in the decision path | Requires restructuring the entire process topology; moving collectors into platform; building new SPSC ring pipeline; amending ADR |
| 2 | #2 — JSON + six heap Strings per envelope | Requires rkyv derive on all payload types; intern table from Postgres; relaxing unsafe_code; updating all consumers |
| 3 | #3 — Interpreter re-parses every event | Requires designing and implementing a complete bytecode instruction set, compiler front-end, and stack evaluator |
| 4 | #4 — Feature map rebuilt per tick | Requires coordinating with #2 and #3; touches WorldState, runtime, and all feature producers |
| 5 | #8 — 10,000 sequential Redis RTTs | Requires ClickHouse DDL migration (ReplacingMergeTree), in-process ring buffer, Redis scope reduction, and replay dedup |

---

## 10 Quick Wins

These can each be completed in < 2 hours and provide immediate, measurable improvement.

| Rank | Issue | Change Required | Benefit |
|------|-------|----------------|---------|
| 1 | #10 — Build config | 4 lines in Cargo.toml + 2 in config.toml + 3 in main.rs | 10–30% throughput free |
| 2 | #55/#63 — signals.contains() O(n) | Change Vec<String> to HashSet at intent-build time | Eliminates 50k string searches/sec at scale |
| 3 | #43 — RateBudget Mutex<u32> | Replace with AtomicU32 | Lock-free rate limiting |
| 4 | #45 — Throttle Mutex<u32> | Replace with AtomicU32 | Lock-free WS throttle |
| 5 | #29 — Demand registry string clones | Use Arc<str> for lane/instrument IDs | 2 allocs eliminated per demand change |
| 6 | #31 — FifoEngine string clones | Store Side enum directly in PnlLot | 1 alloc eliminated per lot |
| 7 | #38 — Debug format + to_lowercase | Add as_str() or serde rename_all | Zero alloc enum serialization |
| 8 | #58 — serde_json::to_value() per asset class | Add AssetClass::as_str() | 11 JSON cycles at init removed |
| 9 | #26/#27 — Subscription removal clones | Collect Vec<Uuid> instead of Vec<Subscription> | Zero struct copies on disconnect |
| 10 | #39 — Reddit title clone | Use as_deref() instead of clone() | 1 String alloc per post eliminated |

---

## Measurement Strategy

Track these four metrics before and after each phase. Use `cargo bench` for micro-benchmarks and the platform integration harness for end-to-end measurements.

### Metric 1: Decision Latency (tick-to-intent p99)
- **What:** Time from WebSocket frame receipt in the collector to strategy emitting an OrderIntent
- **How to measure:** Add `Instant::now()` at WS frame receipt; record `elapsed()` at OrderIntent construction; emit as a histogram metric to NATS metrics stream
- **Baseline:** 0.5–5 ms (dominated by JetStream ACK wait)
- **Phase A target:** < 500 µs
- **Final target:** < 50 µs p99

### Metric 2: Allocations per Tick
- **What:** Heap allocations (malloc calls) per complete tick through the strategy hot path
- **How to measure:** Instrument with `dhat-rs` in test mode; use `#[global_allocator]` allocation counter in benchmarks
- **Baseline:** ~150+ allocations per tick
- **Phase B target:** < 20
- **Final target:** < 5

### Metric 3: API Latency (order submit-to-wire)
- **What:** Time from OrderIntent receipt by the execution adapter to TCP send completion
- **How to measure:** `Instant::now()` at intent receipt; record at `AsyncWrite::flush()` completion; histogram metric
- **Baseline:** 100–300 ms (cold), ~5 ms (warm)
- **Phase D target:** < 1 ms steady state; zero cold starts after 30s idle

### Metric 4: WS Throughput (UI frames/sec per connection)
- **What:** Maximum UI event frames per second the gateway can deliver per WS connection without dropping frames
- **How to measure:** Load test with simulated WS client counting received frames vs sent; measure at 100, 500, 1000 frames/sec
- **Baseline:** JSON-bound; frame size ~200 bytes; throughput bounded by serialization
- **Phase E target:** Binary frames < 20 bytes; 5× throughput improvement

---

## Success Criteria per Phase

### Phase A
- [ ] tick-to-intent p99 < 500 µs (from baseline > 1 ms)
- [ ] Zero `serde_json` on market data lanes
- [ ] Zero `Publisher::publish` calls in strategy input path
- [ ] Fixed envelope header ≤ 96 bytes

### Phase B
- [ ] Allocations per tick < 20 (from ~150+)
- [ ] Zero HashMap allocations in `process_event`
- [ ] Dispatch cost O(instances on this instrument), not O(total instances)
- [ ] Intent filtering: O(1) signal lookup

### Phase C
- [ ] Zero owned Strings allocated between WS frame receipt and TradePayload construction
- [ ] Zero SHA-1 or UUID v5 computation in collector normalize()
- [ ] robots.txt path matching: O(path_length) trie lookup

### Phase D
- [ ] Zero Redis calls in writer_task flush path
- [ ] Flush of 10,000 events completes < 10 ms
- [ ] Zero TLS handshakes on order submission under steady state
- [ ] All lock contention issues replaced with DashMap or atomic operations

### Phase E
- [ ] All quick-win issues resolved (34 issues)
- [ ] Build uses fat LTO, panic=abort, codegen-units=1, target-cpu=native
- [ ] mimalloc registered as global allocator
- [ ] All subscription operations use Arc<Subscription>

### Phase F
- [ ] Alpaca trade side inference implemented (tick test)
- [ ] Web scraper robots.txt cache per domain with TTL
- [ ] Vec<Vec<>> graph structures flattened

### Phase G
- [ ] Zero `.map_err(|e| Error::Http(e.to_string()))` patterns across all 5 account adapters
- [ ] Typed error enums with thiserror throughout execution crate
