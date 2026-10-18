# Phase 4 — Search, the Skills page and SKILL.md procedures

**Completion: 60% (3 / 5 tasks), plus 2 `PARTIAL`.**

**Requirement IDs:** SK-10, UI-06, SK-12
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §6.3, §8
**Depends on:** Phases 1 to 3.
**Migration range:** none.

The part a user and the agent actually touch: finding a skill by meaning, seeing the registry, and the *procedure* skills (instruction bundles) that ADR-0028 and AGENT-007 describe.

---

### RA-4.1 `tbot skills search` over full bodies — M
`PARTIAL` 2026-10-04 — `GET /api/skills/search` (Postgres full-text over whole `SKILL.md` bodies), the `search_skills` MCP tool and `tbot skills search`; the response carries `"mode": "lexical", "embedder": false` and says why (ADR-RA-11, ADR-0049). **Missing:** the dense leg (a query by intent finding a skill that shares no word with it) until a skill body has an embedding.

Hybrid dense + BM25 retrieval with rerank over full skill bodies. **Check the store first:** KM-07 chooses between Milvus and pgvector (RB-1.6); until it does, a BM25-only path over Postgres full-text is the honest v1 and is labelled as such.

**Files:** `crates/api` (outside this track), `apps/tbot`
**Acceptance:** a query by intent finds a skill whose name shares no word with it (when the embedder is present); without an embedder the response says it fell back to lexical.
**Traces:** SK-10; AGENT-003 §8

### RA-4.2 The skills REST surface — M
`DONE` 2026-10-04 — `routes/skills.rs` mounted by `routes/mod.rs`: `GET /api/skills/glossary`, `GET /api/skills/search`, `GET /api/skills/item/{id}` (not `{id}`: a bare parameter beside the Python library's static `/api/skills/run` would shadow its methods), `POST /api/skills/proposals`, `GET /api/skills/proposals`, `POST /api/skills/{id}/vote`, and the admin routes (`quarantine`, `deprecate`, `promote`, `conditions`, `proposals/{id}/pairs|steps|verify|admit`, `curator/*`). An agent token reads the glossary and cannot reach any admin route (`skills_admit_is_never_an_agent_scope`); every handler takes a `BearerToken`, so `every_mounted_route_refuses_an_anonymous_caller` covers the table (10 of 10 pass, and `a_static_route_does_not_shadow_its_parameterised_sibling` passes).

`GET /api/skills/glossary`, `GET /api/skills/search`, `GET /api/skills/{id}`, `POST /api/skills/proposals`, `POST /api/skills/{id}/vote`; promote, deprecate and quarantine are owner and admin only.

**Files:** `crates/api/src/routes` (outside this track; `routes/mod.rs` lines for the orchestrator)
**Acceptance:** an agent token reads the glossary and cannot promote; the route table is covered by `every_mounted_route_refuses_an_anonymous_caller`.
**Traces:** SK-03; AGENT-003 §8

### RA-4.3 UI-06: the Skills page — L
`PARTIAL` 2026-10-05 (R-WIRE) — built, not yet reachable: `frontend/src/pages/SkillsPage.tsx` (Registry, Proposals, Notices), `components/skills/{RegistryTab,ProposalsTab,NoticesTab,SkillDetailPanel}.tsx`, `api/skills.ts` and `lib/skills/view.ts`, over the served routes only: `GET /api/skills/glossary`, `/search`, `/item/{id}`, `/proposals`, `/notices` and `POST /api/skills/proposals/{id}/verify`, `/admit` and `/api/skills/{id}/quarantine`. Registry list with resident skills marked and a search that prints its mode (lexical); a proposal queue whose **admit is off until this view has run verify and the result says admissible** and then goes through a confirmation that needs the typed word `ADMIT`; quarantine needs a reason, is confirmed, and shows the registry's refusal in its own words (the server's `skills.admit` check is the authority); quarantine notices newest first; per-skill usage (exposures, activations, balance; a skill nobody was exposed to is "never exposed", not zero). Server: `glossary` and `search` now return each skill's `skill_id` (additive; the page could list a skill and do nothing to it without it). Tests: `the-skills-client-calls-only-routes-the-server-serves` (every `skillsApi` URL against the router's `.route` table, and no promote/deprecate/curator/vote method), `view.test.ts` (9), `the-skills-page-verifies-and-admits-only-through-confirmation` (8: a read that failed is not an empty registry; admit gated and confirmed; the owed step named; a quarantine the registry accepts re-reads the glossary so the skill leaves the list without a reload); `npx tsc -b --noEmit`, `npm run lint` 0, full `vitest` 114 files / 1058+ tests, `vite build` clean; mutation-checked (offering admit with no verification fails 2 tests). **Remaining:** the page is not routed or in the nav until the orchestrator applies the `App.tsx` lazy import, route and `nav-sections.ts` entry (in the report); promote/deprecate and the curator queue have no controls here by design.

Glossary, skill history, usage statistics and the promotion queue, on Meridian.

**Files:** `frontend/src` (outside this track)
**Acceptance:** every client call is to a served route (`every_client_path_is_served`); a quarantined skill disappears from the glossary view without a reload; lint stays at 0.
**Traces:** UI-06; BS-007 15

### RA-4.4 SKILL.md procedures — M
`DONE` 2026-10-04 — `kind: procedure` in the frontmatter, `skills/core/meta/skill-authoring/SKILL.md` (a "Procedure skills" and "How to propose" section) and `procedure-template.md`, and `registry::check_procedure`: a tool named outside the catalogue (`[tool:name]` checked against `mcp_server_lib::spec::catalog()` by `skills_store::tool_catalogue`) is refused, so is one that reaches the network (`Risk::Outbound`), one with `shell`, `network`, `permissions`, `env`, `allowed-tools` or `hooks` in the frontmatter, and one missing `## Steps`, `## Evidence` or `## Failure`; a clean one is admitted (`a_procedure_naming_an_unknown_tool_is_refused`, `a_procedure_requesting_shell_hooks_or_network_is_refused`, `a_clean_procedure_is_admitted_and_a_missing_section_is_not`).

A *procedure* skill is an instruction bundle (not a Python transformation): ordered steps, the tools it uses, the evidence it produces, and the failure it was written for. It is admitted only
through RA-3.3 and may grant no capability (SK-12). Author the template and the `skill-authoring` guidance to match.

**Files:** `skills/core/meta/skill-authoring/SKILL.md` (additive), `crates/skills/src/lint.rs`
**Acceptance:** a procedure skill that names a tool outside the catalogue is refused; one that requests shell, hooks or network is refused; a clean one is admitted.
**Traces:** SK-12; ADR-0028; AGENT-007

### RA-4.5 provisional becomes admitted after N exposures — S
`DONE` 2026-10-04 — `skills::lifecycle::provisional_verdict` (N exposures with a non-negative balance; below N stays; at N with a negative balance stays and is flagged; N is `SKILLS_ADMIT_AFTER_EXPOSURES`, default 20) and `SkillStore::settle_provisional`, called by `session_start` so no scheduler has to be remembered (ADR-RA-13). `provisional_becomes_admitted_only_at_n_exposures_with_a_non_negative_balance` (19 stays, 20 with a downvote is flagged, the vote flipped promotes, a vote from a session never exposed is refused).

After N exposures with a non-negative helpfulness balance (N is AGENT-003 open question 3; default 20, ADR-RA-04).

**Files:** `crates/api`, `crates/skills/src/registry.rs`
**Acceptance:** a skill below N stays provisional; at N with a negative balance it stays provisional and is flagged; the threshold is configuration.
**Traces:** AGENT-003 §6.3
