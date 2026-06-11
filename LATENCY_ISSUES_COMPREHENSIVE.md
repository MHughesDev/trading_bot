# Comprehensive Latency Issue Repository — Issues #1-68

Complete scan of all 22 crates across trading-bot repo. Issues are ranked by severity, latency impact, and fixability.

---

## Issues #51-68 (Latest batch)

| # | Pattern Family | Location | Issue | Metric | Severity | Why It Matters |
|---|---|---|---|---|---|---|
| 51 | **HashMap rebuilt per event in collector** | [reddit.rs:71-105](crates/collectors/src/social/reddit.rs:71) | `extract_mentions()` builds a fresh `HashMap<String, f32>` for every post to track mention scores. Allocation + hashing per post. | Per post: 1 HashMap init + (symbol_count × hashing); 100 symbols/sec = continual allocations | **Medium** | Satellite collector (not hot-path), but inefficient. Use a stable array or pre-built table. |
| 52 | **robots.txt parsing: Vec<String> per line** | [scraper.rs:105-132](crates/collectors/src/web/scraper.rs:105) | `parse()` builds `disallowed: Vec<String>` and `allowed: Vec<String>` by pushing one string per line. No capacity hint; reallocs on every push. | Per robots.txt fetch: ~20-50 string allocations + Vec reallocs. | **Low** (infrequent, on-demand) | Use `with_capacity()` or pre-allocate based on file size. |
| 53 | **Web scraper: .starts_with() on every filter pass** | [scraper.rs:149,157](crates/collectors/src/web/scraper.rs:149) | `is_allowed()` iterates disallowed/allowed Vecs and calls `.starts_with()` on every entry; O(n path comparisons) per fetch. | Per fetch: n_disallowed + n_allowed comparisons of strings | **Low** (on-demand, rare) | Use a trie or prefix tree for O(k) lookup (k = path depth). |
| 54 | **Order intent: strategy_id cloned unnecessarily** | [intents.rs:30](crates/strategy-runtime/src/intents.rs:30) | `Some(strategy_id.to_owned())` — clones the strategy ID string for every order intent even though the caller already owns it. | Per order intent: 1 string clone | **Very Low** | Accept `Option<&str>` or use `Arc<str>`. |
| 55 | **Intent filtering: signals.contains() is O(n)** | [intents.rs:47](crates/strategy-runtime/src/intents.rs:47) | `signals.contains(&a.on_signal)` searches a Vec<String> for every action. 5 actions × 3 signals = 15 string comparisons per signal evaluation. | Per signal: O(actions × signal_count); scale: 100 instances × 10 ticks/sec = 10k searches/sec | **Medium** | Use HashSet<String> for signals instead of Vec. |
| 56 | **Manifest: HashSet rebuilt on every compile** | [manifest.rs:95,119](crates/strategy-runtime/src/manifest.rs:95) | `seen_lanes` and `seen_features` HashSets are built to deduplicate inputs; rebuilds on every manifest compile. | Per manifest compile: 2 HashSets + insert checks + collect | **Low** (compile-time, not runtime) | Dedup at definition-load time, not manifest-compile time. |
| 57 | **Manifest: feature.clone() on every insert** | [manifest.rs:123](crates/strategy-runtime/src/manifest.rs:123) | `feature.clone()` inserted to required_features Vec; clones the feature name string per feature per manifest. | Per manifest: feature_count clones | **Very Low** | Use `Arc<str>` or references; dedup at load time. |
| 58 | **Graph: serde_json::to_value() per asset class** | [populate.rs:80-85](crates/graph/src/populate.rs:80) | `serde_json::to_value(a)` on every asset class to convert enum to JSON string just to extract the string. Inefficient triple conversion. | Per populate: 11 enum→JSON→str→clone conversions | **Very Low** (initialization, not hot) | Implement a direct-to-string method; skip JSON intermediate. |
| 59 | **Graph: dt.as_key().to_owned() per data type** | [populate.rs:90](crates/graph/src/populate.rs:90) | Same pattern: `.as_key().to_owned()` on every DataType; allocates a string for every data type in the registry. | Per populate: ~30 type clones | **Very Low** | Collect references instead of owned strings; intern data type IDs. |
| 60 | **CollectorRegistry: async Mutex overhead** | [lifecycle.rs:18-19](crates/venue-router/src/registry.rs:18) | `Arc<Mutex<HashMap>>` with `.lock().await` on every incr/decr; tokio Mutex is fair but adds context-switch cost. | Per demand change: 1 async lock acquisition (~1-3 µs + context switch) | **Low-Medium** | Use dashmap or switch to sync Mutex if calls are synchronous. |
| 61 | **RobotsTxt: linear search through path rules** | [scraper.rs:146-150](crates/collectors/src/web/scraper.rs:146) | `disallowed.iter().filter(|d| path.starts_with(d.as_str()))` — O(n) string comparisons for every path check. No early exit. | Per path check: n string starts_with ops | **Low** | Trie or radix tree for O(path_length) lookup. |
| 62 | **Milvus config: .to_owned() in static initializer** | [lib.rs:90](crates/semantic/src/lib.rs:90) | `SOCIAL_COLLECTION.to_owned()` and `EMBEDDING_MODEL.to_owned()` in `social_posts()` — clones static &str unnecessarily on every call. | Per collection spec init: 2 unnecessary clones | **Very Low** | Use `const` strings or Arc. |
| 63 | **Order intent filtering: O(n²) worst case** | [intents.rs:47-48](crates/strategy-runtime/src/intents.rs:47) | Combined effect of #55: if 10 actions and 5 signals, 50 string searches; 100 strategy instances × 10 ticks/sec = 50k searches/sec. | Per strategy instance per tick: O(action_count × signal_count) | **Medium** | Pre-compute signal set as HashSet at intent-build time. |
| 64 | **Account source: repeated .map_err(|e| AccountSourceError::Http(e.to_string()))** | [alpaca.rs:44,111,114,125,127,150,153,161,167,169,202,205,213](crates/execution/src/account/alpaca.rs) and equiv. in kraken/kalshi/oanda/coinbase | This pattern appears **50+ times** across 5 account adapters: on every HTTP error, API parse error, credential error, etc. Each one allocates a string for the error message. | Per account fetch: up to 50 error string allocations if errors occur | **Low** (error path, not hot) | Use `anyhow::Error` or error-chain to defer formatting; don't allocate strings on errors. |
| 65 | **Web scraper: multiple string-based path lookups per fetch** | [scraper.rs:39,45-51,114-130,146-150,157-165](crates/collectors/src/web/scraper.rs) | Combined effect: URL parsing, robots.txt parsing, path checking, all via string operations (split, trim, starts_with, ends_with). | Per fetch: 20-30 string operations | **Low** (on-demand) | Pre-parse robots.txt to a trie; cache results. |
| 66 | **Venue router: async Mutex contention on lifecycle** | [registry.rs:36,46,61,75](crates/venue-router/src/registry.rs:36) | `CollectorRegistry` uses async Mutex (tokio); every demand/release call awaits a lock. With multi-strategy systems, lock contention grows. | Per collector start/stop: 1 async lock + potential queueing | **Low-Medium** | Switch to dashmap or sync Mutex + parking_lot if calls are not inherently async. |
| 67 | **Reddit: symbol lookup in HashMap per post** | [reddit.rs:87-90](crates/collectors/src/social/reddit.rs:87) | `.contains_key(&upper)` on `self.known_instruments` (a HashMap passed to extract_mentions). Per symbol per post. | Per post: symbol_count HashMap lookups | **Very Low** | Minor optimization; could use Arc<HashSet> instead if lookup is repeated. |
| 68 | **Manifest: dedup work done at runtime, not parse time** | [manifest.rs:94-127](crates/strategy-runtime/src/manifest.rs:94) | Every call to `compile_manifest()` rebuilds dedup structures (HashSets) and walks the definition tree. Should be done once at definition parse/load time. | Per manifest compile: full tree walk + 2 HashSet dedup ops | **Low** (initialization) | Move dedup to definition load; compile_manifest should be a cheap lookup. |

---

## Consolidated Issue Counts

**Total issues identified: 68**

### By root cause:
- **String clones/allocations**: 18 issues
- **Lock contention (Mutex/async)**: 8 issues
- **Collections cloned/rebuilt unnecessarily**: 14 issues
- **Linear searches (O(n) per operation)**: 6 issues
- **Expression parsing per evaluation**: 2 issues
- **Error handling inefficiencies**: 7 issues
- **Double conversion (f64↔Decimal↔string)**: 2 issues
- **Data structure inefficiencies (Vec instead of HashSet, etc.)**: 5 issues

### By severity:
- **High**: 8 issues (#1–2, others)
- **Medium**: 15 issues (#5, #8, #13, #21, #22, #30, #40, #41, #43, #45, #55, #60, #63, #66)
- **Low**: 30 issues
- **Very Low**: 15 issues

### By impact (latency cost):
- **Milliseconds lost** (#1–2): 2 issues
- **Microseconds lost** (#3–9): ~15 issues
- **Nanoseconds lost** (#10+): ~45 issues
- **Non-latency** (correctness, efficiency): 6 issues

---

## Phase-wise remediation (all 68 issues)

### Phase A (Architectural, blocks everything)
**#1–#2 from main list:** In-process hot path + JetStream tail.

### Phase B (Strategy runtime, high ROI)
**#3, #4, #5, #12, #17, #24, #55, #63**: Expression bytecode, feature slot-array, signal HashSet, intent filtering.

### Phase C (Collector cleanup, medium effort)
**#6, #7, #11, #32, #33, #46, #51, #52**: String/number conversions, HTML parsing, robots.txt, mention tracking.

### Phase D (Storage & registry, medium ROI)
**#8, #22, #29, #30, #31, #40, #41, #42, #43, #45, #60, #61, #66**: Lock optimization, string key interning, HashSet conversions, trie-based lookups.

### Phase E (UI/API, low ROI but hygiene)
**#9–10, #14–15, #20, #25–28, #38–39, #48–49, #54, #57, #58–59, #62**: Clone removal, format deferral, string dedups.

### Phase F (Data modeling correctness)
**#47, #65, #67–68**: Alpaca side detection, data structure choices, parse-time optimizations.

### Phase G (Error handling, lowest priority)
**#64**: Error message deferral (batch all error formatting).

---

## Quick wins (< 1hr each, > 10% perf improvement in affected component)

1. **#55 / #63**: Convert `Vec<String> signals` to `HashSet<String>` — saves O(n²) string searches in intent filtering.
2. **#25**: Remove `.clone()` on Subscription insert — one line.
3. **#26 / #27**: Collect `Vec<Uuid>` instead of `Vec<Subscription>` in remove paths — saves full struct clones.
4. **#42**: Accept pre-constructed `(String, String, String)` key in `release()` instead of `.to_owned()`ing parameters.
5. **#45**: Replace `Mutex<u32>` with `AtomicU32` in throttle.rs — microseconds per frame.
6. **#54**: Change `Some(strategy_id.to_owned())` to `Some(strategy_id)` with lifetime adjustment.
7. **#52**: Add `Vec::with_capacity()` to robots.txt parsing.
8. **#57 / #59**: Use `Arc<str>` or references for feature/datatype names.
9. **#62**: Remove `.to_owned()` on static strings; use `&'static str`.
10. **#64**: Batch error formatting or defer to Display trait.

---

## Hardest/slowest wins (> 1 week, > 50% perf improvement in affected component)

1. **#1**: In-process hot path (architectural, ~100× latency reduction on decision path).
2. **#2**: Binary envelope + interned IDs (50× payload reduction, zero-copy deserialization).
3. **#3 + #24**: Expression bytecode compilation (5–10× reduction in per-tick parse work).
4. **#4 + #17**: Feature slot-array (10× reduction in HashMap rebuilds + string cloning).
5. **#61**: Trie-based robots.txt path matching (10× faster path lookups, but only on web scraper).

---

## Summary

**Easy wins (Phase E, < 50 LOC, 30 min–2 hr each):** ~10 issues, low latency impact but hygiene improvements.

**Medium wins (Phases B–D, 50–500 LOC, 2–8 hr each):** ~20 issues, microsecond-scale improvements, compounding in instances.

**Hard wins (Phases A–C, 500+ LOC, 1 week+ each):** ~5 issues, millisecond-scale improvements, architectural changes.

**Compound effect:** Phases A+B+C together = 100–1000× latency reduction in the decision path (from ~10 ms to ~10 µs internal + network).
