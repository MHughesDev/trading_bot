# Set E — Future Scope (not counted in primary total)

These components compile and are well-built, but have **zero functional
consumers** today. The recommendation is **keep the code, gate the infra, and
revisit when a real consumer use-case exists** — not implement now and not
delete. The only urgent slice (infra gating + the Milvus port bug) is pulled
forward into **Phase 0.5**.

---

## FS.1 — TigerGraph capability graph (`crates/graph`)

- **Purpose.** A capability/compatibility graph: vertices `AssetClass`,
  `Instrument`, `Venue`, `DataType`, `StrategyDefinition`, `Widget`; edges like
  "venue provides data type", "strategy requires data" (`schema.rs:11-49`). Client
  (`connect`/`ping`), idempotent GSQL schema init, and a 350-line `populate.rs`
  all exist.
- **Consumed today?** **No.** `graph` is referenced only in the root
  `Cargo.toml` workspace list; no app/crate depends on it, no binary calls
  `init_schema`/`populate`, zero `graph::` usages. It only compiles.
- **Infra.** Runs a real `tigergraph:3.9.3` container by default
  (`docker-compose.yml:71-79`) + volume — heavyweight, with nothing connecting.
- **Recommendation.** **Defer with an explicit scope note; gate the container
  behind a compose profile (Phase 0.5).** The relational `instruments` table +
  `ScheduleKind::for_asset_class` cover current needs. Don't delete — the design
  is sound.
- **If pursued (L):** build a `graph-populate` binary (Postgres → vertices/edges)
  and a real consumer (MCP discovery compatibility queries, or strategy-validator
  capability checks). Needs a use-case first, not just wiring.
- **Risk of as-is:** carrying cost + drift; a running container implies a
  capability that doesn't exist, with no test against a live instance.

## FS.2 — Milvus vector store (`crates/semantic`)

- **Purpose.** Semantic search over social/web text: client + `MilvusConfig`,
  `trading_social_posts` collection (1536-dim), OpenAI `text-embedding-3-small`
  upsert + `search_similar`. Carries real hardening (deterministic upsert keys,
  char-boundary truncation, closed-set source filter against DSL injection).
- **Consumed today?** **Partially / not functionally.** `apps/embedder` depends
  on it and calls `ensure_collection`, but its event loop is a stub that just
  waits on ctrl-c (`apps/embedder/src/main.rs:55-62`) — `embed_and_upsert` /
  `search_similar` are never called in production, and no API/MCP surface exposes
  search.
- **Infra.** Heavy: `milvus` + required `etcd` + `minio` (3 services,
  `docker-compose.yml:82-126`) + an external **OpenAI** dependency. **Latent
  bug:** `lib.rs:21` defaults to port 19530 while compose/embedder use 9091 —
  a real connection would fail (fix in Phase 0.5).
- **Recommendation.** **Defer with a scope note; gate the 3 containers behind a
  compose profile; fix the port bug regardless (Phase 0.5).** Either finish the
  embedder loop (M) or mark it explicitly dormant — don't leave it half-live
  emitting a false "ready" log.
- **If pursued (M):** finish `apps/embedder`'s subscription loop (subscribe
  `social.post`/`web.page_snapshot` → `EmbeddingRequest` → `embed_and_upsert`),
  add a search surface (API route or MCP tool), reconcile the port.
- **Risk of as-is:** false "ready" signal, latent port bug, paid-API config
  surface for an off feature.

## FS.3 — Minor hygiene (LOW)

- **Config secrets** (`config/default.toml`, CL row): dev defaults
  (`trading:trading`) must move to env vars before any production deploy. Pair
  with the Phase 1.7 network-exposure gate.
- **Ops documentation** (`docs/observability/`, `docs/procedures/user-management.md`,
  CL rows): write runbooks for metrics/alerting and user management once 5.1 and
  Phase 1 land — documentation should follow the implementation, not precede it.

---

## Overall

Neither FS.1 nor FS.2 should remain in the **default-on** infra footprint while
unused. None should be deleted (code quality is high). The concrete near-term
actions are all in **Phase 0.5** (gate infra, fix the Milvus port); the rest
waits for a consuming feature to justify the carrying cost.
