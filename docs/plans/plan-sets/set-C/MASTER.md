# Latency Optimization Master Plan — Set C

**Completion: 54 / 68 shipped · 13 open (2 worth doing) · 1 obsolete — CLOSED
(audited against the code 2026-09-15)**

> **Do not execute this set as written.** ROADMAP §9 gap **G-6** says Set C's
> completion was "never tracked". That is not quite right, and the difference
> matters: **it was tracked — in the wrong files.**
> [`SUMMARY.md`](SUMMARY.md) and [`SESSION_FINAL_SUMMARY.md`](SESSION_FINAL_SUMMARY.md)
> recorded 51/68 and then 55/68 on 2026-06-11, with commit SHAs. Nobody
> propagated either into this MASTER, so the set looked untouched for three
> months. Those two files are also **internally incomplete** (`SUMMARY.md`
> carries verdict rows for only 36 of the 68 issues) and **already stale**: they
> record issue #2 as "NOT DONE — requires rkyv", and rkyv shipped afterwards
> (`crates/domain/src/envelope.rs:34-46`, `crates/event-bus/src/publish.rs:31`).
>
> **Every one of the 68 issues below was therefore re-verified against the code,
> not inherited from those summaries.**
>
> ### Result
>
> | Verdict | Count | Meaning |
> |---|---|---|
> | **SHIPPED** | **54** | the fix is in the code; cited below |
> | **OPEN** | **13** | the file exists and the inefficiency is still there |
> | **OBSOLETE** | **1** | #44 — the `Vec<Vec<_>>` graph structures it assumed never existed (`crates/graph/src/schema.rs` returns flat `Vec<String>`) |
> | **UNKNOWN** | **0** | — |
>
> **Of the 13 open issues, 2 are worth doing — and they are the same loop**, so
> one task slot closes both. The other 11 are cold-path micro-allocations
> (a 30-second reconcile sweep, collector start/stop, one `String` per *order*,
> a rate-limited web crawl, a polled Reddit fetch, an on-demand dashboard REST
> handler). **Close them.** Spending task slots on them would be the
> most expensive way this programme could buy nothing.
>
> ### Phase-level outcome
>
> | Phase | Issues | Verdict |
> |---|---|---|
> | **A** Architectural | 2 | **Complete.** #1: four-stage `rtrb` SPSC ring pipeline in `apps/platform/src/hot_path.rs:65-88`, collectors moved in-process (`crates/collectors/src/crypto/kraken.rs:186` `run_in_process`), JetStream demoted to a tee (`apps/platform/src/tee.rs:18-28` → `publish_fire_and_forget`, `publish.rs:60-80`). #2: `EventEnvelope` is six u32 interned handles + an rkyv `Vec<u8>` payload with a compile-time `assert!(size_of ≤ 96)` (`envelope.rs:34-46`) |
> | **B** Strategy runtime | 11 | **10 shipped, 1 open-not-worth-doing (#21, a documented deliberate choice).** There *is* a real compile step: `crates/strategy-runtime/src/bytecode.rs` compiles conditions to postfix ops with `LoadFeature(u16)` slots at init (`runtime.rs:74`), runs on a fixed `[f64; 32]` stack (`bytecode.rs:284`), and the tree interpreter survives only as the compile-failure fallback (`runtime.rs:321`). Features are a pre-sized `Vec<f64>` (`world.rs:62-66`), not a rebuilt map |
> | **C** Collector cleanup | 8 | **Complete.** xxh3-64 dedup keys replace UUIDv5/SHA-1 (`kraken.rs:36-43`); borrowed `&'a str` WS frames (`kraken.rs:56-90`); robots.txt `PrefixTrie` (`scraper.rs:85-138`) |
> | **D** Storage & registry | 10 | **8 shipped, 2 partial-not-worth-doing (#29, #34 — `Arc::from` still allocates on collector start/stop).** The 10,000 Redis round-trips are gone: in-process `DedupRing` (`crates/storage/src/writer.rs:35-65`), zero Redis calls in `flush_batch` |
> | **E** UI/API hygiene | 34 | **29 shipped, 5 open — and this is where the only work worth doing lives (#14, #15)** |
> | **F** Data modelling | 3 | **2 shipped (#47 Alpaca side, #65 partial), 1 obsolete (#44).** See the note on #47 below |
> | **G** Error handling | 1 | **Complete.** `crates/execution/src/account_source.rs:56-80` is `thiserror` with `#[from]`; each adapter now has exactly one `map_err`, down from "50+" |
>
> ### The only two issues worth a task slot
>
> **#14 + #15 — JSON and deep clones on the per-frame WebSocket fan-out.**
> One task, `crates/api/src/ws/live.rs`.
> - `live.rs:134-139` clones `sub.lane`, `sub.instrument` **and deep-clones
>   `frame.payload` — a `serde_json::Value` — once per subscription per frame.**
>   `Subscription`'s fields are still `String` (`crates/ui-gateway/src/subscriptions.rs:22-26`).
> - `live.rs:140` → `json_msg` → `serde_json::to_vec` (`:210-215`) re-serialises
>   per subscriber rather than once per frame.
> - **The worse instance is upstream and is not in any Set C issue:**
>   `apps/platform/src/hot_path.rs:257-275` builds a `serde_json::json!` object
>   **per trade tick, inside hot-path stage 2** — the pipeline Phase A was built
>   to keep allocation-free. Fold this into the same task.
>
> Everything else: close.
>
> ### Four things this audit found that Set C was not looking for
>
> Recorded here because they are latency-adjacent and were found while reading
> these files. **None is a Set C task and none was fixed.** Items 1–3 are all in
> `apps/platform/src/hot_path.rs` and belong together.
>
> 1. **The live hot path never runs a strategy.** `hot_path.rs:333` —
>    `let mut strategy: Option<StrategyInstance> = None;`, a placeholder. Stage 3
>    pops every `WorldEvent` and discards it; stage 4 can never receive an intent.
>    `InstanceManager::dispatch` is called only from unit tests, and
>    `AppState.instance_manager` (`crates/api/src/state.rs:48,299`) is never wired
>    in. **All of Phase B's optimisation is correct code on a path that cannot
>    fire.** This *is* tracked — as Set E item 26, task 5.4 — but nothing in Set C
>    says so, and Set C's own metric targets ("tick-to-intent p99 < 50 µs") are
>    unmeasurable until it lands.
> 2. **Stage 4 gates orders against a fabricated risk context — untracked, and it
>    becomes dangerous the moment item 1 is fixed.** `hot_path.rs:364-373` builds
>    `risk::GateContext::for_manual_order(Decimal::ZERO, None, Decimal::ONE,
>    Decimal::ONE, Decimal::ZERO, true, 0, 0)` — position size, equity and
>    exposure are hardcoded constants, so every risk check on the live hot path
>    evaluates against fake portfolio state. The comment points at "set-C issue
>    #5", but issue #5 is about dispatch cost and its acceptance criteria say
>    nothing about this: **the note points at a task that will not fix it.**
>    Wiring a strategy into stage 3 without fixing this turns the risk gate into a
>    rubber stamp.
> 3. **Busy-spin on all three ring consumers — untracked.** `hot_path.rs:312`,
>    `:346`, `:385`: every empty-ring poll calls `tokio::task::yield_now().await`
>    in a tight loop. Three tasks *per instrument* saturate a worker slot whenever
>    the ring is empty; at N instruments that is 3N spinning tasks. This defeats
>    the rtrb design at any realistic instrument count — a latency regression
>    introduced *by* the Phase A fix, which no Set C issue covers.
> 4. **`crates/domain/src/ids.rs` is dead code that still documents the retired
>    design.** `DedupKey` / `sequenced_key` / `trade_key` / `event_id_from_key`
>    (UUIDv5 + SHA-1) have no callers; the module is only re-exported at
>    `lib.rs:35`. Its doc comment still describes event identity as if UUIDv5 were
>    live, when it is now xxh3-64 in each collector. **A future collector author
>    who follows that doc reintroduces issue #6.**
>
> Also: `cargo xtask lint-no-json-hotpath` exists (`xtask/src/main.rs:183-305`)
> and enforces exactly Set C's Phase-A rule over `collectors`, `event-bus`,
> `strategy-runtime` and `features` — but **it is not wired into CI**
> (`.github/workflows/ci.yml:129,139` run only `check-money-f64` and
> `check-bars-v1-frozen`), and it currently fails on a zero-copy
> `serde_json::value::RawValue` import its allowlist does not cover
> (`crates/collectors/src/crypto/kraken.rs:20`). An unenforced lint protecting a
> shipped architectural decision will rot.
>
> Finally, on **#47 (Alpaca trade side)**: it shipped and the *collector* mapping
> is correct (`crates/collectors/src/equity/alpaca_data.rs:172-177`, with a
> Lee-Ready tick test at `:126-142` and tests at `:533-590`). But the audit found
> the **inbound order-side parser fails open to `Sell`** — see the report in
> [Set E](../set-E/MASTER.md)'s audit header; it is a correctness issue, not a
> latency one.

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

| # | Title | Phase | Severity | Quick Win | Sub-Plan | **Verdict (2026-09-15)** | **Evidence** |
|---|-------|-------|----------|-----------|----------|---|---|
| 1 | JetStream in the decision path | A | High | No | [issue-001.md](issue-001.md) | ✅ SHIPPED | `hot_path.rs:65-88` rtrb rings; `tee.rs:18-28`; collectors in-process `kraken.rs:186` |
| 2 | JSON + six heap Strings per envelope | A | High | No | [issue-002.md](issue-002.md) | ✅ SHIPPED | `domain/src/envelope.rs:34-46` u32 interned + rkyv; `assert!(size_of<=96)` |
| 3 | Interpreter re-parses expression strings every event | B | High | No | [issue-003.md](issue-003.md) | ✅ SHIPPED | `strategy-runtime/src/bytecode.rs`; compiled at init `runtime.rs:74` |
| 4 | Feature map rebuilt + every key cloned per tick | B | High | No | [issue-004.md](issue-004.md) | ✅ SHIPPED | `world.rs:62-66` `feature_slots: Vec<f64>` |
| 5 | Dispatch scans all instances linearly with string compares + clones event per match | B | Medium | No | [issue-005.md](issue-005.md) | ✅ SHIPPED | `runtime.rs:429` `by_instrument` map; `:514-534` passes `&event` |
| 6 | UUID v5 (SHA-1) + two format! allocations per tick for dedup identity | C | Medium | No | [issue-006.md](issue-006.md) | ✅ SHIPPED | `kraken.rs:36-43` xxh3-64 over `[u8;24]` |
| 7 | Collector deserializes into owned Strings per trade | C | Medium | No | [issue-007.md](issue-007.md) | ✅ SHIPPED | `kraken.rs:56-90` `#[serde(borrow)] &'a str` |
| 8 | 10,000 sequential Redis round-trips per flush | D | Medium | No | [issue-008.md](issue-008.md) | ✅ SHIPPED | `storage/src/writer.rs:35-65` in-process `DedupRing`; no Redis in `flush_batch` |
| 9 | Cold REST order egress | D | Medium | No | [issue-009.md](issue-009.md) | ✅ SHIPPED | `execution/src/alpaca.rs:34-40` http2 keep-alive + `tcp_nodelay` |
| 10 | Build/runtime config leaves free speed unused | E | Medium | Yes | [issue-010.md](issue-010.md) | ✅ SHIPPED | `Cargo.toml:193-199` lto=fat, panic=abort, cgu=1; mimalloc `main.rs:2-3` |
| 11 | Double conversion: f64→string→Decimal→string | C | Medium | No | [issue-011.md](issue-011.md) | ✅ SHIPPED | `prediction/kalshi.rs:86,94,102` `Decimal::try_from(f64)` direct |
| 12 | Deep clone of feature payload | B | Medium | No | [issue-012.md](issue-012.md) | ✅ SHIPPED | `world.rs:62-66,114-120` slot write, no payload store |
| 13 | Universe cloned across pipeline | B | Medium | No | [issue-013.md](issue-013.md) | ✅ SHIPPED | `nodes/mod.rs:70` `Arc<Universe>` |
| 14 | WS JSON per message | E | Medium | Yes | [issue-014.md](issue-014.md) | ❗ OPEN — **DO** | `ws/live.rs:140,210-215` re-serialises per subscriber; **worse: `hot_path.rs:257-275` builds `json!` per trade tick** |
| 15 | Panel/instrument IDs cloned in loop | E | Low | Yes | [issue-015.md](issue-015.md) | ❗ OPEN — **DO** | `ws/live.rs:134-139` deep-clones `frame.payload` per subscription per frame — same task as #14 |
| 16 | Rollup: multiple HashMap rebuilds per request | E | Low | Yes | [issue-016.md](issue-016.md) | ⭕ OPEN — close | `api/src/rollup/mod.rs:81-107` five maps per request — on-demand REST handler |
| 17 | FeatureValue name cloned as HashMap key | B | Medium | No | [issue-017.md](issue-017.md) | ✅ SHIPPED | `world.rs:62-66`; `runtime.rs:250` slot lookup |
| 18 | Node ID and universe filtering with string compares | B | Low | No | [issue-018.md](issue-018.md) | ⭕ OPEN — close | `nodes/mod.rs:70,75-124` — universe pipeline, <10 nodes/pass |
| 19 | instrument_id cloned in HashMap keys | E | Low | Yes | [issue-019.md](issue-019.md) | ✅ SHIPPED | `runtime.rs:415` `InstanceKey = (Arc<str>, Arc<str>)` |
| 20 | Error messages formatted unnecessarily | E | Very Low | Yes | [issue-020.md](issue-020.md) | ✅ SHIPPED | `routes/dashboard.rs:43-49` static `&str`; zero `format!` in file |
| 21 | Universe entry: String+HashMap fields | B | Medium | No | [issue-021.md](issue-021.md) | ⭕ OPEN — close | `nodes/mod.rs:27-31` — deliberate, documented at `:24-26` |
| 22 | Arc<Mutex> lock contention | D | Low-Medium | No | [issue-022.md](issue-022.md) | ✅ SHIPPED | `writer.rs` lock-free; `demand-manager/src/registry.rs:36-37` DashMap |
| 23 | Strategy manifest cloned per compile (test code) | E | Very Low | Yes | [issue-023.md](issue-023.md) | ✅ SHIPPED | `tests/manifest_compile.rs` zero clones; `compile_manifest(&def)` |
| 24 | Expressions parsed per evaluation in universe | B | Medium | No | [issue-024.md](issue-024.md) | ✅ SHIPPED | `nodes/mod.rs:56-68` pre-compiles filters once; `filter.rs:18-28` |
| 25 | Subscription cloned on insert | E | Low | Yes | [issue-025.md](issue-025.md) | ✅ SHIPPED | `subscriptions.rs:48` `DashMap<Uuid, Arc<Subscription>>` |
| 26 | Subscription removal: filter+clone+iterate again | E | Low | Yes | [issue-026.md](issue-026.md) | ✅ SHIPPED | `subscriptions.rs:111-122` `Vec<Arc<..>>`, zero struct clones |
| 27 | Panel removal: two-pass with clones | E | Low | Yes | [issue-027.md](issue-027.md) | ✅ SHIPPED | `subscriptions.rs:125-136` `Arc::clone` only |
| 28 | Subscription list: filter+clone+collect | E | Very Low | Yes | [issue-028.md](issue-028.md) | ✅ SHIPPED | `subscriptions.rs:139-145` returns `Vec<Arc<Subscription>>` |
| 29 | Demand registry: string clones on every add/remove | D | Medium | Yes | [issue-029.md](issue-029.md) | ⭕ OPEN — close | `demand-manager/src/registry.rs:53,73,99` 2 `Arc::from` per op — pipeline lifecycle, not per-tick |
| 30 | Demand registry: lock contention with nested unwraps | D | Low-Medium | No | [issue-030.md](issue-030.md) | ✅ SHIPPED | `registry.rs:6,36-45` DashMap; zero `lock()` in crate |
| 31 | FifoEngine: string clones in P&L path | D | Medium | Yes | [issue-031.md](issue-031.md) | ✅ SHIPPED | `storage/src/pnl.rs:31-32` `side: Side` enum; lots as `Arc<PnlLot>` |
| 32 | Account source: credential parsing with clones | C | Low | Yes | [issue-032.md](issue-032.md) | ✅ SHIPPED | `account/alpaca.rs:42-52` `parse_creds` returns `(&str,&str)` |
| 33 | Account source: .to_owned() per JSON field | E | Low | Yes | [issue-033.md](issue-033.md) | ⭕ OPEN — close | `account/kalshi.rs:103,108,151` — periodic account reconciliation, cold |
| 34 | Venue router: triple string clone on key | D | Low | Yes | [issue-034.md](issue-034.md) | ⭕ OPEN — close | `venue-router/src/lifecycle.rs:42-47` 3 `Arc::from` — collector start/stop only |
| 35 | PnlLot: lot cloned on archive insert | E | Very Low | Yes | [issue-035.md](issue-035.md) | ✅ SHIPPED | `storage/src/pnl.rs:61-68` `Arc<PnlLot>`; comment cites (#35, #36) |
| 36 | PnlLot: lot cloned again on VecDeque push | E | Very Low | Yes | [issue-036.md](issue-036.md) | ✅ SHIPPED | `pnl.rs:112-116` two `Arc::clone`, no struct copy |
| 37 | Subscription fully cloned at insertion | E | Low | Yes | [issue-037.md](issue-037.md) | ✅ SHIPPED | `subscriptions.rs:48,84-96` one `Arc::new` + `Arc::clone` |
| 38 | Debug formatting per strategy request | E | Very Low | Yes | [issue-038.md](issue-038.md) | ✅ SHIPPED | `routes/strategies.rs:348-349` `.as_str()`; helper `manifest.rs:29-39` |
| 39 | Reddit: title cloned then chained | E | Very Low | Yes | [issue-039.md](issue-039.md) | ✅ SHIPPED | `social/reddit.rs:112-118` `as_deref().unwrap_or_default()` |
| 40 | Lock/unwrap chains creating contention | D | Low | No | [issue-040.md](issue-040.md) | ✅ SHIPPED | `subscriptions.rs:48,96-121` DashMap; zero `.lock().unwrap()` |
| 41 | Reconciliation: string comparison with alloc | E | Low | Yes | [issue-041.md](issue-041.md) | ⭕ OPEN — close | `reconciliation/src/positions.rs:33` one `String` per position — 30 s sweep |
| 42 | Venue-router: to_owned on parameters | E | Low | Yes | [issue-042.md](issue-042.md) | ⭕ OPEN — close | `venue-router/src/lifecycle.rs:81-85` — collector start/stop only |
| 43 | RateBudget: lock/unwrap on every check | E | Very Low | Yes | [issue-043.md](issue-043.md) | ✅ SHIPPED | `demand-manager/src/rate_budget.rs:20,46,82-104` DashMap + atomics |
| 44 | Vec<Vec<>> nested allocations | E | Low | Yes | [issue-044.md](issue-044.md) | ✖ OBSOLETE | `crates/graph/` has zero `Vec<Vec` hits; `schema.rs:117-132` returns flat `Vec<String>` — the premise never existed |
| 45 | Throttle: atomic-like lock per WS frame | E | Low-Medium | Yes | [issue-045.md](issue-045.md) | ✅ SHIPPED | `ui-gateway/src/throttle.rs:17-33` `tokio::sync::watch`; Mutex only in `#[cfg(test)]` |
| 46 | Collector: repeated as_deref+unwrap_or | E | Very Low | Yes | [issue-046.md](issue-046.md) | ✅ SHIPPED | `equity/alpaca_data.rs:65-88` all fields borrow `&'a str` |
| 47 | Alpaca trades: side always Unknown | F | Medium | No | [issue-047.md](issue-047.md) | ✅ SHIPPED | `alpaca_data.rs:172-177` correct B/S map + Lee-Ready tick test `:126-142`; tests `:533-590` |
| 48 | Subscription clone in remove path | E | Very Low | Yes | [issue-048.md](issue-048.md) | ✅ SHIPPED | `subscriptions.rs:101-108` `remove` yields the `Arc` |
| 49 | Format errors in health checks | E | Very Low | Yes | [issue-049.md](issue-049.md) | ✅ SHIPPED | `routes/venue_health.rs:127-147` typed `PingError`; comment cites (#49) |
| 50 | Multiple iterations over lanes collection | E | Very Low | Yes | [issue-050.md](issue-050.md) | ✅ SHIPPED | `event-bus/src/nats.rs:44-89` single loop over lanes |
| 51 | HashMap rebuilt per post in reddit collector | C | Medium | No | [issue-051.md](issue-051.md) | ✅ SHIPPED | `social/reddit.rs:197,229,57-59` `mention_scratch` allocated once |
| 52 | robots.txt parsing: Vec<String> per line without capacity hint | C | Low | Yes | [issue-052.md](issue-052.md) | ✅ SHIPPED | `web/scraper.rs:158-186` rules go into tries from `&str` |
| 53 | Web scraper: .starts_with() on every filter pass | C | Low | No | [issue-053.md](issue-053.md) | ✅ SHIPPED | `web/scraper.rs:85-138` `PrefixTrie`, comment cites (#53, #61) |
| 54 | Order intent: strategy_id cloned unnecessarily | E | Very Low | Yes | [issue-054.md](issue-054.md) | ⭕ OPEN — close | `strategy-runtime/src/intents.rs:46` one `String` per *order*, not per tick |
| 55 | Intent filtering: signals.contains() is O(n) | B | Medium | Yes | [issue-055.md](issue-055.md) | ✅ SHIPPED | `intents.rs:56,63` `&HashSet<String>` — O(1) |
| 56 | Manifest: HashSet rebuilt on every compile | E | Low | Yes | [issue-056.md](issue-056.md) | ✅ SHIPPED | `manifest.rs:98-146` sort/dedup helpers, comment cites (#56) |
| 57 | Manifest: feature.clone() on insert | E | Very Low | Yes | [issue-057.md](issue-057.md) | ✅ SHIPPED | `manifest.rs:135-145` `Vec<&str>`, one alloc per unique name |
| 58 | Graph: serde_json::to_value() per asset class | E | Very Low | Yes | [issue-058.md](issue-058.md) | ✅ SHIPPED | `graph/src/populate.rs:68-83` `Vec<&'static str>` |
| 59 | Graph: dt.as_key().to_owned per data type | E | Very Low | Yes | [issue-059.md](issue-059.md) | ✅ SHIPPED | `graph/src/populate.rs:85-88` `Vec<&'static str>` |
| 60 | CollectorRegistry: async Mutex overhead | D | Low-Medium | No | [issue-060.md](issue-060.md) | ✅ SHIPPED | `venue-router/src/registry.rs:17-18` `Arc<DashMap<..>>` |
| 61 | RobotsTxt: linear search through path rules | C | Low | No | [issue-061.md](issue-061.md) | ✅ SHIPPED | `web/scraper.rs:120-137` `longest_match` O(path_len) |
| 62 | Milvus: .to_owned() on static strings | E | Very Low | Yes | [issue-062.md](issue-062.md) | ✅ SHIPPED | `semantic/src/lib.rs:80-94` `&'static str` fields, `Copy` |
| 63 | Order intent filtering: O(n²) worst case | B | Medium | Yes | [issue-063.md](issue-063.md) | ✅ SHIPPED | same fix as #55 — `intents.rs:56,63` |
| 64 | Account source: repeated map_err formatting | G | Low | Yes | [issue-064.md](issue-064.md) | ✅ SHIPPED | `execution/src/account_source.rs:56-80` thiserror `#[from]`; 1 `map_err` per adapter (was 50+) |
| 65 | Web scraper: multiple string-based path lookups per fetch | F | Low | No | [issue-065.md](issue-065.md) | ⭕ OPEN — close | `scraper.rs:553,573-578` cache+trie done; domain/path still derived twice per fetch — network-bound crawl |
| 66 | Venue router: async Mutex contention on lifecycle | D | Low-Medium | No | [issue-066.md](issue-066.md) | ✅ SHIPPED | `venue-router/src/registry.rs:34-73` DashMap entry/get_mut/remove, all sync |
| 67 | Reddit: symbol lookup in HashMap per post | E | Very Low | Yes | [issue-067.md](issue-067.md) | ⭕ OPEN — close | `social/reddit.rs:70` std HashMap — polled collector, SipHash on <=8 chars is noise |
| 68 | Manifest: dedup work done at runtime, not parse time | E | Low | Yes | [issue-068.md](issue-068.md) | ✅ SHIPPED | `manifest.rs:98-157` helpers, doc cites (#56, #68) |

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
- [x] Zero `.map_err(|e| Error::Http(e.to_string()))` patterns across all 5 account adapters
- [x] Typed error enums with thiserror throughout execution crate

---

## Progress Log

| Date | Change |
|------|--------|
| 2026-06-11 | Optimisation sweep run across Phases A–G. Recorded in [`SUMMARY.md`](SUMMARY.md) (51/68) and [`SESSION_FINAL_SUMMARY.md`](SESSION_FINAL_SUMMARY.md) (55/68), **never propagated into this file** — which is why the set appeared untouched for three months. Many fixes carry their issue number in a code comment (`pnl.rs:63` "(#35, #36)", `scraper.rs:85` "(#53, #61)", `manifest.rs:98` "(#56, #68)", `venue_health.rs:129` "(#49)"). |
| 2026-09-15 | **Set closed.** All 68 issues re-verified against the code by the plan-hygiene track (ROADMAP §9 gap G-6, human gate H-5). Verdicts and evidence are in the master table above: **54 shipped · 13 open · 1 obsolete**. Only #14 and #15 are worth doing, and they are one task. Four untracked defects recorded in the header — three of them in `apps/platform/src/hot_path.rs`. `SUMMARY.md` and `SESSION_FINAL_SUMMARY.md` are **superseded by this table**. |

> **On the two sibling summary files.** Keep them for their commit SHAs and
> nothing else. `SUMMARY.md` carries verdict rows for 36 of 68 issues and
> declares #2 "NOT DONE"; #2 shipped. The lesson is the one
> [`add-plan.md`](../../../procedures/add-plan.md) already states — **a plan's
> completion lives in the plan, not in a sibling report** — and the omission cost
> this programme a roadmap gap (G-6) and a full re-audit to recover what was
> already known on 2026-06-11.
>
> **One success criterion above is deliberately left unticked.** Phase E's
> "target-cpu=native" was declined with a written rationale at
> `.cargo/config.toml:1-4` (it breaks CI runners without AVX-512; the note
> suggests `RUSTFLAGS` at build time, or `x86-64-v3`). That is a reasoned
> deviation, not an omission, and it should not be re-opened as work.

---

## Closure of the "do" items (track LEGACY, 2026-10-03)

The six items the verdict above marked worth doing. The other 11 stay "close, not worth doing".

| Item | Status | What changed |
|---|---|---|
| **#14/#15** `ws/live.rs` re-serialises and clones per subscriber | `DONE` | `crates/api/src/live_bus.rs`: a frame's payload is serialised **once**, at publish, into a `Box<RawValue>`, and the frame travels as `Arc<LiveFrame>` (`LiveSender = broadcast::Sender<Arc<LiveFrame>>`), so fan-out to N sockets is N reference-count bumps, not N deep clones of a `serde_json::Value`. `ws/live.rs::forward_frame` splices the already-serialised text into its own envelope (`FrameOut`, which adds only the per-subscription `sub_id`). Tests: `live_bus::one_serialisation_is_shared_by_every_receiver` (the receivers hold the same allocation: `Arc::ptr_eq`), `no_listener_means_no_work`, `invalid_text_is_dropped_not_forwarded`, and `ws::live::frame_envelope_matches_the_enum_variant` (the wire bytes equal the old `WsOutMessage::Frame` serialisation) |
| **(fold)** `hot_path.rs` builds a `json!` per tick | `DONE` | The tape frame is written straight into one `String` by `tape_frame_json` (no `serde_json::Value`), and only when `live_bus::has_listeners` is true: with no panel open the tape costs nothing. `publish_text` validates once and drops invalid text. Test: `tape_frame_is_valid_json_with_the_published_shape` |
| **finding 3** busy-spin `yield_now` on all three ring consumers | `DONE` | `IdleBackoff` in `hot_path.rs`: a wall-clock-bounded spin (200 us), then park with backoff (200 us doubling to 1 ms). Used by stages 2, 3 and 4. Tests: `idle_backoff_spins_then_sleeps_then_caps_and_resets` (pure, synthetic clock), `an_idle_consumer_parks_instead_of_spinning` (179 polls in 60 ms; **mutation-checked**: replacing the sleep with a yield gives 62,072 polls and the test fails). All 18 `hot_path` tests, including the end-to-end fill tests that run stages 3 and 4, pass. **A measured cost:** a parked wake-up is **14-16 ms on this Windows box** for a requested 1 ms, because the OS timer tick is 15.6 ms (`a_parked_wakeup_is_measured_not_assumed`); on Linux, where the platform runs, it is about the request. So after a lull a tick can wait up to one timer tick. Acceptable for 1-minute bars and the tape; `timeBeginPeriod(1)` at boot on Windows is the fix if it ever matters (`main.rs`, orchestrator-owned) |
| **finding 4** dead UUIDv5 code in `ids.rs` | `DONE` | `crates/domain/src/ids.rs` deleted and its two lines in `crates/domain/src/lib.rs` (`pub mod ids;`, `pub use ids::...`). It had no caller anywhere (grep over `crates/` and `apps/`), so removing it beat correcting a doc for code nobody runs. `docs/Specs/DATA-003-timestamps-and-identity.md` AC-5 still cites `domain::tests::event_id_from_key_is_deterministic`, which no longer exists: that spec line should now cite the xxh3 dedup tests instead (spec not edited here) |
| **lint** `lint-no-json-hotpath` not in CI, fails on `RawValue` | `DONE` | `.github/workflows/ci.yml`: new job `lint-no-json-hotpath` (own block, before `check-bars-v1-frozen`). The lint was red on **34** lines, not one, all of them *decoding* a venue's REST/WS response (`RawValue` imports, `serde_json::Number` fields, `from_slice` calls), none encoding a payload. `xtask/src/main.rs::json_use_is_boundary_decode` now lets the decode side through and still fails on `to_string`, `to_vec`, `to_writer`, `to_value`, `to_raw_value`, `Serializer`, `Map`. **`crates/collectors/src/crypto/kraken.rs` was not edited**: it is pre-existing dirty, and its `RawValue` use is the legitimate zero-copy parse the lint exists to allow. 25 `xtask` tests pass, including `decoding_a_venue_response_is_allowed_but_encoding_a_payload_is_not`; `cargo xtask lint-no-json-hotpath` exits 0 |
| **metrics** p99 and allocations-per-tick never measured | `DONE` for the synchronous path; end to end `NOT MEASURED` | Below |

### Measured results

Machine: the Windows 11 dev box, release profile with `lto=off, codegen-units=16`
(`hot_path::metrics::per_tick_allocations_and_latency`, 20,000 ticks;
`crates/domain/tests/trade_decode_allocs.rs`). A counting global allocator, thread-local so parallel tests
do not pollute it. Reproduce with `cargo test --release -p platform -- hot_path::metrics --nocapture`.

| Metric | Result | Target |
|---|---|---|
| Tick-to-intent latency: aggregate, build the world event, evaluate one EMA-cross strategy | **p50 0.3 us, p99 0.4-0.5 us, p99.9 0.5-0.6 us** (three runs) | final < 50 us p99 |
| `rkyv` decode of the trade payload | 0.1 us p50, 0.2 us p99 (debug build); **1 allocation** (the trade-id `String`) | n/a |
| Allocations per tick, whole synchronous path | decode 1 + stage 2 (`instrument_id.clone()` into the `WorldEvent`) 1 + stage 3 (one strategy) 4 = **6** | Phase B < 20 (met); final < 5 (**missed by one**) |
| Tape frame | 0 with no socket listening; 1 (the `String`) with one | n/a |
| Baseline for comparison | ~150+ allocations, 0.5-5 ms | |

**What this does and does not show.** The synchronous decision work is about 100x inside the final
latency target, and allocations are 25x below the baseline but one above the final target (the stage 3
four are inside `strategy-runtime`, and the stage 2 one is the `String` in `WorldEvent::Bar`, a `strategy-runtime`
type: neither is this track's code). **Not measured, and why:** (1) WebSocket frame receipt to
ring push (the Kraken JSON parse and the `rtrb` push, in `crates/collectors`, which is not this track's file
and needs a live socket or a recorded frame replay to drive); (2) the cross-thread ring hand-off and the
wake-up of a parked consumer, which is the OS-timer figure above on this box and about the sleep request on
Linux; (3) the order-submit leg (metric 3) and UI throughput (metric 4). A true tick-to-intent p99 needs
the `Instant` stamped at frame receipt and read at `OrderIntent` construction (the method this section
specifies), which touches the collector and `strategy-runtime`. Until then the figure above is the bound
on the part of the path this repo's tests can drive, not the end-to-end p99.

### ADR rows

| ID | Decision | Rejected alternative |
|---|---|---|
| ADR-C-LINT-1 | The lint's rule is "no JSON **encoding** in a hot-path crate"; boundary **decoding** is allowed | Allow-listing the 34 lines by file, which would rot; editing `kraken.rs` to drop the `RawValue` import, which would trade a zero-copy parse for an allocating one to satisfy a lint |
| ADR-C-IDS-1 | Delete `ids.rs` | Fix its doc and keep the dead UUIDv5 code (a future author could still follow it) |
| ADR-C-IDLE-1 | Spin by wall-clock budget, then sleep | A fixed count of `yield_now` polls (64 polls is tens of microseconds and every real tick arrives later, so every tick met a sleeping consumer) |
| ADR-C-BUS-1 | `LiveFrame` carries a pre-serialised `RawValue`; `LiveSender` carries `Arc<LiveFrame>`. `live_bus.rs` is outside the brief's literal file list; it is the other half of `ws/live.rs` and has no other user than the publishers in `apps/platform` | Leaving `live_bus.rs` alone and caching serialisation in the socket loop, which cannot share across sockets |

