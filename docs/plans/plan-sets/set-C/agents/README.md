# Agent Query Files — Set C Latency Issues

This directory contains 15 standalone agent query files, each a complete self-contained implementation prompt that can be pasted verbatim into a fresh Claude Code session to implement a group of latency issues in the trading-bot codebase.

---

## File Index

| File | Title | Issues | Phase |
|------|-------|--------|-------|
| [agent-01](agent-01-phase-a-jetstream-hot-path.md) | Remove JetStream from the decision path — in-process SPSC ring pipeline | #1 | A |
| [agent-02](agent-02-phase-a-binary-envelope.md) | Replace JSON envelope with rkyv binary + interned u32 IDs | #2 | A |
| [agent-03](agent-03-phase-b-bytecode-compiler.md) | Compile strategy expressions to postfix bytecode at init; execute per tick | #3, #24 | B |
| [agent-04](agent-04-phase-b-slot-array-features.md) | Replace per-tick HashMap feature rebuild with a stable slot array | #4, #12, #17 | B |
| [agent-05](agent-05-phase-b-dispatch-and-universe.md) | O(1) instrument-indexed dispatch + Arc<Universe> through pipeline stages | #5, #13, #18, #21 | B |
| [agent-06](agent-06-phase-b-intent-filtering.md) | O(1) signal filtering with HashSet + eliminate strategy_id clone | #54, #55, #63 | B |
| [agent-07](agent-07-phase-c-collector-hot-path.md) | Zero-alloc collector normalization: xxh3 identity + borrowed WS frames + direct Decimal | #6, #7, #11 | C |
| [agent-08](agent-08-phase-c-scraper-and-reddit.md) | Web scraper trie + reddit mention table + credential borrow cleanup | #32, #46, #51, #52, #53, #61, #65 | C |
| [agent-09](agent-09-phase-d-redis-and-storage.md) | Delete Redis from the write path; PnL lot move semantics | #8, #31, #35, #36 | D |
| [agent-10](agent-10-phase-d-lock-free-registries.md) | Replace async Mutex in registries with DashMap + Arc<str> IDs | #22, #29, #30, #34, #40, #60, #66 | D |
| [agent-11](agent-11-phase-d-execution-and-ratelimit.md) | Warm HTTP/2 order pools + atomic rate budget + atomic WS throttle | #9, #43, #45 | D |
| [agent-12](agent-12-phase-e-build-config.md) | Apply full release profile + mimalloc + native CPU target | #10 | E |
| [agent-13](agent-13-phase-e-subscriptions.md) | Arc<Subscription> through the UI gateway subscription lifecycle | #25, #26, #27, #28, #37, #48 | E |
| [agent-14](agent-14-phase-e-api-and-misc.md) | API/WS hygiene sweep: binary WS frames, clone removal, format deferral, manifest dedup | #14–#23, #33, #38–#42, #44, #49, #50, #56–#59, #62, #67, #68 | E |
| [agent-15](agent-15-phase-f-and-g.md) | Alpaca side inference + typed error chain | #47, #64, #65 | F+G |

---

## Recommended Execution Order

Work through the agents in the numbered order below. The dependency chain flows top to bottom within Phase A and B; Phase C/D/E/F can mostly be parallelized once Phase A is complete.

```
01  (Phase A — JetStream hot path)
 └─ 02  (Phase A — binary envelope + intern table)
     ├─ 03  (Phase B — bytecode compiler)       ← needs #1 (ring) + can use #4 slot IDs
     ├─ 04  (Phase B — slot array features)     ← needs #2 (intern table)
     ├─ 05  (Phase B — O(1) dispatch)           ← needs #2 (InstrumentId type)
     └─ 07  (Phase C — collector hot path)      ← needs #2 (xxhash-rust dep)
         └─ 09  (Phase D — Redis + storage)    ← needs #7 (xxh3 IDs for dedup ring)

06  (Phase B — intent filtering)                ← independent; do any time
08  (Phase C — scraper + reddit)                ← independent; do any time
10  (Phase D — lock-free registries)            ← independent; do any time
11  (Phase D — execution + rate limit)          ← independent; do any time
12  (Phase E — build config)                    ← independent; do THIS FIRST before benchmarking
13  (Phase E — subscriptions)                   ← independent; can run after 12
14  (Phase E — API/WS hygiene sweep)            ← independent; do any time
15  (Phase F+G — side inference + errors)       ← independent; do any time
```

---

## Parallelism Notes

The following agents can be run **simultaneously** in separate Claude Code sessions (they touch non-overlapping files):

- **12** is fully independent — do it before measuring any other phase's benchmarks.
- **06** and **07** are independent of each other and of Phase A; both can start immediately.
- **08** and **11** are independent of all other agents.
- **10** and **11** are independent of each other; both are Phase D.
- **13** and **14** are independent; both are Phase E.
- **15** is independent of all other agents.

The following agents can run **after 12** in a second parallel wave:

- **13** (subscriptions — no deps beyond 12 being optional context)
- **14** (API hygiene sweep — no deps)

---

## Blocking Dependencies

These must complete **before** the agents that depend on them:

| Must complete first | Unblocks |
|--------------------|---------|
| **01** (JetStream hot path — in-process ring) | 03, 04, 05 (ring must exist for strategy pipeline to use it) |
| **02** (binary envelope + intern table) | 03, 04, 05 (intern table and InstrumentId type needed for slot IDs, dispatch keying) |
| **07** (collector xxh3 IDs) | 09 (storage dedup ring uses xxh3 IDs from the collector normalization change) |

If running agents in parallel, ensure:
- Start **01** and **02** in separate sessions simultaneously — they are independent of each other.
- Do not start **03**, **04**, or **05** until **both** 01 and 02 have landed and their changes are in the working tree.
- Do not start **09** until **07** has landed.

---

## How to Use Each File

1. Open a new Claude Code session in the trading-bot repository root.
2. Paste the full contents of the chosen agent file as your first message.
3. The agent will read relevant source files, implement all fixes listed, run the acceptance tests, and report completion.
4. Each file contains checkboxes in the "Overall Acceptance Criteria" section — the agent should check them off as each criterion is verified.

---

## Issues Coverage Summary

All issues covered across the 15 agents:

**Phase A (agents 01–02):** #1, #2

**Phase B (agents 03–06):** #3, #4, #5, #12, #13, #17, #18, #21, #24, #54, #55, #63

**Phase C (agents 07–08):** #6, #7, #11, #32, #46, #51, #52, #53, #61, #65

**Phase D (agents 09–11):** #8, #9, #22, #29, #30, #31, #34, #35, #36, #40, #43, #45, #60, #66

**Phase E (agents 12–14):** #10, #14, #15, #16, #19, #20, #23, #25, #26, #27, #28, #33, #37, #38, #39, #41, #42, #44, #48, #49, #50, #56, #57, #58, #59, #62, #67, #68

**Phase F+G (agent 15):** #47, #64, #65
