# ADR-0034: Context assembly — one budget, routed exposure, cache-stable layers

**Status:** Proposed
**Date:** 2026-09-14
**Spec:** `docs/Specs/AGENT-005-context-assembly.md`
**Amends:** AGENT-001 §8.2, AGENT-002 §2 (P2, P10), BS-007 04 §3, HARNESS_CONFORMANCE item 10, ADR-0031 §3, ADR-0033 §4

## 1. Context

A code audit on 2026-09-14 (numbers in AGENT-005 §0) found that the hosted driver sends ~51 KB of byte-identical prefix on every call with no caching and the full 56-tool catalogue, while the local loop routes tools correctly but never charges the charter or the schemas against the budget it claims to enforce. Both drivers were built to different, and partly contradictory, written doctrines.

## 2. Decision

1. **One budget.** Every byte sent to a model is charged against `effective_budget_tokens − reserve_for_output` before the working state is sized. Tool schemas and the system prompt are reserved first; the body gets the remainder, with a 25% floor. This holds at every tier.
2. **Stable-first layers with cache breakpoints.** Requests are assembled tools → charter → opening message → history → subtask. The charter is byte-stable and small (≤ 3.2 KB); dynamic values live in the first user message. On Anthropic, breakpoints close each of the first three layers; on OpenAI the same layering makes automatic prefix caching hit; on local backends it makes prompts reproducible.
3. **Routed exposure at every tier; scope is a profile value.** The catalogue is unbounded. The frontier profile sees a *session-scoped* slice (routed once from the goal, extended append-only by `search_tools`, evicting only at the cap). Local profiles keep *step-scoped* routing. Truncation is never alphabetical.
4. **Loss is recoverable.** Anything cut from context leaves a scratchpad handle. The hosted driver gets the workspace tools so the handles are readable.
5. **The estimator is calibrated by the provider.** Per-model bytes-per-token is learned from `Usage` and fed back; 4.0 is only the prior.
6. **Code execution is the escape valve for catalogue growth.** `run_python`, gated by the existing `code_execution` profile field, backed by the existing skill runner; the analysis suite ships as templates rather than as fifty bespoke tools.

## 3. What this amends (the doctrine conflicts it resolves)

| Conflict | Resolution |
|---|---|
| AGENT-002 P2/P10 and BS-007 04 §3 ("no tool schemas in context", "the tool list never changes within a session") vs. ADR-0031/0032 (per-step dynamic routing with schemas in front of the model) | The harness path routes. Session-scoped, append-only exposure on the frontier honours the *reason* for the older rule (cache stability) without its cost (56 schemas every call). The older rule stands only for the SDK-container path (ADR-0024). |
| Compaction: AGENT-001 §17 "same-model, server-side, at ~100–150k" vs. ADR-0033 §4 "deterministic at 0.95 of the window" | Deterministic first at every tier, against the *body* budget. Model summarisation additionally on `frontier` and `local_high` when deterministic compaction cannot fit. |
| "Fixed layout" (conformance item 10) never given numerically | AGENT-005 §2.1 is the layout; §4.5 the shares. `Section::Tools` is removed because nothing ever rendered it. |
| Spill summary "≤ 300 tokens" (AGENT-001 §12) vs. "≤ 1k" (§22 A8) | ≤ 300 tokens plus the handle, for both drivers. |
| Exposure numbers: ≤ 15 (AGENT-002 §8) / 24 (profile) / 20–40 (guide) / 2–6 local | Profile values are authoritative: 24 frontier (4 of them reserved for search), 8 local_mid, 6 local_small. AGENT-002 §8's "≤ 15 always-loaded" applies to the external MCP client only. |
| `search_tools` status (escape hatch vs. "removed the round-trip") | Both: the deterministic goal search runs first; `search_tools` remains the recovery path and is now on the hosted tier too. |
| Wall clock (AGENT-001 §17 vs. ADR-0033) | Unchanged by this ADR: none. Profiles keep `max_wall_clock_s` for the proxy path only. |
| Degraded tier capability (ADR-0033 vs. ADR-0032 amendment) | ADR-0032's amendment stands: `multi_step` is earnable by evidence. `HARNESS_CONFORMANCE.md` header to be updated. |
| Outbound leg "cut at the container" (ADR-0031 §4) vs. profiles `outbound_channel: true` | Profiles are right; ADR-0031 §4 and conformance item 8 are amended to cite the egress mechanism in `webfetch::egress`. |
| ADR-P7-01…05 cited in code and config but absent from `docs/adr/` | Out of scope here; recorded as a documentation debt item. The web, skill and evidence tools have no written design. |
| `AGENT_HARNESS_GUIDE_v3.md` cited by the conformance report but not in the repo | Same: documentation debt. Its numeric bands survive only as quotations. |
| Memory/skills injection budgets undefined for the harness-assembled path | Deferred to AGENT-003's materialiser work; when built they enter as `Reference` blocks under that section's share. |

## 4. Consequences

- Per-call cost on the frontier tier drops from ~13k static tokens uncached to a cached prefix read; the local tiers' working state grows by the tokens the schemas were silently taking.
- Two profile fields are added and must have conformance readers. `Section::Tools` is deleted.
- The internal driver gains file tools and, later, `run_python`; both are already policy-classified paths, not new trust surfaces.
- The two drivers still have two loops. Sharing the assembler is the step that makes a later unification (ADR-0033's direction) a refactor rather than a redesign.
