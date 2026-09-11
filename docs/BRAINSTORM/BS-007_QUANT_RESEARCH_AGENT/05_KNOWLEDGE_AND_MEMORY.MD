# BS-007 / 05 — Knowledge and memory (and what embeddings should represent)

Status: draft proposal.

> **Updated by 08 §9 and 12:** L1 reference cards now ship as **core skills** (files,
> with descriptions in context and bodies loaded on demand), not a vector store first. A
> seventh store, the **skill registry** (procedures, including the ACE playbook as
> `kind: playbook`), sits beside L2. It's searched with hybrid BM25 plus local embeddings
> over full skill bodies. Skills hold procedures only; findings stay in L2 (12 §2).

---

## 1. Short answer

**Yes, use embeddings, but narrowly.** Embeddings are the right index for *text-shaped
knowledge whose retrieval is semantic*: concept explanations, past research findings,
news and social posts. They are the wrong store for numbers, time series, or anything that
must be exact or point-in-time correct. Those belong in Postgres/ClickHouse, retrieved by
key or query.

**What one embedding should represent:** one self-contained, citeable unit, meaning a
concept, a finding, a hypothesis verdict or a document, written so the agent can use it
without opening the source. Never a transcript chunk, never raw tool JSON, never a number
table.

## 2. Six memory layers

| Layer | Contents | Source of truth | Retrieval | Embeddings? |
|---|---|---|---|---|
| **L0 Core prompt** | Identity, behaviour contract, phase protocol, invariants, tool-group map, top pitfalls | `prompt.rs` (versioned) | Always in context, prompt-cached | No |
| **L1 Reference library** | Quant concept cards (06), platform how-to cards (the split authoring guide), strategy archetypes, diagnostic→move playbook | Versioned files in repo → loaded into a table at boot | `search_knowledge` hybrid (BM25 + vector) → `get_card` | Yes |
| **L2 Research memory** | Hypotheses, findings, verdicts, rejected ideas, insights, final reports, across all sessions | Postgres (FEAT-003 §9 knowledge-base tables + findings) | Structured filters first (instrument, asset class, timeframe, archetype, gate), then semantic | Yes, over the text rendering of each finding |
| **L3 Market knowledge** | Instrument profiles: stylized facts, regime history, cost estimates, data coverage, fitted processes | Postgres/ClickHouse, recomputed on a schedule | By key (`get_instrument_profile`) | No; analog search uses explicit feature vectors |
| **L4 External text** | News, Reddit, web pages | ClickHouse/Postgres raw + Milvus vectors (exists for social/web) | PIT-filtered semantic search (`available_time ≤ as_of`) | Yes |
| **L5 User memory** | Preferences, watchlists, risk tolerance, style, prior sessions list | Postgres KV | By key | No |

## 3. L1 card format

```yaml
id: vol.yang_zhang
kind: concept            # concept | howto | archetype | playbook | pitfall
title: Yang–Zhang volatility estimator
aliases: [YZ vol, open-high-low-close volatility, drift-robust range volatility]
summary: >              # embedded; 1–3 sentences
  OHLC volatility estimator that combines overnight, open-to-close and Rogers–Satchell
  variances; drift-robust and efficient; handles opening gaps.
body: |                 # returned by get_card, not embedded
  formula, when to use, pitfalls …
tool_hooks: [volatility_report]
related: [vol.parkinson, vol.garman_klass, vol.rogers_satchell]
platform_status: "not yet in crates/features (Parkinson and GK are)"
version: 1
```

- **Embed `title + aliases + summary`, not the body.** Formulas and tables embed poorly and
  add noise; aliases carry the paraphrases users actually type.
- Scale: ~300–800 cards. At that size BM25 alone works well; embeddings add paraphrase
  recall ("how jumpy is it" → `jumps.bipower_variation`). Use both, merged with reciprocal
  rank fusion.
- Cards carry `platform_status`, so the agent knows whether a concept is computable today
  or only knowledge.

## 4. L2 finding format (the most valuable memory)

A finding is an atomic, cited claim written at the end of an evaluation step:

```yaml
id: fnd_01J…
claim: "On BTC-USD 1h (2024-01..2026-06), EMA-cross trend family failed Gate 3:
        DSR 0.41 after 37 trials; losses concentrated in months with Hurst(1h) < 0.47."
instrument: BTC-USD
asset_class: crypto_spot_cex
timeframe: 1h
window: [2024-01-01, 2026-06-30]
archetype: trend.ema_cross
gate_reached: G2
evidence: [exp_…, study_…, art_…]
class: result            # result | exploration | model_output
created_at: …
supersedes: null
```

- **Embed the `claim` text.** Filter on metadata before vector search. A finding about
  BTC 1h should outrank a semantically closer one about AAPL daily when researching BTC.
- **Staleness.** Findings carry their data window. Retrieval shows age and flags findings
  whose window ended long ago or whose platform capability has since changed (e.g. before
  exits existed).
- **Negative results are first-class.** "Already tried, failed because X" is what saves
  the most compute.
- **Cross-session honesty.** Past findings inform *hypothesis generation* only. They never
  count as evidence in a new Experiment, and reusing a prior hypothesis on the same data
  is flagged as a repeated trial.

## 5. L3 analog search without learned embeddings

"When did the market last look like this?" is a real quant question. Answer it with an
explicit vector per window: standardised Hurst (returns, log-RV), variance ratios, RV
percentile, vol-of-vol, trend strength, autocorrelation, drawdown state, volume z-score.
Use kNN with a time-exclusion zone (no neighbours within the window's own horizon).

Why not learned time-series embeddings (TS2Vec etc.) first: explicit features are
interpretable ("similar because low vol and trending"), need no training, and are easy to
make PIT-correct. Learned embeddings are a later experiment that must beat the explicit
baseline on "does the analog's future predict ours?"

## 6. L4 news and social: point-in-time rules

- Store `published_at`, `ingested_time` and `available_time`. Every search takes `as_of`
  and filters `available_time ≤ as_of`. Revisions are new versions, never overwrites.
- Near-duplicate collapse by embedding similarity. Novelty = 1 − max similarity to the
  entity's items in the prior N days.
- **LLM contamination:** an LLM or embedding model trained after the backtest date may
  "know" outcomes. Label text-derived features with the model's knowledge cutoff; treat
  pre-cutoff backtests of text features as contaminated; prefer forward paper evidence.
  Entity anonymisation is a partial control (Glasserman & Lin 2023 study look-ahead bias
  in GPT sentiment signals).

## 7. Embedding model and store

- **Store:** Milvus already exists (`crates/semantic`, `apps/embedder`). Put L1, L2 and L4
  in separate collections with metadata fields for filtering. Milvus 2.4+ supports hybrid
  dense + sparse (BM25) search.
- **Model:** today it's OpenAI `text-embedding-3-small` (1536-d) for social/web. For L1/L2,
  recommend supporting a **local** embedding model (served via Ollama or the inference
  sidecar) so research notes never leave the machine and the agent works offline. Record
  `embedding_model` and `dim` per vector; re-embed on model change.
- **Rerank:** for L2, a small cross-encoder or LLM rerank over the top ~30 is worth it,
  because findings are high-value and few.

## 8. What not to do

- Don't embed transcripts. They're long, repetitive and full of superseded reasoning. Write
  findings instead.
- Don't embed tool JSON or numeric tables. Query them.
- Don't let retrieved text act as instructions. News, posts and even past findings are
  data. The harness wraps them as quoted material.
- Don't let memory replace measurement. A retrieved "BTC is trending" from last month is a
  prompt to call `regime_report`, not an answer.
