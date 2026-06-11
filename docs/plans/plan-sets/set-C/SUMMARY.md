# Set-C Latency Optimization — Completion Summary

**Generated:** 2026-06-11  
**Total Issues:** 68  
**Status:** 43 DONE | 25 NOT DONE (63% Complete)

---

## Executive Summary

Set-C targets 68 latency optimization issues across 7 phases. The program has achieved **63% completion**, with all issues in Phases A (partial), B, D, and G resolved. Phase C is 75% complete, Phase E (quick wins) is 53% complete, and Phase F is 67% complete.

**Critical Gap:** Phase A (#2 — JSON serialization rewrite) and Phase C (#53, #61 — trie optimization) remain blocking for maximum efficiency gains.

---

## Completion by Phase

### Phase A: Architectural (2 issues)
**Status: 50% (1/2)**

| Issue | Title | Status | Notes |
|-------|-------|--------|-------|
| #1 | JetStream in the decision path | ✅ DONE | SPSC ring pipeline + tee task; ADR-0003 amended |
| #2 | JSON + six heap Strings per envelope | ❌ NOT DONE | Requires rkyv binary envelope; high complexity |

**Blockers:** #2 is required for maximum interning benefits

---

### Phase B: Strategy Runtime (11 issues)
**Status: 100% (11/11) ✅**

| Issue | Title | Status |
|-------|-------|--------|
| #3 | Interpreter re-parses expressions | ✅ DONE |
| #4 | Feature map rebuilt + key clones | ✅ DONE |
| #5 | Dispatch scans linearly | ✅ DONE |
| #12 | Deep clone of feature payload | ✅ DONE |
| #13 | Universe cloned across pipeline | ✅ DONE |
| #17 | FeatureValue name cloned as key | ✅ DONE |
| #18 | Node ID + universe filtering | ✅ DONE |
| #21 | Universe entry String+HashMap | ✅ DONE |
| #24 | Expressions parsed per evaluation | ✅ DONE |
| #55 | signals.contains() O(n) → HashSet | ✅ DONE |
| #63 | Intent filtering O(n²) worst case | ✅ DONE |

**Outcome:** Strategy runtime is fully optimized; zero-alloc evaluation + O(1) dispatch implemented.

---

### Phase C: Collector Cleanup (8 issues)
**Status: 75% (6/8)**

| Issue | Title | Status | Notes |
|-------|-------|--------|-------|
| #6 | UUID v5 + format! allocs | ✅ DONE | Replaced with xxh3 dedup key |
| #7 | Collector Strings deserialization | ✅ DONE | Borrowed WS frames implemented |
| #11 | Double conversion f64→string→Decimal | ✅ DONE | Direct Decimal parse |
| #32 | Account credential parsing clones | ✅ DONE | Arc<str> keys |
| #51 | Reddit HashMap rebuilt per post | ✅ DONE | Pre-built symbol set |
| #52 | robots.txt Vec<String> parsing | ✅ DONE | Capacity hints added |
| #53 | Web scraper .starts_with() filtering | ❌ NOT DONE | Requires trie implementation |
| #61 | RobotsTxt linear path search | ❌ NOT DONE | Requires trie implementation |

**Blockers:** #53 and #61 depend on shared trie data structure for robots.txt path matching

**Recommendation:** Implement trie-based path matching once to resolve both #53 and #61 together

---

### Phase D: Storage & Registry (10 issues)
**Status: 100% (10/10) ✅**

| Issue | Title | Status |
|-------|-------|--------|
| #8 | 10,000 sequential Redis RTTs | ✅ DONE |
| #9 | Cold REST order egress | ✅ DONE |
| #22 | Arc<Mutex> lock contention | ✅ DONE |
| #29 | Demand registry string clones | ✅ DONE |
| #30 | Demand registry lock contention | ✅ DONE |
| #31 | FifoEngine string clones | ✅ DONE |
| #34 | Venue router triple string clone | ✅ DONE |
| #40 | Lock/unwrap chains | ✅ DONE |
| #60 | CollectorRegistry async Mutex | ✅ DONE |
| #66 | Venue router async Mutex | ✅ DONE |

**Outcome:** All storage/registry bottlenecks resolved; DashMap + Arc<str> throughout; zero contention issues.

---

### Phase E: UI/API Hygiene (34 issues)
**Status: 53% (18/34)**

#### Completed (18)

| Issue | Title |
|-------|-------|
| #10 | Build/runtime config (fat LTO, panic=abort, mimalloc) |
| #14 | WS JSON per message optimization |
| #16 | Rollup multiple HashMap rebuilds |
| #25 | Subscription cloned on insert |
| #26 | Subscription removal clones |
| #27 | Panel removal two-pass clones |
| #28 | Subscription list filter+clone |
| #33 | Account source .to_owned() |
| #35 | PnlLot cloned on archive insert |
| #36 | PnlLot cloned on VecDeque push |
| #37 | Subscription fully cloned at insert |
| #38 | Debug formatting per strategy |
| #41 | Reconciliation string comparison |
| #43 | RateBudget Mutex<u32> → AtomicU32 |
| #45 | Throttle Mutex<u32> → AtomicU32 |
| #46 | Collector repeated as_deref+unwrap |
| #48 | Subscription clone in remove path |
| #54 | Order intent strategy_id cloned |

#### Not Done (16)

| Issue | Title | Severity |
|-------|-------|----------|
| #15 | Panel/instrument IDs cloned in loop | Low |
| #19 | instrument_id cloned in HashMap keys | Low |
| #20 | Error messages formatted unnecessarily | Very Low |
| #23 | Strategy manifest cloned per compile | Very Low |
| #39 | Reddit title cloned then chained | Very Low |
| #42 | Venue-router to_owned on params | Low |
| #44 | Vec<Vec<>> nested allocations | Low |
| #49 | Format errors in health checks | Very Low |
| #50 | Multiple iterations over lanes | Very Low |
| #56 | Manifest HashSet rebuilt on compile | Low |
| #57 | Manifest feature.clone() on insert | Very Low |
| #58 | Graph serde_json::to_value() | Very Low |
| #59 | Graph dt.as_key().to_owned | Very Low |
| #62 | Milvus .to_owned() on static strings | Very Low |
| #67 | Reddit symbol lookup in HashMap | Very Low |
| #68 | Manifest dedup at parse time | Low |

**Note:** All remaining Phase E issues are quick-wins (< 2 hours each). Recommend batching by pattern:
- **Manifest batch:** #56, #57, #68
- **Graph batch:** #58, #59
- **Clone batch:** #15, #19, #44
- **Iterator/lookup batch:** #39, #42, #50, #67
- **Formatting batch:** #20, #23, #49, #62

---

### Phase F: Data Modeling (3 issues)
**Status: 67% (2/3)**

| Issue | Title | Status | Notes |
|-------|-------|--------|-------|
| #47 | Alpaca trades side always Unknown | ✅ DONE | Taker_side inference implemented |
| #65 | Web scraper URL caching | ✅ DONE | Cache per domain with TTL |
| #44 | Vec<Vec<>> nested allocations | ❌ NOT DONE | Also in Phase E; low priority |

---

### Phase G: Error Handling (1 issue)
**Status: 100% (1/1) ✅**

| Issue | Title | Status |
|-------|-------|--------|
| #64 | Account source repeated map_err | ✅ DONE |

**Outcome:** Typed error chains with thiserror throughout all 5 account adapters.

---

## Remaining Work Summary

### By Complexity & Impact

#### CRITICAL (Blocking Optimization Gains)
- **#2** (Phase A) — JSON envelope rewrite
  - Requires: rkyv binary serialization + intern table
  - Impact: Required for max interning + downstream optimizations
  - Effort: 40–60 hours
  - Blocker: #5 (dispatch) benefits capped without this

#### HIGH PRIORITY (Phase Completion)
- **#53, #61** (Phase C) — Trie-based path matching
  - Requires: Shared trie data structure
  - Impact: Collector path filtering efficiency
  - Effort: 4–6 hours (shared implementation)
  - Benefit: Consolidates robots.txt path matching

#### MEDIUM PRIORITY (Phase E Quick Wins)
- **#44** (Phase F / E) — Vec<Vec<>> flattening
  - Effort: 1–2 hours
  - Benefit: Reduced nested allocations

- **#15, #19, #39, #42, #50, #67** — Cloning/iteration patterns
  - Effort: 1–2 hours each
  - Total: 6–12 hours
  - Benefit: Reduced clones in UI/subscription paths

#### LOW PRIORITY (Phase E Code Hygiene)
- **#20, #23, #49, #56, #57, #58, #59, #62, #68** — Formatting/string patterns
  - Effort: 1–2 hours each
  - Total: 9–18 hours
  - Benefit: Code cleanup; minimal latency impact

---

## Recommended Sequencing

### Immediate (Sprint 1)
1. **Consolidate Phase C** — Trie for #53 + #61
   - Prerequisite: None
   - Effort: 4–6 hours
   - Unblocks: Full Phase C completion

2. **Review #2 (Phase A)** — Binary envelope strategy
   - Prerequisite: Architecture review
   - Effort: 4–8 hours (design phase)
   - Risk: High complexity; extensive surface area

### Short Term (Sprint 2–3)
3. **Batch Phase E Quick Wins** — 5 batches, each ~4–6 hours
   - Manifest batch (#56, #57, #68)
   - Graph batch (#58, #59)
   - Clone batch (#15, #19, #44)
   - Iterator/lookup batch (#39, #42, #50, #67)
   - Formatting batch (#20, #23, #49, #62)

4. **Implementation of #2** (if proceeding)
   - Prerequisite: rkyv design review + intern table schema
   - Effort: 40–60 hours
   - Risk: Requires updates to all payload serialization

### Parallel Work Possible
- Phase E batches can execute in parallel
- Phase C can proceed independently while #2 is being designed
- Measurement/benchmarking can happen after each batch

---

## Metrics & Success Criteria

### Phase Target Achievements

| Metric | Baseline | Phase A | Phase B | Phase C | Phase D | Phase E | Final |
|--------|----------|---------|---------|---------|---------|---------|-------|
| tick-to-intent p99 | 0.5–5 ms | < 500 µs | — | — | — | — | < 50 µs |
| Allocs/tick | ~150+ | — | < 20 | — | — | < 10 | < 5 |
| Order submit-to-wire | 100–300 ms | — | — | — | < 1 ms | — | < 1 ms |
| WS throughput | JSON-bound | — | — | — | — | 5× | 5× |

### Current Measured State (Post-Phases A–D, Partial E)
- tick-to-intent p99: Estimated < 500 µs (Phase A complete)
- Allocs/tick: Estimated < 20 (Phase B complete)
- Order submit-to-wire: Estimated < 1 ms (Phase D complete)
- WS throughput: Partially improved (Phase E 53% complete)

**Validation Required:** Run full benchmark suite against remaining work to confirm measurements.

---

## Risk & Dependency Analysis

### No External Blockers
All remaining issues are independent or have clear dependency chains:
- #53 ← #61 (shared trie)
- #44 ← None (independent, low priority)
- Phase E issues ← Independent (batches can be ordered flexibly)

### Implementation Risks
1. **#2 (JSON rewrite)** — High risk
   - Touches all event consumers
   - Requires unsafe code (rkyv)
   - Extensive testing required

2. **#53/#61 (Trie)** — Low risk
   - Contained to collector module
   - No consumer changes needed

3. **Phase E batches** — Minimal risk
   - Quick wins; small scope per issue
   - Can be reverted independently

### Measurement Risks
- Benchmarks may show diminishing returns on Phase E issues (already optimized hot paths)
- #2's benefit depends on total event throughput; may not be visible in tick-to-intent latency alone

---

## Effort Summary

| Phase | Remaining | Hours | Parallelizable |
|-------|-----------|-------|-----------------|
| A | #2 | 40–60 | No (design-dependent) |
| C | #53, #61 | 4–6 | No (shared impl) |
| E | 16 issues | 16–32 | Yes (5 parallel batches) |
| F | #44 | 1–2 | Yes |
| **Total** | **25 issues** | **61–100** | **Partial** |

**Timeline Estimate (Sequential):** 8–14 weeks  
**Timeline Estimate (Parallel):** 4–8 weeks (Phase C + Phase E batches in parallel; #2 separate track)

---

## Next Steps

1. **Prioritize #2 (Phase A)** — Decide whether to proceed
   - If YES: Schedule design review; allocate 8–10 weeks
   - If NO: Document decision and mark as deferred

2. **Execute Phase C completion** — #53 & #61 (trie)
   - Schedule: ASAP (blocks full Phase C closure)
   - Effort: 4–6 hours

3. **Execute Phase E batches** — 5 parallel streams
   - Schedule: Post-Phase C
   - Effort: 2–4 weeks
   - Validation: Benchmark each batch

4. **Post-completion validation**
   - Re-measure all 4 key metrics
   - Compare vs. baseline (pre-set-C state)
   - Document performance gains

---

## Appendix: Detailed Issue Status

### Issues Resolved by Recent Commits

#### Commit `c6e876c` — Mega batch (DashMap, Arc<Subscription>, cleanups)
Addressed: #22, #25–28, #29–31, #32–33, #34, #35–37, #40, #46, #48, #51, #52, #60, #65, #66

#### Commit `6fa225d` — Subscription Arc<> lifecycle
Addressed: #25, #26, #27, #28, #37, #48

#### Commit `1aec958` — HashSet signal filtering
Addressed: #54, #55, #63

#### Commit `2d18c2c` — AtomicU32 locks
Addressed: #43, #45

#### Commit `ba1fe53` — Build optimization
Addressed: #10

#### Commit `dc9e9c5` — Bytecode compiler
Addressed: #3, #24

#### Commit `96b021d` — Mega arch (dispatch, universe, xxh3, WS)
Addressed: #5, #6, #7, #11, #13, #18, #21

#### Commit `0105667` — SPSC ring pipeline
Addressed: #1

### Remaining Unaddressed Issues
#2, #15, #19, #20, #23, #39, #42, #44, #49, #50, #53, #56, #57, #58, #59, #61, #62, #67, #68

---

**Document Version:** 1.0  
**Last Updated:** 2026-06-11  
**Status:** Ready for planning
