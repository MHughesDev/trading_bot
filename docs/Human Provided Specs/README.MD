# Human-provided specs

This folder holds **documents you supply** (product intent, external requirements, design notes, stakeholder checklists). They are **not** auto-generated from the codebase.

## Purpose

1. **Source of intent** — What you want the system to become or enforce, independent of current implementation.
2. **Diff driver** — When an agent (or you) compares these files to **`docs/Specs/`** (as-built) and to the code, gaps become backlog items (e.g. new rows in **`docs/FEATURES_BACKLOG.MD`**) or issue log entries (**`docs/ISSUE_LOG.MD`**).

## Files in this repo

| File | Description |
|------|-------------|
| [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) | Full forecaster: VSN, latent CNN, multi-resolution xLSTM, quantiles, conformal, ensemble |
| [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](POLICY_MODEL_ARCHITECTURE_SPEC.MD) | Forecast-conditioned RL policy: observation, target exposure, risk gate, execution plan |
| [`TEMPLATE_HUMAN_SPEC.MD`](TEMPLATE_HUMAN_SPEC.MD) | Blank template for new documents |

## Workflow (recommended)

1. Add or edit a file here (use **`TEMPLATE_HUMAN_SPEC.MD`** or your own naming).
2. In chat or a task, ask to **reconcile**: “Compare `Human Provided Specs/<file>` to `docs/Specs/` and the repo; update backlog / issues / code as needed.”
3. After implementation, update **`docs/Specs/`** so as-built docs stay accurate.

## Naming

- Use descriptive names: `PRODUCT_VISION_2026.MD`, `EXECUTION_REQUIREMENTS.MD`, etc.
- Keep **`.MD`** extension to match repo doc conventions.

## Relationship to `docs/Specs/`

| Folder | Owner | Content |
|--------|--------|---------|
| **`docs/Specs/`** | Maintained with the code | What **is** implemented |
| **`docs/Human Provided Specs/`** (here) | You | What **should** be true or what **you** assert |

## Do not commit secrets

Treat human-provided files like any other doc: no API keys, passwords, or private URLs with embedded credentials.
