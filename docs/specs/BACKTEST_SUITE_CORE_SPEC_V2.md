# Backtest Suite — Core Spec v2: Research-Standard Extensions

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 2.0-draft
**Extends:** [BACKTEST_SUITE_CORE_SPEC.md](BACKTEST_SUITE_CORE_SPEC.md) (v1, implemented
by Set J). Everything in v1 stays in force: Run/Study/Experiment, INV-1/2/3, the Null
Library, the Gate 0→4 funnel, `SelectionRule`, DSR/PBO corroborators.
**ADR(s):** ADR-0019/0020/0021 (kept), ADR-0023 (P1), ADR-0025 (platform-side
enforcement), ADR-0030 (counting at job submission)
**Derived from:** BS-007
[11_EVALUATION_STANDARD](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD);
evidence in research/R2 and the sources below
**Plan set:** Q (Research standard), parallel with M; EV-09 in R-b; EV-11 in Track
**Crates:**
- `crates/backtest` (`experiment/`, `gates/`, `stats/`, new `hypothesis/`, `dossier/`,
  `family/`);
- `crates/research` (random arm, effective N);
- `crates/api` (`/api/hypotheses`, `/api/backtest/experiments/*` extensions);
- `migrations/0043_research_standard.sql`.

---

## 0. What changes, in one table

| Area | v1 (Set J) | v2 |
|---|---|---|
| Holdout | Vault on the Experiment | + the Data API research cutoff; the vault gate is the only post-cutoff reader (DATA-005 §4) |
| Pre-registration | `primary_test` on the Experiment | + a **hypothesis registry** required by `create_experiment` (§1) |
| Counting | Trial counter per Experiment | + model trials; **effective N**; **specification count**; **exploration ledger**; **`post_hoc`** flag (§2) |
| Run kinds | Definition | + `position_series` (FEAT-004), both counted identically |
| Gate 0 | Close-stamped leak scan | + truncation test; prediction-series overlap; `data_qc` grade; planted-violation CI (§4) |
| Gate 3 | Primary null + selection-bias correction; DSR ≥ 0.95 & PBO ≤ 0.5 corroborators | + DSR on effective N; **theme-posterior alpha**; **family test** for campaigns (§5) |
| Vault entry | G3 passed | + a complete **12-part dossier** (§6) |
| Forward | Reconciliation live-vs-backtest | + **forward paper gate** once G-11 is wired (§8) |
| State | In-memory suite and sweep stores | Job service + Set K stores (COMP-005) |

## 1. Hypothesis registry

```sql
-- migrations/0043_research_standard.sql
CREATE TABLE hypotheses (
  hypothesis_id TEXT NOT NULL,               -- 'hyp_' || ULID; edits create a new version row
  version INT NOT NULL DEFAULT 1,
  project_id UUID NOT NULL, user_id UUID NOT NULL, session_id UUID,
  archetype TEXT NOT NULL, statement TEXT NOT NULL,
  mechanism TEXT NOT NULL, counterparty TEXT NOT NULL, why_persists TEXT NOT NULL,
  falsifier TEXT NOT NULL, kill_criterion TEXT NOT NULL,
  null_kind TEXT NOT NULL, null_rationale TEXT NOT NULL,        -- Null Library entry + preserves/destroys reasoning
  alternatives_considered JSONB NOT NULL DEFAULT '[]',
  causal_graph JSONB,                         -- optional DAG {nodes, edges, roles}
  source TEXT NOT NULL,                       -- original | literature:<ref> | memory:<fnd_…>
  instruments TEXT[] NOT NULL, timeframe TEXT NOT NULL, variables TEXT[] NOT NULL,
  post_hoc BOOLEAN NOT NULL,                  -- computed at registration (§2.4), immutable
  post_hoc_evidence JSONB,                    -- ledger rows that triggered it
  data_qc_grade TEXT NOT NULL, qc_waiver_id UUID,
  status TEXT NOT NULL DEFAULT 'untested',    -- untested|running|rejected|supported|vaulted
  registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (hypothesis_id, version)
);
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS hypothesis_id TEXT;
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS hypothesis_version INT;
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS project_id UUID;
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS window_json JSONB;     -- immutable
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS specification_count INT NOT NULL DEFAULT 0;
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS effective_n DOUBLE PRECISION;
ALTER TABLE backtest_experiments ADD COLUMN IF NOT EXISTS dossier_handle TEXT;
```

**API:** `POST /api/hypotheses` (scope `research:experiments`; CLI
`tbot exp hypothesis register`).

**Validation:**
- non-empty mechanism, counterparty, why_persists, falsifier and kill criterion
  (length ≥ 20 chars each);
- `null_kind` is a Null Library entry;
- the `data_qc` grade for (instruments, timeframe, window) is ≥ B, or C with an approved
  waiver (DATA-005 §7);
- in a **Desk** project, registration is allowed, but the resulting Experiments are
  capped at G2.

**`create_experiment` now requires `hypothesis_id`.** It copies the window (immutable)
and the primary test from the hypothesis's `null_kind`. Changing the window means a new
Experiment, which counts as a new trial source. A repeated hypothesis (same archetype,
instruments, timeframe and overlapping window as a prior rejected hypothesis, or
`source=memory:` pointing at one) is flagged `repeated` in the verdict.

## 2. Counting

### 2.1 Trials (INV-1, unchanged semantics; new entry point)

Every evaluation-counted job increments `trial_counter` in the job-insert transaction
(COMP-005 §10), whatever the client. That covers:
- Definition and PositionSeries backtests;
- sweep and study members;
- gate advances.

Model `train`/`hpo` trials increment `model_experiments.trial_count` (FEAT-006 §9).

### 2.2 Effective number of trials

- **Inputs:** the return series of every counted run in the Experiment, aligned on the
  in-sample window.
- **Method (default):**
  1. hierarchical clustering on the correlation distance `d = sqrt(0.5·(1 − ρ))`, with
     average linkage;
  2. `N_eff` = the number of clusters at threshold `d* = 0.3` (configurable, recorded in
     the verdict);
  3. cross-check: the eigenvalue estimate `N_eff' = (Σλ)² / Σλ²`.
  Report both, and use the larger (more conservative) value in DSR.
- Alternatives listed in `alternatives_considered` add 1 each to `N_eff` (the
  parallel-universe rule), unless they were actually run.
- **Used by:** DSR (`expected_max_standard_gaussian(N_eff)`) and the selection-bias
  correction's trial input. The raw trial count is always reported beside it.

### 2.3 Specification count

Incremented whenever a new strategy version or dataset in the Experiment adds a feature,
filter, transform or interaction beyond its parent. It is computed from AST and
DatasetSpec diffs (FEAT-004 §6, FEAT-006 §3), reported, and used in the protocol
checklist. It is not an automatic penalty.

### 2.4 Exploration ledger and `post_hoc`

At hypothesis registration, the service queries `exploration_ledger` (COMP-005 §9) for
the user's rows (including Desk rows) that:
- predate registration;
- touch the same instruments;
- overlap the window;
- and intersect `variables`.

If any exist, `post_hoc = true`, with those rows as evidence. `post_hoc` doesn't block
testing. It is shown on every verdict and in the dossier, and the report validator
requires a caveat when it's true.

## 3. Protocol rules (Arnott–Harvey–Markowitz) → mechanisms

| # | Rule | Mechanism | Tag |
|---|---|---|---|
| 1a | Ex-ante economic foundation | §1 required fields | E |
| 1b | Beware ex-post stories | §2.4 `post_hoc` | E |
| 2a | Track everything and the correlation between tries | INV-1 + §2.2 | E |
| 2b | Count interactions | §2.3 | E |
| 2c | Parallel universes | `alternatives_considered` → N_eff | A+E |
| 3a | Fix the sample ex ante | Immutable `window_json` | E |
| 3b | Data quality first | `data_qc` gate (§1) | E |
| 3c | Decide transformations in advance; results survive small changes | Declared in DatasetSpec/params; the robustness suite perturbs them (§7) | E |
| 3d–e | No arbitrary outlier exclusion; winsorisation fixed pre-model | Part of the declared spec; a change is a new specification | E |
| 4a–b | Iterated OOS is in-sample | Cutoff + one-shot vault | E |
| 4c | Costs | Cost models on by default (FEAT-005 §5) | E |
| 5a | Structural change | RegimeConditional study required before G3; sub-period stability in the robustness suite | A+E |
| 5b | Crowding and decay | `expected_decay`, `capacity_report` in the dossier | A |
| 5c | Don't tweak a live model | Immutable deployed versions (FEAT-004 §4) | E |
| 6 | Simplicity, regularisation, interpretability | `simplify` move before G3; complexity penalty in proposal scoring; driver explanation for model-based candidates | A+E |
| 7 | Reward quality, not discoveries | Agent eval suite scores honest nulls as success (AGENT-004) | E |

## 4. Gate 0 additions (Integrity)

Gate 0 fails with a specific reason if any of the following holds:
- **Truncation test** (Layer 1): any pre-truncation position difference (FEAT-004 §2.3).
- **Prediction-series overlap:** any consumed prediction with
  `train_end ≥ t − embargo` (FEAT-005 §4.3).
- **`data_qc`:** grade D, or grade C without a waiver.
- **Unsafe flags:** any `UnsafeFlags` set (exists), including `costs_disabled`.
- **Revised data:** a run whose `data_snapshot` includes revisions made available after
  the decision times they affect, when the policy is `as_of` (DATA-005 §3).
- **Version mismatch** (FEAT-005 §6).

**Auditor CI** (AGENT-004 §4): each check has violating and clean fixtures. The target is
100% caught and 0 false rejections.

## 5. Gate 3 (Significance) v2

**Pass requires all of:**
1. **Primary null** p-value, after selection-bias correction with the **trial counter
   and N_eff** (report both), below α (default 0.05). This is unchanged in form.
2. **DSR ≥ 0.95** computed with N_eff (§2.2). Exists, with N_eff substituted.
3. **PBO ≤ 0.5** (exists).
4. **Theme-posterior alpha** (§5.1) > 0, with its 90% credible interval excluding 0.
5. **Family test** (§5.2) passed when the Experiment belongs to a campaign with ≥ 2
   candidates reaching G3.
6. **Regime-conditional study** present (rule 5a).

The verdict records every component: the raw and effective trial counts, the null used,
and the thresholds.

### 5.1 Theme-posterior alpha (empirical Bayes; Jensen–Kelly–Pedersen)

- **Prior:** a normal prior on per-period alpha for the candidate's `archetype` theme,
  estimated by empirical Bayes from:
  - `findings` and verdicts in the user's research memory (AGENT-003) for the same
    archetype and asset class;
  - the null-hypothesis distribution of the archetype's historical random-arm results;
  - with a floor on prior variance.

  With fewer than `k_min` (default 5) related results, fall back to a skeptical default
  prior centred at 0 with a declared variance.
- **Likelihood:** the candidate's walk-forward out-of-sample alpha (vs its benchmark)
  with a HAC standard error.
- **Output:**
  - posterior mean;
  - 90% credible interval;
  - shrinkage factor;
  - the list of prior sources (ids).

`POST /api/backtest/experiments/{id}/posterior` (`tbot exp posterior`).

### 5.2 Family tests (data-snooping-robust)

- **Methods:** `stepwise_spa` (default), `spa` (Hansen) and `romano_wolf`.
- **Inputs:** the net-of-cost return series of every candidate in the campaign family at
  G3, against the benchmark (buy-and-hold or the declared benchmark).
- **Procedure:** stationary bootstrap (reusing the null generator), B=2,000.
- **Output:** the adjusted p-value per candidate. Candidates failing the family test
  can't enter the vault.

## 6. Dossier (vault entry requirement)

`POST /api/backtest/experiments/{id}/dossier` (`tbot exp dossier build`) assembles a
`dossier` artifact. `…/funnel/advance` to G4, and the `final_report` validator (for
`gate_reached ≥ G3` candidates), refuse without a complete dossier. The 12 parts, with
their sources:

| # | Part | Source |
|---|---|---|
| 1 | Hypothesis card, registration time, `post_hoc`, `repeated` | §1 |
| 2 | `data_qc` grade, waiver, as-of universe or instrument-selection note | DATA-005 §7–8 |
| 3 | Specification count, trials, N_eff, exploration-ledger count | §2 |
| 4 | Walk-forward result; CPCV path distribution (median, worst-5%, share positive) | Set J studies |
| 5 | Primary null p, DSR, PBO, theme-posterior alpha | §5 |
| 6 | Matched-budget random-search comparison | §7 `compare_to_random` |
| 7 | Cost breakdown, net metrics, capacity, turnover | FEAT-005 §5; §7 `capacity_report` |
| 8 | Sized vs unsized at matched average risk; left-tail metrics (worst 1% bar, ES, max DD) | §7 |
| 9 | Crypto market-factor beta; return decomposition (vol-timing / drift / selection) | §7 |
| 10 | Regime-conditional and sub-period stability; transformation perturbations | Robustness suite |
| 11 | Protocol checklist (§3) pass/flag; alpha-translation-chain audit | §7 |
| 12 | Expected live haircut; forward-test plan | §7 `expected_decay` |

## 7. Evaluation services

| Endpoint (`/api/backtest/experiments/{id}/…`) | CLI | Returns | Priority |
|---|---|---|---|
| `robustness` `{preset}` | `tbot exp robustness` | Cost ladder (×0.5, ×1, ×2, ×3), delay-by-one-bar, sub-periods (thirds), neighbourhood, transformation perturbations, regime-conditional; submitted as studies (counted) | High |
| `compare` `{other}` | `tbot exp compare` | Sealed-distribution comparison (median, worst-5%, overlap); never point estimates | High |
| `benchmark` `{kind}` | `tbot exp benchmark` | Buy-and-hold, random-entry-matched, simple momentum, **vol-timed momentum** | High |
| `capacity` | `tbot exp capacity` | Net expectancy vs order size under √-impact + EDGE; break-even size | High |
| `alpha-chain` | `tbot exp alpha-chain` | 5 stages pass/fail/unknown with evidence | High |
| `posterior` | `tbot exp posterior` | §5.1 | High |
| `family-test` (campaign) | `tbot exp family-test` | §5.2 | High |
| `vs-random` (campaign) | `tbot exp vs-random` | Agent vs shadow random arm on the sealed metric | High |
| `decompose` | `tbot exp decompose` | Vol-timing / drift / selection components; intercept always included | Medium |
| `factors` | `tbot exp factors` | Crypto market/size/momentum betas and alpha (Liu–Tsyvinski–Wu construction from the platform universe) | Medium |
| `collider` | `tbot exp collider` | Filters plausibly caused by both signal and future returns | Medium |
| `decay` | `tbot exp decay` | McLean–Pontiff-style haircut prior by source | Medium |

**Random-search shadow arm:** every campaign (FEAT-003 Phase 2) reserves a fraction of
its backtest budget (default 20%, matched) for uniform random structures and params from
the same grammar and space. Arm runs are counted in their own Experiment family. The
campaign report shows agent vs random on the sealed metric.

## 8. Forward paper gate (Track; after G-11)

Once live strategy evaluation is wired:
- a vaulted candidate may be proposed for paper deployment (approvals inbox);
- **Gate 5 (Forward)** compares the paper-fill P&L distribution over N days with the
  backtest's sealed distribution for the same period, using the reconciliation module;
- pass if the paper distribution lies within the backtest's central 90% band on the
  pre-registered metric.

## 9. QC rules by topic (normative summary)

These are enforced where tagged. Full rationale and sources are in BS-007 11 §7.

| T | Rule |
|---|---|
| T2 | DSR **and** positive theme posterior (§5) |
| T3 | Filters added by moves declare a causal role; `collider` flags need justification |
| T4 | Forecast features need OOS R² > 0 with Clark–West significance (FEAT-006 §7) |
| T5 | Complex models must beat vol-timed momentum (`benchmark`) |
| T6 | Equal-weight combination unless estimated weights beat it OOS |
| T7 | Vol models must beat HARQ on QLIKE |
| T8 | Sizing compared at matched risk, with left-tail metrics |
| T10–T11 | EDGE-based costs; capacity reported |
| T14 | Crypto market beta reported |
| T18 | Expected live haircut stated |
| T19 | As-of universes; instrument-selection bias acknowledged |
| T20 | ES backtests before sizing on ES/VaR |

## 10. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| E1 | `create_experiment` without a hypothesis → `422`; a window change creates a new Experiment | EV-02 |
| E2 | 20 near-duplicate sweep members → N_eff ≪ 20; DSR uses N_eff; both counts reported | EV-03 |
| E3 | A hypothesis registered after exploring the same variables is `post_hoc` with evidence | EV-03 |
| E4 | Position-series and definition runs increment the same counter; HPO increments the model counter | EV-04 |
| E5 | The auditor fixtures: 100% caught, 0 false rejections | EV-05 |
| E6 | G4 without a complete dossier → refused, listing the missing parts | EV-06 |
| E7 | On the pure-noise suite, the G3 pass rate is ≤ nominal + 2 pp with v2 rules | EV-07 |
| E8 | A campaign report shows the random-arm comparison and family-test results | EV-08, EV-09 |
| E9 | Suite, sweep and funnel state survive a restart | EV-10 |

## 11. Open questions

1. The N_eff clustering threshold `d*` (default 0.3): calibrate on the synthetic suite.
2. Theme-prior construction when research memory is sparse: accept the skeptical default
   prior? Recommended: yes.
3. The random-arm budget fraction (default 20%).

## Sources

Arnott–Harvey–Markowitz (2019); Jensen–Kelly–Pedersen (JF 2023); López de Prado et al.
(causal factor investing); Goyal–Welch–Zafirov (RFS 2024); Hansen (SPA); Hsu–Hsu–Kuan
(step-SPA); Romano–Wolf; Bailey–López de Prado (DSR, PBO); McLean–Pontiff (JF 2016);
Liu–Tsyvinski–Wu (JF 2022); Acerbi–Szekely; arXiv 2608.25348 (agent vs random search
under PIT audit); arXiv 2609.04917 (alpha-translation chain). Full links are in BS-007
[11_EVALUATION_STANDARD](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD).
