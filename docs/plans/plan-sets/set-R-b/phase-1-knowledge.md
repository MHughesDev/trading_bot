# Phase 1 — Knowledge and memory (KM-02 to KM-10)

**Completion: 56% (5 / 9 tasks), plus 4 `PARTIAL`.**

**Requirement IDs:** KM-02…KM-10
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §3–§7; [BS-007 12_KNOWLEDGE_MEMORY](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/12_KNOWLEDGE_MEMORY.MD)
**Depends on:** nothing inside this set.
**Migration range:** 0213.

KM-01 (findings through a validated write path) is Set S Phase 4. The rest of the knowledge requirements split into things Set S partly built (retrieval, tainting, instrument profiles, the local embedder) and things nobody has: move memory
with a veto, the curator, and the vector store the spec names. Several rows below are therefore *verify* tasks: read the code, find what exists, and close or re-scope with evidence rather than build a second copy.

---

### RB-1.1 Retrieval filters by metadata first and shows staleness and capability flags (KM-02) — S
`PARTIAL` 2026-10-04 — verified against `GET /api/knowledge/insights` (`knowledge_jobs::search_insights`) and found **short of the acceptance**: it embedded the query *before* it ran any metadata filter, and returned no age, staleness or capability flag. Fixed: the structured filter (`tier`) runs first and `knowledge_routes::should_issue_semantic_query` says whether a semantic query is worth issuing (never when the filter matched nothing; `semantic_skipped` in the response), the query targets one embedding model, and every row now carries `age_days`, `stale` (older than 90 days), `capability_mismatch` (`null`, not `false`, when the caller did not say what it has; `?capabilities=` and `scope.requires`), `missing_capabilities` and `contested` (`harness::memory::rules::annotate`, `annotate_insights`). Tests: `a_filter_that_excludes_everything_issues_no_semantic_query`, `the_insight_handler_asks_before_it_embeds` (source order), `age_staleness_and_capability_flags_are_present_on_every_annotation`. **Missing:** a run of the handler against a database: `knowledge.insight` needs pgvector, which the scratch server lacks, so the SQL is compile-checked and not executed here.

Verify against `GET /api/knowledge/insights` and the S-4.5 retrieval log: the structured filters run before semantic ranking, the response carries age, a staleness flag and a capability-mismatch flag.

**Files:** `crates/api/src/knowledge_*.rs` (read; another track's)
**Acceptance:** a retrieval with a metadata filter that excludes everything returns nothing without a semantic query being issued; staleness and capability flags are present on every row.
**Traces:** KM-02; AGENT-003 §5

### RB-1.2 Findings are never evidence; repeated hypotheses are flagged (KM-03) — S
`DONE` 2026-10-05 (R-WIRE) — the rules and a store (`harness::memory::rules`, `agent_memory.hypothesis_source`, `POST /api/knowledge/hypothesis-sources`, `tag_hypothesis_source`) were already there; what was missing was the callers. `knowledge_routes::finding_evidence_refusal` is now the one refusal (422 `finding_is_not_evidence`, naming the findings) and is called by `routes/lifecycle::state_hypothesis` and `routes/experiments::create_experiment`, each of which accepts an `evidence_ids` list and refuses a finding in it before anything is written (ADR-RW-10). Tests: `a_finding_offered_as_evidence_is_refused_by_every_route_that_takes_evidence` (the shared refusal's status and body, and that the three routes call it, read from source); earlier: the pg round trip for repeats and the three rule unit tests. **Residual, stated rather than hidden:** stating a hypothesis through `state_hypothesis` does not require a source tag, so a repeat is flagged only for hypotheses tagged through `hypothesis-sources`; the acceptance (a finding as evidence is refused; a repeat is flagged and counted) holds on both routes that exist.

A hypothesis whose `source=memory:<fnd>` on the same instrument and window is flagged a repeated trial.

**Files:** `crates/api`, `crates/ledger` (read)
**Acceptance:** an experiment citing a finding as evidence is refused; a repeated hypothesis is flagged and counted.
**Traces:** KM-03; AGENT-003 §5

### RB-1.3 Move memory with a confidence-gated veto (KM-04) — M
`DONE` 2026-10-05 (R-WIRE) — the pure parts were built 2026-10-04 (`research::move_memory`, 0213, the veto/records/overrides routes, `check_move_history`). The campaign driver now reaches them: `CampaignWorker::consult_veto` runs in `advance` **before** an EXPERIMENT child is created (a block nobody overrode returns `move_vetoed` citing the records and appends nothing, so the driver resumes at the same position once `POST /api/knowledge/moves/overrides` is recorded; a warn rides in the EXPERIMENT event's detail as `move_veto`), and `record_move_outcomes` writes one `agent_memory.move_record` per judged generation, idempotent on `run_ref = campaign:<id>:<COMPARE seq>`, after every COMPARE and once at driver start so a driver killed between a COMPARE finishing and the write loses nothing (ADR-RW-01, ADR-RW-02). Tests (scratch Postgres, `campaign_wiring`, 7 passing): `a_campaign_with_no_arm_records_nothing_and_a_blocked_move_stops_until_a_person_overrides` (allow, then five recorded failures block and cite `old:0`, then a person's override lets it proceed and says so), `each_judged_generation_records_what_its_move_did_once` (a rejected candidate is a `failed` move; the second call writes 0), `a_generation_with_no_typed_move_is_not_vetoed`; unit: `a_comparison_is_attributed_to_the_hypothesize_that_preceded_it`, `a_comparison_says_what_the_move_did_and_only_a_rejection_is_a_failure`, `advance_consults_the_veto_before_dispatch_and_records_the_outcome_after_compare`. Mutation-checked: disabling the block makes the veto test fail.

Record the canonical AST-diff motif of every structural move (FEAT-004 §6) with its outcome; before a move is submitted, a veto fires when the motif has a record of failure at a stated confidence. Advisory below the confidence, blocking above it.

**Files:** `crates/research` (additive), `migrations/0213_move_memory.sql`
**Acceptance:** a motif with enough failures vetoes a move and says why; below the confidence it only warns; the veto cites the records.
**Traces:** KM-04; AGENT-003 §3

### RB-1.4 The playbook lives in the skill registry (KM-05) — S
`DONE` 2026-10-04 — the playbook is a registry skill (`scope=core`, loaded by `sync_core`) and changes only through admitted deltas: `app_role` cannot write `skill`/`skill_version` at all (grant test), the mount is read-only and `write_file` cannot reach it, and a *patch proposal* for `playbook` goes through lint, trigger, coexistence, replay and Reviewer steps and a person's `skills.admit` before it becomes a new version whose parent is the old one; the release loading its own older bytes never overwrites it (`propose_verify_and_admit_with_every_refusal_naming_its_step`, last block; `core_sync_is_idempotent_and_a_changed_bundle_is_a_new_version...`).

Changes only through admitted deltas. Blocked on Set R-a's registry (RA-1.1, RA-1.2).

**Files:** `skills/core/research/playbook/`
**Acceptance:** a playbook edit outside the proposal path is refused.
**Traces:** KM-05; AGENT-003 §6.4

### RB-1.5 Embeddings are local and carry model and dimension (KM-06) — S
`PARTIAL` 2026-10-04 — verified `OllamaEmbedder` and found two gaps, fixed: (1) a remote `OLLAMA_URL` sent claim text off the host: `is_local_endpoint` + a refusal before any connection (`an_embedding_request_never_leaves_the_host`: loopback, `localhost`, `::1` and the Docker host alias pass; remote, private-range, look-alike hosts such as `127.0.0.1.evil.example` and `user@evil.example` are refused, and the error names the policy rather than a network failure); (2) no vector named its model: migration 0226 adds `embedding_model` and `embedding_dim` with a trigger refusing a vector without them (`a_stored_vector_names_its_model_and_dimension_and_a_legacy_row_stays_updatable`, run on a scratch Postgres; a legacy row stays updatable), `embed_missing` records both and re-embeds a row whose model differs, and `search_semantic_for_model` ranks only vectors of the query's model. **Missing:** the embedding and search SQL against a pgvector database (not available here): compile-checked, not executed.

Verify `OllamaEmbedder` (`nomic-embed-text`, 768 dims) records model and dim per vector and that no embedding request leaves the host (the egress test).

**Files:** `crates/api/src/knowledge_jobs.rs` (read)
**Acceptance:** a vector row names its model and dim; a re-embed on a model change creates a new version and queries target one version; an egress test fails if a request leaves the host.
**Traces:** KM-06; AGENT-003 §4

### RB-1.6 Hybrid retrieval store (KM-07): decide Milvus or pgvector — L
`PARTIAL` 2026-10-04 — **decided: ADR-0049, stay on pgvector plus Postgres full-text; Milvus remains deferred** (ADR-RB-06; revisit on a measured recall or latency failure). Built toward it: `embedding_model`/`embedding_dim` on both vector tables (0226), the lexical leg live in `GET /api/skills/search`. **Missing:** the hybrid query over findings with filters (dense plus lexical rank fusion over `agent_memory.entry`), which needs a pgvector database to build and test.

AGENT-003 names Milvus collections (`findings`, `skills`, `text`) with hybrid dense + BM25. The platform's local Postgres has no pgvector (verified 2026-10-03) and no Milvus is deployed. **Human step:** decide
the store (ADR-RB-01 recommends pgvector plus Postgres full-text, which needs `CREATE EXTENSION vector` on the production database) and approve the dependency.

**Files:** `crates/semantic`, `docs/adr` (new)
**Acceptance:** the chosen store serves a hybrid query over findings with filters; the collection records `embedding_model` and `dim`.
**Traces:** KM-07; AGENT-003 §3

### RB-1.7 Instrument profiles and analog search (KM-08) — M
`DONE` 2026-10-04 — `research::analog` (the explicit `profile_vector` with six named statistics, a pool standardiser, `analogs` with the exclusion zone) over the stored `knowledge.instrument_fingerprint` vectors through `knowledge_routes::analog_search`, `GET /api/knowledge/analogs`, the `find_analog_windows` tool and `tbot knowledge analogs`. As-of bounded: nothing known after `as_of` is returned. `analog_search_over_stored_fingerprints_never_returns_the_same_episode_or_the_future` (scratch Postgres, migration 0184): the query's own window, the same instrument inside one window length, later fingerprints and the future are excluded; another market at the same moment is an analog; an instrument with no fingerprint is a typed absence. Unit: `the_query_window_and_its_neighbours_are_never_returned`, and a mutation case (a negative horizon removes the guard and the overlap comes back first).

Profiles are deterministic jobs (built: `InstrumentProfileWorker`). Add analog search: kNN over the explicit standardised vector with a **time-exclusion zone** equal to the window's horizon, so the nearest analog is never the same episode.

**Files:** `crates/api/src/knowledge_jobs.rs` (read), `crates/research` (additive)
**Acceptance:** the query's own window is excluded from its results; an analog inside the exclusion zone is never returned; the vector is the explicit one, not an embedding.
**Traces:** KM-08; AGENT-003 §7

### RB-1.8 The curator (KM-09) — L
`DONE` 2026-10-05 (R-WIRE decision, ADR-RW-11) — `harness::memory::curator` (an input type with no field that can hold prose; a token cap with the cut reported; output is `CuratorItem` proposals), `skill_registry.curator_item` (0225), `POST /api/skills/curator/run` (`skills.admit`), `GET .../items`, `POST .../items/{id}/decide`. Tests: `the_curators_input_cannot_hold_a_sentence_from_a_session`, the cap/cut report, and a source scan that the module has no store, filesystem or network handle. Every acceptance clause is met. The run is started by a person (route or CLI) and **not on a timer**: a scheduled run would only add unreviewed items to a queue that waits for a person, so the schedule is left out deliberately rather than missing.

A sleep-time sub-agent or offline batch that reads **structured deltas only** (findings, `skill_runs`, verdicts) under a token cap and proposes playbook deltas and merges of look-alike skills through the proposal path.

**Files:** `crates/api` (outside this track), `crates/harness`
**Acceptance:** the curator's input contains no free text from a session; a run over the cap is cut and reports it; its output is a proposal, never a write.
**Traces:** KM-09; AGENT-003 §6.4

### RB-1.9 Untrusted-derived memory is tagged `tainted` (KM-10) — M
`PARTIAL` 2026-10-04 — taint is `extra.tainted` on an entry (`Entry::mark_tainted`, ADR-RB-10); `local_driver` marks every finding a run writes after it consumed a `web` tool's result (and a resumed run scans its log for one); `MemoryStore::for_subject` and `by_kind` exclude tainted entries by default, `*_including_tainted` and the browser (`routes/memory.rs`, with a `tainted` field per row) see everything, and the conflict guard sees everything; `SkillStore::cite_finding` reads the mark **from the store** and a tainted provenance refuses promotion even with all six conditions recorded (`the_default_retrieval_excludes_tainted_entries_and_the_browser_still_sees_them`, `promotion_to_global_needs_six_recorded_conditions_and_a_tainted_finding_blocks_it`). **Missing:** the driver's marking is not run end to end (a `Pump` over a real tool result is outside what a unit test can build).

A finding is tainted if the session consumed reader extractions or web text; a tainted finding never reaches a global skill (RB-2.1) and is excluded from the default retrieval.

**Files:** `crates/api` memory store (outside this track)
**Acceptance:** a session that read a fetched page writes a tainted finding; the default retrieval excludes it; promotion of a skill whose provenance includes a tainted finding is refused.
**Traces:** KM-10; AGENT-003 §5
