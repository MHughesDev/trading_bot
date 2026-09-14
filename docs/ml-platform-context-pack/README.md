# ML Platform Context Pack
### Data Architecture & MLOps for a multi-tenant quantitative trading platform

**Version 1.0 · 2026-09-13 · Prepared for a coding agent**

---

## What this is

A self-contained context pack. It contains one **normative specification**, a set of **non-negotiable invariants**, an **implementation backlog**, **executable schema DDL**, and the **research source material** the spec was derived from.

It exists so a coding agent can go into an existing codebase and change the MLOps layer and data architecture to match, with full traceability back to evidence.

## Read in this order

| Order | File | Why |
|---|---|---|
| 1 | `CLAUDE.md` | How to use this pack. Operating rules for the agent. Read fully before touching code. |
| 2 | `decisions/INVARIANTS.md` | 24 things that must be true. Violating one is a build failure, not a preference. |
| 3 | `spec/SPEC.md` | The normative spec (~12,000 words). This is what you implement. |
| 4 | `backlog/IMPLEMENTATION-CHECKLIST.md` | Ordered, checkboxed work items mapped to spec sections. |
| 5 | `backlog/ACCEPTANCE-TESTS.md` | The tests that must exist and pass. Several block the build by design. |
| 6 | `schemas/*.sql` | Executable DDL extracted from the spec. |
| 7 | `reference/` | Source-of-truth research. Consult when the spec seems wrong or under-specified. |

## Authority order

When two documents disagree:

```
INVARIANTS.md  >  SPEC.md  >  reference/*  >  your own judgment
```

**Exception:** if `reference/` contains evidence that contradicts `SPEC.md`, do **not** silently follow the reference. Stop, write the conflict into `decisions/OPEN-QUESTIONS.md`, and ask. The spec was written *after* the research and may already have accounted for it.

## The one-paragraph summary

Build a model-development environment, not a training button. A human or authorized agent poses an objective; the system runs a long-horizon, multi-session campaign of controlled experiments and converges on a candidate that is *demonstrably* better than baseline — where "demonstrably" means statistically defensible under the **true number of trials the system has run**. Every configuration, run, failure, rejection and decision lands in an immutable, propensity-logged Trial Ledger. That ledger is the platform's primary asset: the training corpus for internal models that progressively take over the platform's own judgment.

## Scope

**In scope:** the data plane (L0 market data → L1 datasets/features → L2 trial ledger → L3 knowledge plane), and the MLOps layer (orchestration, job state machine, search, evaluation gates, internal models, retraining policy, agent tool surface, observability).

**Out of scope:** the trading engine, broker integration, order routing, general application stack, billing, auth UI. The spec touches UI only to state what the MLOps layer must expose.

## Provenance

Derived from 7 deep research passes (~72,000 words, ~500 cited sources) conducted September 2026, plus a synthesis brief. Sixteen spec decisions **reverse** the initial brief; each is listed with its evidence in `decisions/CHANGED-FROM-BRIEF.md`. Those reversals are the highest-value content in this pack — they are the places where the obvious answer is wrong.

## File map

```
README.md                              you are here
CLAUDE.md                              agent operating instructions
MANIFEST.json                          machine-readable index

spec/
  SPEC.md                              normative specification

decisions/
  INVARIANTS.md                        24 must-be-true statements
  CHANGED-FROM-BRIEF.md                16 evidence-backed reversals
  ADR-INDEX.md                         architecture decisions + rejected alternatives
  OPEN-QUESTIONS.md                    write conflicts and blockers here

backlog/
  IMPLEMENTATION-CHECKLIST.md          phased, checkboxed, spec-linked
  ACCEPTANCE-TESTS.md                  required tests, several build-blocking

schemas/
  01_identity.sql                      instruments, symbols, venues
  02_market_data.sql                   bars, corporate actions, futures, options, chain
  03_datasets.sql                      feature/label/split specs
  04_ledger.sql                        trial ledger, decisions, trajectories
  05_knowledge.sql                     embeddings, regimes, outcome tensor, insights
  06_tenancy.sql                       RLS policies and the feature firewall

reference/
  00-research-brief.md                 synthesis + rationale
  01-mlops-infra.md                    orchestration, tracking, versioning, resources
  02-hpo-search.md                     HPO, multi-fidelity, meta-learning, statistics
  03-financial-ml.md                   backtest validity, leakage, gates
  04-agentic-ml.md                     agent architecture, memory, tools, verifiers
  05-asset-embeddings.md               vector spaces, outcome tensor mathematics
  06-experiment-data.md                experiment data as training data
  07-finetuning-drift.md               fine-tuning, drift, retraining, multi-tenancy
  08-market-data-arch.md               multi-asset minute-bar data engineering
```
