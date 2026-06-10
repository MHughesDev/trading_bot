# CI failure — single root-cause analysis (formal prompt)

**Purpose:** When **GitHub Actions** (or local parity) fails **across multiple recent runs**, use this structured prompt—**as-is for agents** or as a checklist for humans—to identify **one** systemic **root cause**, not a list of symptoms.

**Repository context (this repo):**

| Artifact | Role |
|----------|------|
| [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | Primary CI: **`lint-test`** (Ruff, pytest, `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`), **`docker-image`** (Hadolint, `docker build`, smoke import, Trivy fs), optional **`integration-optional`** (`workflow_dispatch`) |
| [`docs/Specs/TESTING_AND_CI.MD`](Specs/TESTING_AND_CI.MD) | Test and CI overview |
| [`AGENTS.md`](../AGENTS.md) §9 | Required checks before merge |

---

## Formal analysis prompt *(copy below)*

```text
You are analyzing a **systemic** continuous-integration failure in the **trading_bot** repository (GitHub Actions workflow `.github/workflows/ci.yml`). Your objective is to identify the **single root cause** that explains **all** or **nearly all** recent failing builds—not a catalog of unrelated errors.

### Constraints
1. **One root cause** — If multiple failures appear, determine whether they share a **common upstream trigger** (e.g. dependency resolution, Python version, runner image, broken `main`, bad cache). State that common cause first.
2. **Evidence-first** — Cite the **first failing step** in the workflow graph, the **first error line** in logs, and the **commit range** or **workflow run IDs** you used.
3. **Scope to this repo’s CI** — Map failures to actual job names: `lint-test` (Ruff → Pytest → Spec compliance → MLflow policy) or `docker-image` (Hadolint → Build → Smoke → Trivy). Do not assume steps that do not exist unless workflows changed.
4. **Exclude red herrings** — Flaky tests isolated to one test file may be a **different** issue unless every run fails the same assertion.
5. **Output shape** — Produce:
   - **Verdict:** One sentence naming the root cause.
   - **Mechanism:** Why that cause produces the observed logs across jobs (if applicable).
   - **Verification:** Exact command(s) or local reproduction mirroring CI (e.g. `pip install -e ".[dev]"`, `ruff check .`, `pytest`, `bash scripts/ci_spec_compliance.sh`).
   - **Fix:** Minimal change (PR-level) or infrastructure action (pin version, rotate secret, branch protection).
   - **Residual uncertainty:** What you could not confirm without more access.

### Inputs you must obtain
- Links or IDs for **at least two** failed workflow runs on **`main`** or the same PR branch.
- Whether **both** `lint-test` and `docker-image` fail, or only one job—this narrows shared vs job-specific causes.

### Do not
- List ten fixes without ranking a single root cause.
- Blame “CI is flaky” without identifying **what** is nondeterministic (network, time, test order).
- Print or repeat **secrets** from logs.
```

---

## Quick reference — job → likely failure classes

| Job | Steps (order) | Typical root-cause buckets |
|-----|----------------|----------------------------|
| **`lint-test`** | install → Ruff → pytest → spec compliance → MLflow policy | **Dev deps / Python**, import policy (`ci_spec_compliance`), test collection, MLflow API grep |
| **`docker-image`** | Hadolint → build → smoke import → Trivy | **Dockerfile**, base image, build context, image scan policy |
| **`integration-optional`** | services + pytest integration | **Service images**, ports, `NM_INTEGRATION_SERVICES`, flaky external timing |

---

## Related

- [`docs/Specs/TESTING_AND_CI.MD`](Specs/TESTING_AND_CI.MD)  
- [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)

**Documentation added:** formalized from an informal “single root cause” request; revision tracked in git.
