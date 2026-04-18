# Human-provided specs

This folder holds **documents you supply** (product intent, external requirements, design notes, stakeholder checklists). They are **not** auto-generated from the codebase.

## Canonical target architecture (APEX)

**[`new_specs/canonical/`](new_specs/canonical/)** holds the **canonical APEX** specifications — the **intended** end-state architecture during the **FB-CAN-*** migration. **Start here:** **[`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD)** (precedence rules and full index). For a short ordering table (canonical vs as-built `docs/Specs/` vs legacy human intent), see **[`../Specs/CANONICAL_PRECEDENCE.MD`](../Specs/CANONICAL_PRECEDENCE.MD)** (**FB-CAN-023**).

Older drafts live under [`new_specs/superseded/`](new_specs/superseded/). Root-level `*.MD` files in **this folder** (e.g. `MASTER_SYSTEM_PIPELINE_SPEC.MD`) remain **legacy human intent** for the pre-canonical pipeline; where they disagree with **APEX canonical** on future design, **canonical wins** for roadmap decisions.

## Purpose

1. **Source of intent** — What you want the system to become or enforce, independent of current implementation (canonical APEX + legacy docs below).
2. **Diff driver** — When an agent (or you) compares these files to **`docs/Specs/`** (as-built) and to the code, gaps become queue items in the **[queue system](../QUEUE_SCHEMA.md)** (`QUEUE_STACK.csv` + `QUEUE_ARCHIVE.MD` per **`docs/QUEUE.MD`** §6).

## Files in this repo

| File | Description |
|------|-------------|
| [`MASTER_SYSTEM_PIPELINE_SPEC.MD`](MASTER_SYSTEM_PIPELINE_SPEC.MD) | **End-to-end** system: runtime order, training, nightly refresh, objects, promotion gates |
| [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) | Full forecaster: VSN, latent CNN, multi-resolution xLSTM, quantiles, conformal, ensemble |
| [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](POLICY_MODEL_ARCHITECTURE_SPEC.MD) | Forecast-conditioned RL policy: observation, target exposure, risk gate, execution plan |
| [`NIGHTLY_TRAINING_AND_REFRESH_SPEC.MD`](NIGHTLY_TRAINING_AND_REFRESH_SPEC.MD) | Nightly forecaster + RL maintenance cadence, seeds, budgets, promotion gates |
| [`INITIAL_OFFLINE_TRAINING_CAMPAIGN_SPEC.MD`](INITIAL_OFFLINE_TRAINING_CAMPAIGN_SPEC.MD) | First offline bootstrap: walk-forward splits, campaign scale, PPO+SAC targets |
| [`TEMPLATE_HUMAN_SPEC.MD`](TEMPLATE_HUMAN_SPEC.MD) | Blank template for new documents |

## Workflow (recommended)

1. Add or edit a file here (use **`TEMPLATE_HUMAN_SPEC.MD`** or your own naming).
2. In chat or a task, ask to **reconcile**: “Compare `Human Provided Specs/<file>` to `docs/Specs/` and the repo; update backlog / issues / code as needed.”
3. After implementation, update **`docs/Specs/`** so as-built docs stay accurate.

## Naming

- Use descriptive names: `PRODUCT_VISION_2026.MD`, `EXECUTION_REQUIREMENTS.MD`, etc.
- Keep **`.MD`** extension to match repo doc conventions.

## Relationship to `docs/Specs/` and canonical APEX

| Folder | Owner | Content |
|--------|--------|---------|
| **`docs/Human Provided Specs/new_specs/canonical/`** | Product / program | **Target** APEX architecture (roadmap contract) — see **[`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD)** |
| **`docs/Specs/CANONICAL_PRECEDENCE.MD`** | Maintained with the code | **FB-CAN-023** — explicit doc hierarchy so **`docs/Specs/`** is not read as the APEX contract |
| **`docs/Specs/`** | Maintained with the code | What **is** implemented **today** (as-built) |
| **`docs/Specs/FORECASTER_AND_POLICY.MD`** | Maintained with the code | Path-by-path map from legacy human specs (§26 forecaster, §27 policy) to `forecaster_model/` and `policy_model/` |
| **`docs/Human Provided Specs/`** (here) | You | Legacy intent + canonical **`new_specs/canonical/`** |

## Do not commit secrets

Treat human-provided files like any other doc: no API keys, passwords, or private URLs with embedded credentials.
