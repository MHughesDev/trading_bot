# Spec precedence (FB-CAN-023)

**Purpose:** Prevent **`docs/Specs/*.MD`** from being read as the **APEX target architecture**.

| Precedence | Location | Use when |
|------------|----------|----------|
| **1 — Target / roadmap** | [`docs/CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) → [`docs/Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/) | Defining **intended** end-state behavior, migration slices (**FB-CAN-***), governance |
| **2 — As-built** | **`docs/Specs/*.MD`** (this folder) | What the **code does today** — module maps, entry points, CI |
| **3 — Legacy human intent** | [`docs/Human Provided Specs/`](../Human%20Provided%20Specs/README.MD) root `*.MD` (not under `new_specs/canonical/`) | Historical product intent; **canonical wins** on conflicts for future design |

Gap analysis: [`docs/reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](../reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md).
