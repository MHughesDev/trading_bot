# Traceability: spec section → evidence

Use this when the spec seems wrong, under-specified, or when you need the numbers behind a threshold.

| Spec § | Topic | Primary reference |
|---|---|---|
| §0.1, §1.1–1.3 | Bitemporal model, identity, PIT read path | `08-market-data-arch.md` |
| §1.4 | Corporate actions, adjustment, index membership | `08-market-data-arch.md` |
| §1.5 | Futures rolls, continuous contracts | `08-market-data-arch.md` |
| §1.6 | Options storage, IV vs greeks, universe gating | `08-market-data-arch.md` |
| §1.7 | Crypto venues, funding, seasonality | `08-market-data-arch.md`, `05-asset-embeddings.md` |
| §1.8 | DeFi pools, reorgs, finality | `08-market-data-arch.md` |
| §2 | Cross-asset alignment, master clock, staleness | `08-market-data-arch.md` |
| §3.1–3.3 | Dataset hashing, feature runtime, consistency logging | `08-market-data-arch.md`, `01-mlops-infra.md` |
| §3.4–3.5, §12.2 | Labels, splits, embargo, purging | `03-financial-ml.md` |
| §4 | Trial ledger, propensity, censoring, fixation | `06-experiment-data.md`, `01-mlops-infra.md` |
| §4.5 | Exploration floor | `02-hpo-search.md`, `07-finetuning-drift.md` |
| §5.2–5.3 | Asset fingerprints, EDGE, portfolios, TSFM caution | `05-asset-embeddings.md`, `02-hpo-search.md` |
| §5.4 | Regimes, filtered vs smoothed | `05-asset-embeddings.md` |
| §5.5 | Outcome Tensor, MNAR, LCB shrinkage, HRP | `05-asset-embeddings.md`, `06-experiment-data.md` |
| §5.6 | Agent memory, insights, injection cap | `04-agentic-ml.md` |
| §6 | Engines, partitioning, cost | `08-market-data-arch.md`, `01-mlops-infra.md` |
| §7 | Multi-tenancy, RLS, feature firewall | `07-finetuning-drift.md`, `08-market-data-arch.md` |
| §8–9 | Control plane, job state machine | `01-mlops-infra.md` |
| §10 | Campaign lifecycle, delta_practical, diminishing returns | `02-hpo-search.md`, `04-agentic-ml.md` |
| §11.1–11.3 | Searcher selection, ASHA hardening, LC stopping | `02-hpo-search.md` |
| §11.4 | Post-hoc pipeline: soup, ensemble, calibration, threshold | `02-hpo-search.md` |
| §11.5 | Statistically defensible comparison | `02-hpo-search.md` |
| §12.1–12.4 | Multi-objective selection, gate stack, N_eff, deflation | `03-financial-ml.md` |
| §12.5 | Leakage suite | `03-financial-ml.md` |
| §12.6 | Paper trading as process test, kill criteria | `03-financial-ml.md` |
| §12.7 | Gate-hacking countermeasures | `03-financial-ml.md`, `04-agentic-ml.md` |
| §13 | Internal model roster, cold-start ladder | `06-experiment-data.md`, `02-hpo-search.md` |
| §14.1–14.4 | Retraining policy, learning debt, self-improvement guardrails | `07-finetuning-drift.md` |
| §14.5 | Judges, PRMs, private eval harness | `04-agentic-ml.md`, `07-finetuning-drift.md` |
| §14.6 | Fine-tuning break-even, trajectory logging | `07-finetuning-drift.md` |
| §15 | Agent tool surface, conventions, approval envelopes | `04-agentic-ml.md` |
| §16 | Streaming, cardinality, platform self-monitoring | `01-mlops-infra.md` |
| §17 | Failure handling | `01-mlops-infra.md` |

**General synthesis and rationale:** `00-research-brief.md`
