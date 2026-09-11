# BS-007 / 12 — Knowledge and memory (and what embeddings represent)

**Spec status:** spec-ready draft · **Req prefix:** KM- · **Formal spec:** AGENT-003 ·
**Plan set:** R (core pieces in L) · **Decisions:** D-03, D-06 · **Related:** 13_SKILLS
(procedures), research/R1 §3 (memory techniques)

---

## 1. Short answer

**Use embeddings, but narrowly.** Embeddings index *text-shaped knowledge retrieved by
meaning*:
- past research findings;
- news, social posts and filings;
- skill bodies.

They are the wrong store for numbers, time series or anything that must be exact or
point-in-time correct. Those stay in Postgres or ClickHouse, retrieved by key or query.

**One embedding represents one self-contained, citeable unit:** a finding, a skill, or a
document, written so the agent can use it without opening the source. Never a transcript
chunk, raw tool JSON, or a table of numbers.

**Reference knowledge is files first, not a vector store.** For a coding agent,
skills plus grep beat vector retrieval-augmented generation (RAG) on a corpus this size.

## 2. Memory layers

| Layer | Contents | Source of truth | Retrieval | Embeddings |
|---|---|---|---|---|
| **L0 Core** | System core + generated CLAUDE.md: identity, behaviour contract (02 §4), protocol, workspace map, compaction instructions | Versioned in the repo; generated per project version | Always resident, cached (04 §3) | No |
| **L1 Reference** | Quant concept cards, platform how-tos, archetype library, QC rules (11 §7) | **Core skills** (13), seeded from research/R3 and 11 | Skill descriptions resident; bodies on demand; glossary; grep | Skill bodies are indexed for `tbot skills search` |
| **L2 Research memory** | Findings, hypothesis verdicts, rejected ideas, final reports, across sessions | Postgres | Structured filters first (instrument, asset class, timeframe, archetype, gate), then semantic | Yes, over each finding's `claim` |
| **L2m Move memory** | Which structural moves worked in which contexts (AlphaMemo-style) | Postgres | By (archetype, regime profile, gate) → move | No (structured) |
| **L2p Playbook** | Strategic bullets with helpful/harmful counters (ACE) | Skill registry, `kind: playbook` (13 §2) | As a core/global skill | Via skills search |
| **L3 Market knowledge** | Instrument profiles: stylized facts, `data_qc`, regime history, cost model, coverage, fitted processes | Postgres/ClickHouse, recomputed by deterministic jobs | By key (`tbot profile <instr>`) | No; analog search uses explicit vectors (§7) |
| **L4 External text** | News, social, web, filings | ClickHouse/Postgres + Milvus | PIT-filtered semantic search, reader scope only (06 §7) | Yes |
| **L5 User memory** | Preferences, watchlists, risk tolerance, style | Postgres KV | By key | No |

## 3. L2 findings (the most valuable memory)

A finding is an atomic, cited claim written when an evaluation step ends. The Stop hook
requires one per tested hypothesis (B-07).

```yaml
id: fnd_01J…
claim: "On BTC-USD 1h (2023-01..2026-03), vol-targeted EMA trend family failed G3:
        DSR 0.81 (N_eff 14 of 37 trials); losses concentrated where Hurst(1h) < 0.47."
instrument: BTC-USD  asset_class: crypto_spot_cex  timeframe: 1h
window: [2023-01-01, 2026-03-31]   archetype: trend.ema_cross   gate_reached: G2
evidence: [exp_…, study_…, art_…]   class: result | exploration | model_output
platform_capabilities: { features_engine: v2, exits: true, costs: edge_v1 }
created_at, supersedes: null, project_id, user_id, visibility: project | user | global
```

- **Retrieval:** filter by metadata before vector search. Show age, and flag findings
  whose window is stale or whose `platform_capabilities` predate a relevant change (for
  example, before exits existed).
- **Negative results are first-class.** "Already tried; failed because X" saves the most
  compute.
- **Cross-session honesty:** past findings inform *hypothesis generation* only. They are
  never evidence in a new Experiment. Re-testing a prior hypothesis on the same data is
  flagged as a repeated trial, and a hypothesis whose source is memory records it
  (`source: memory:<fnd_…>`, 11 §2).
- **Global findings** require the dossier path (11 §5) and owner consent.

## 4. L2m move memory

Store edges, not end states:
- `(parent archetype, regime profile, gate reached)`;
- `move` (a typed SLv2 AST-diff motif);
- the **residual outcome** on the sealed metric vs a base prior;
- n and variance.

**Confidence gating** mutes sparse memories. An **asymmetric veto** lets reliable
negative patterns block proposals, while positive patterns only nudge. For example:
"adding an RSI filter to trend families on crypto 1h failed 9/9 times" blocks that move.
This complements FEAT-003's regime → strategy prior table.

## 5. L2p playbook

The diagnostic → move playbook (research/R3 Part E) and the pitfall list become one
versioned playbook skill per category, with id'd bullets and helpful/harmful counters
kept in the registry. The Curator proposes *delta* updates (add, edit, retire bullets)
from structured session deltas. Deltas are admitted through the skill path (13 §4). The
file is never regenerated wholesale (this avoids context collapse).

## 6. Embeddings and store

- **Model:** a **local** embedding model (D-03) served by the inference sidecar or a
  local server. `apps/embedder` switches from OpenAI `text-embedding-3-small`. Record
  `embedding_model` and `dim` per vector; re-embed on model change. Pick the model with a
  small retrieval eval over findings and skills (open question 1).
- **Store:** Milvus (exists). Separate collections for `findings`, `skills` (full
  bodies; 13 §5) and `text` (L4), each with metadata fields for filtering. Hybrid dense
  plus BM25 (Milvus 2.4+) with reciprocal-rank fusion.
- **Rerank:** a small cross-encoder over the top ~30 for findings and skills.

## 7. L3 instrument profiles and analog search

- **Profiles** are deterministic `instrument_profile` jobs (05), not LLM work:
  - stylized facts (tails, clustering, Hurst, variance ratios);
  - `data_qc`;
  - the EDGE cost model;
  - coverage;
  - regime history (filtered);
  - fitted processes.
  They're recomputed on a schedule and when data changes.
- **Analogs** ("when did the market last look like this?"): kNN over an explicit,
  standardised vector of Hurst (returns, log-RV), variance ratios, RV percentile,
  vol-of-vol, trend strength, autocorrelation, drawdown state and volume z-score. There is
  a time-exclusion zone. Learned embeddings (TS2Vec etc.) are a later experiment and must
  beat this baseline on "does the analog's future predict ours?"

## 8. Curation and hygiene

- The **Curator** sub-agent (03 §9) runs as sleep-time work during long job waits, or as
  offline Batch work. It reads **structured deltas only** (new findings, skill
  bill-of-materials rows, verdicts), never raw transcripts. It has a token cap per run.
- Duplicate findings are merged by claim similarity plus identical evidence. Superseded
  findings keep a pointer.
- Memory writes from sessions that read untrusted text are tagged `tainted`, and are
  excluded from global promotion (13 §6).

## 9. What not to do

- Don't embed transcripts, tool JSON or number tables.
- Don't let retrieved text act as instructions. News, posts and even past findings are
  data, wrapped as quoted material.
- Don't let memory replace measurement. A retrieved "BTC is trending" from last month is
  a prompt to recompute, not an answer.

## 10. Requirements

| ID | Requirement | Pri |
|---|---|---|
| KM-01 | Findings (§3) MUST be written through `tbot memory add`, validated (evidence ids exist), and required per tested hypothesis | MUST |
| KM-02 | Retrieval MUST filter by metadata before semantic search, and MUST show staleness and capability flags | MUST |
| KM-03 | Findings MUST NOT be accepted as evidence in new Experiments; repeated hypotheses MUST be flagged | MUST |
| KM-04 | Move memory (§4) SHOULD be recorded for every structural move, with a confidence-gated veto | SHOULD |
| KM-05 | The playbook MUST live in the skill registry and change only through admitted deltas | MUST |
| KM-06 | Embeddings MUST use a local model, with model and dim recorded per vector | MUST |
| KM-07 | Milvus collections `findings`, `skills` and `text` MUST support hybrid dense + BM25 retrieval | MUST |
| KM-08 | Instrument profiles MUST be deterministic jobs; analog search MUST use the explicit vector with a time-exclusion zone | MUST |
| KM-09 | The Curator MUST read structured deltas only, under a token cap | MUST |
| KM-10 | Memory derived from untrusted text MUST be tagged `tainted` | MUST |

## 11. Acceptance criteria

1. A second session on the same instrument retrieves the first session's rejected
   hypotheses and doesn't re-propose them without a flag.
2. No research text leaves the machine: the embedding calls are local, verified by an
   egress test.
3. The analog search returns neighbours outside the exclusion zone with a stated
   similarity basis.

## 12. Open questions

1. Which local embedding model (quality on finance text vs size)? Decide with a 50-query
   retrieval eval.
2. Visibility defaults for findings in a multi-user setup (00 cross-cutting question 3).
