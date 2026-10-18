# ADR-0049: The Retrieval Store Is pgvector Plus Postgres Full-Text; Milvus Stays Deferred

**Status:** Accepted
**Date:** 2026-10-04
**Deciders:** Platform team (decided under Set R-b RB-1.6 by the research-memory track; the owner may reverse it by superseding this ADR)
**Related:** [ADR-0028](0028-skill-registry-and-platform-side-admission.md); AGENT-003 §3 (names Milvus collections `findings`, `skills`, `text`); Set R-b `ADR-RB-01`, `ADR-RB-06`; migrations 0055, 0122, 0125, 0226.

## Context

AGENT-003 §3 names a hybrid (dense plus BM25) vector store with three Milvus collections.
Nothing deploys Milvus: the platform's migrations already require the Postgres `vector`
extension (`0055_knowledge_plane`, `0122_semantic_memory`, `0125` all create `vector(n)`
columns), the insight store ranks by `embedding <=> $query` today, and the skill registry's
search (`routes/skills.rs`) already runs on Postgres full-text and says so in its response.
The corpus is small: findings and insights in the low thousands, skills in the dozens to low
hundreds, on a single-tenant platform.

## Decision

1. **Dense retrieval stays in Postgres with `pgvector`.** Exact nearest-neighbour search with
   metadata filters, as `0055` already prescribes ("deliberately NO ANN index; add HNSW only
   on a measured p99 failure").
2. **The lexical leg is Postgres full-text** (`to_tsvector` / `ts_rank` over the full body).
   The hybrid query is a rank fusion of the two legs over one table.
3. **Every stored vector names its model and dimension** (migration 0226: `embedding_model`,
   `embedding_dim`, enforced by a trigger), and retrieval compares only vectors of one model.
   A model change re-embeds.
4. **No embedding request leaves the host.** `OllamaEmbedder` refuses a non-loopback endpoint
   before connecting (`is_local_endpoint`, with a test).
5. **Milvus remains deferred.** Revisit on a *measured* failure: recall below a stated floor
   on a held-out query set, or p99 latency above budget at the corpus size then current.

## Rationale

One fewer service to run, back up and secure on a single-tenant platform; the database already
holds the data the vectors describe, so a filter on `kind`, `subject` or `tainted` is a `WHERE`
clause and not a cross-store join; and the migrations have already committed the platform to
the extension. A distributed vector store is a scale answer to a scale problem this corpus does
not have.

## Consequences

- A development database without `pgvector` cannot run the dense leg or apply 0055 / 0122
  unmodified (the scratch Postgres on 5433 is one). The lexical leg and every rule in
  `skills`, `research` and `harness::memory` work without it, and the tests that need the
  extension say so.
- `skills search` answers `"mode": "lexical"` until a skill body has a stored embedding; adding
  the dense leg is a change behind `skills::registry::similarity`'s signature and
  `routes/skills::search`, not a new store.
- The production database needs `CREATE EXTENSION vector`, which it already has.

## Alternatives Considered

- **Deploy Milvus as AGENT-003 names.** A second stateful service, a second place `tainted`
  and `archived` must be enforced, and a hybrid index to keep in step with Postgres rows, for a
  corpus that fits in memory.
- **A third-party hosted vector database.** Sends the platform's research text off the host;
  ruled out by KM-06 (embeddings are local).
- **Lexical only.** Honest for v1 and what ships today for skills, but it cannot find a
  procedure by intent when no word is shared; kept as the fallback, not the end state.

## References

- [AGENT-003](../specs/AGENT-003-knowledge-memory-skills.md) §3, §4
- Set R-b `phase-1-knowledge.md` RB-1.5, RB-1.6
