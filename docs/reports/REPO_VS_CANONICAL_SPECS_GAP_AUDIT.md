# REPO vs Canonical Specs Gap Audit

Date: 2026-04-13  
Scope: hard repo-to-canonical-spec audit for full architectural replacement  
Canonical source of truth: `docs/Human Provided Specs/new_specs/canonical/`

## 1) Repository inventory (verified)

### Runtime/entrypoints
- Live runtime: `app/runtime/live_service.py`
- Shared decision step (live + replay): `decision_engine/run_step.py::run_decision_tick`
- Replay runtime: `backtesting/replay.py`
- API/control plane entrypoint: `control_plane/api.py`
- Execution orchestration entry: `execution/service.py`

### Current package surface
- Core: `app`, `data_plane`, `decision_engine`, `risk_engine`, `execution`, `backtesting`, `control_plane`, `models`, `observability`, `orchestration`
- Additional ML modules: `forecaster_model`, `policy_model`
- Support: `services/` microservice bridge modules, `infra/`, `scripts/`, `docs/Specs/`, `tests/`, `legacy/`

### Current architecture summary (code-verified)
1. Live path is Kraken ingest -> features -> `DecisionPipeline` -> `RiskEngine` -> `ExecutionService` (with optional QuestDB persistence and runtime bridge).
2. Decision path is centered on `ForecastPacket`, `PolicySystem`, `DecisionPipeline`, and `run_decision_tick`.
3. Risk path is hard-gate based: stale feed/timestamp, spread, drawdown, mode block, exposure, reduce-only/flatten-all.
4. Execution service is thin and primarily routes risk-approved `OrderIntent` to adapter submit.
5. Config surface (`AppSettings` + `default.yaml`) is broad but legacy-shaped (routing/risk basic thresholds, model file paths, runtime toggles), not canonical domainized config.
6. Queue was previously empty sentinel; now populated with canonical migration program IDs `FB-CAN-001..014`.

## 2) Canonical target inventory

Canonical docs evaluated:
- `APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md`
- `APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md`
- `APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md`
- `APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md`
- `APEX_Execution_Logic_Detail_Spec_v1_0.md`
- `APEX_State_Regime_Logic_Detail_Spec_v1_0.md`
- `APEX_Canonical_Configuration_Spec_v1_0.md`
- `APEX_Replay_and_Simulation_Interface_Spec_v1_0.md`
- `APEX_Monitoring_and_Alerting_Spec_v1_0.md`
- `APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`
- `APEX_Research_Experiment_Registry_Spec_v1_0.md`

Canonical domains required in runtime and governance:
1. Data normalization + feature contracts
2. State/regime/degradation engine
3. Trigger/timing engine
4. Candidate generation + opportunity auction
5. Risk/sizing/exposure with edge budget semantics
6. Execution confidence/style/stress/partial-fill feedback loop
7. Carry sleeve domain
8. Replay/simulation event-contract fidelity
9. Monitoring/alerting domain families
10. Config versioning + release gating lifecycle
11. Research experiment registry

## 3) Domain-by-domain gap analysis

### A. Data ingestion / normalization
- **Aligned:** Kraken-first market data path and normalization layers exist in `data_plane/ingest`.
- **Partial:** Current contracts are not structured as canonical Decision Service input families (market snapshot / structural signal snapshot / safety snapshot / execution feedback snapshot / config snapshot).
- **Gap:** Missing explicit canonical snapshot schema and confidence/freshness normalization at contract boundaries.

### B. Feature engineering
- **Aligned:** Polars feature pipeline exists (`data_plane/features/pipeline.py`) and is reused live/replay.
- **Partial:** Feature families and reliability/freshness metadata are not fully surfaced as canonical contract fields.
- **Gap:** No canonical feature-schema compliance layer from `APEX_Decision_Service_Feature_Schema...`.

### C. State / safety / regime
- **Aligned:** Some regime output exists, but derived via forecast packet mapping.
- **Conflict:** Canonical spec requires explicit state engine with trend/range/stress/dislocated/transition probabilities, transition probability, heat/reflexivity, novelty, and degradation logic.
- **Gap:** No standalone canonical state engine module.

### D. Forecasting / structure
- **Aligned:** Existing forecaster/policy stacks and model artifacts infrastructure.
- **Conflict:** Legacy `ForecastPacket + PolicySystem` control-flow is primary; canonical expects broader leverage-flow structure outputs integrated with state/trigger/auction.
- **Gap:** Canonical structure outputs not represented as first-class contracts.

### E. Trigger / timing
- **Conflict:** No canonical explicit three-stage trigger engine (`Setup -> Pre-Trigger -> Confirmed Trigger`) in current decision flow.
- **Gap:** Trigger confidence/suppression/missed-move logic absent as standalone component.

### F. Opportunity auction
- **Conflict:** Current path typically emits one proposal from policy route; no canonical candidate auction scoring/penalty/top-N selection.
- **Gap:** Entire auction layer missing.

### G. Risk / sizing / exposure
- **Aligned:** Hard safety gates exist and should be retained as overrides.
- **Conflict:** Canonical risk model includes position inertia, degradation multipliers, concentration/thesis awareness, asymmetry caps, edge budget behavior.
- **Gap:** Current risk implementation is materially narrower.

### H. Execution
- **Aligned:** Adapter routing + intent gating foundation exists.
- **Conflict:** Canonical execution logic requires confidence estimation, style selection, stress mode, worst-case edge checks, partial-fill reconciliation, feedback loop.
- **Gap:** These behaviors are mostly absent from `execution/service.py` and adjacent modules.

### I. Carry sleeve
- **Gap:** No explicit carry sleeve subsystem mapped to canonical domain semantics.

### J. Replay / simulation
- **Aligned:** Shared decision path between live and replay already exists.
- **Conflict:** Replay interface lacks canonical event families and replay run contract (config_version/logic_version/fault profile/shadow comparison modes).
- **Gap:** Deterministic contractized replay outputs are incomplete.

### K. Monitoring / alerting
- **Aligned:** Prometheus metrics and basic observability stack exist.
- **Conflict:** Canonical monitoring domains and metric families (decision quality, trigger health, auction quality, drift, governance, replay-shadow divergence) are incomplete.
- **Gap:** Significant metric and alert coverage missing.

### L. Config / release gating
- **Conflict:** Current settings are operationally useful but not canonical-domain structured and not governed by immutable config lifecycle stages.
- **Gap:** Missing release-object lifecycle, promotion gates, and evidence-link workflows.

### M. Research experiment registry
- **Gap:** No canonical registry with required metadata lifecycle tying experiments to release candidates and evidence artifacts.

### N. Docs/specs
- **Conflict:** `docs/Specs/*.MD` describes current/legacy architecture as implemented; canonical specs are separate under `docs/Human Provided Specs/new_specs/canonical/`.
- **Gap:** repo docs do not yet globally declare canonical docs as sole architecture truth.

### O. Tests / CI
- **Aligned:** CI exists with lint/test/security/spec checks.
- **Conflict:** CI gates do not enforce canonical contracts/release-gating evidence; many tests target legacy architecture behaviors.
- **Gap:** Need canonical contract suites and CI gates replacing obsolete assertions.

## 3.1 Missed-items pass (canonical coverage tightening)

Additional canonical items identified as previously under-enumerated and now queued as FB-CAN-031..040:

1. Missed-move handling + false-positive memory penalties (trigger/auction linkage).
2. Per-signal confidence/decay families + feature-family enable flags.
3. Hard override taxonomy + degradation transition occupancy/count logging.
4. Candidate-correlation and thesis clustering penalties in auction.
5. Execution feedback memory loop influencing future decision quality.
6. Canonical decision-record/suppression/safety override output contracts.
7. Fault-injection profile requirements for simulation and promotion evidence.
8. Shadow-environment divergence thresholds + probation windows.
9. Drift/calibration monitoring families and escalation alerts.
10. Canonical glossary / old-to-new naming map to remove ambiguity.

## 3.2 Final completeness pass (full replacement enumeration)

A further completeness pass identified additional queue slices FB-CAN-041..060 so canonical requirements are fully enumerated across:
- regime vector math + transition confidence,
- novelty/heat/reflexivity propagation,
- trigger stage telemetry and latency,
- auction throughput/top-N saturation controls,
- canonical sizing stack details and thesis-aware concentration controls,
- execution style policy and partial-fill residual logic,
- structural signal family ingestion (funding/OI/basis/liquidation/divergence),
- options-context + stablecoin proxy feature families,
- release-object schema + environment-stage enforcement + rollback validation,
- experiment predeclared-metric validation,
- replay mode/event-family coverage,
- monitoring-domain CI coverage assertions,
- immutable config semantic diffing,
- removed-module/docs tombstone index,
- runtime cutover guard and final shim prune.

## 3.3 Detail-closure pass (canonical edge cases and governance completeness)

A final detail-closure pass added FB-CAN-061..070 to cover remaining canonical specifics:
- required config metadata fields and validation,
- canonical common contract conventions (UTC/confidence/freshness semantics),
- stable suppression/safety reason taxonomy,
- carry-sleeve and governance monitoring domain families,
- evidence-package schema for release decisions,
- magic-constant guardrails in canonical paths,
- replay deterministic seeding/provenance metadata,
- post-release probation-window abort policies,
- machine-readable canonical spec section-to-queue coverage matrix.

## 3.4 Remaining-detail pass (operational traceability and acceptance closure)

An additional pass added FB-CAN-071..078 to close remaining operational/canonical traceability details:
- service configuration snapshot contract binding per decision/replay record,
- processing lag/event lag/cycle-duration monitoring detail,
- weekend/low-liquidity occupancy propagation,
- exchange-risk/data-integrity safety inputs,
- execution latency/reliability quality terms in confidence,
- edge-budget proxy monitoring thresholds,
- immutable run identifier bindings (config/logic/dataset/seed),
- automated canonical acceptance audit as migration close gate.

## 4) Starting findings verification summary

1. Top-level package inventory: **confirmed**.  
2. Live/runtime path (Kraken -> features -> decision -> risk -> execution): **confirmed**.  
3. Decision architecture centered on `ForecastPacket`/`PolicySystem`/`DecisionPipeline`/`run_decision_tick`: **confirmed**.  
4. Risk engine hard-gate pattern: **confirmed**.  
5. Execution service relatively thin vs canonical execution logic: **confirmed**.  
6. Settings/config surface legacy-shaped vs canonical domains: **confirmed**.  
7. Queue previously empty sentinel: **confirmed and now changed** (queue items added in this update).  
8. Existing old/current architecture specs in `docs/Specs/`: **confirmed**.

## 5) Exact delete / create / refactor plan

### 5.1 Exact deletion candidates (post-replacement)
Delete once canonical replacements are active:
- `docs/Specs/SYSTEM_OVERVIEW.MD`
- `docs/Specs/DECISION_PIPELINE.MD`
- `docs/Specs/RISK_ENGINE.MD`
- `docs/Specs/EXECUTION_LAYER.MD`
- `docs/Specs/APP_CONFIG_AND_CONTRACTS.MD`
- `docs/Specs/MASTER_SPEC_RISK_STATE_GAP.MD` (if superseded by this audit + canonical mapping)
- Legacy-only decision abstractions in `decision_engine/` tied solely to `ForecastPacket -> PolicySystem -> single proposal` flow (module-level deletion list to be finalized during FB-CAN-004..006)
- Legacy-only risk branches in `risk_engine/engine.py` that become redundant under canonical risk architecture
- Legacy-only execution wrappers in `execution/` that bypass canonical execution-confidence/stress/partial-fill logic
- Obsolete tests asserting removed behavior under `tests/` after canonical equivalents are implemented

### 5.2 Exact modules/docs to create
- `docs/CANONICAL_SPEC_INDEX.MD` (source-of-truth and precedence)
- `app/contracts/canonical/` (or equivalent) for canonical snapshot/input/output contracts
- `decision_engine/state_engine.py`
- `decision_engine/trigger_engine.py`
- `decision_engine/auction_engine.py`
- `execution/confidence.py`
- `execution/style_selector.py`
- `execution/partial_fill_reconciler.py`
- `risk_engine/sizing.py` (or integrated canonical risk model module)
- `orchestration/release_registry.py` (config/logic release lifecycle)
- `models/experiment_registry/` (canonical experiment record model + persistence)
- `tests/canonical/` suites for contracts/domains
- `docs/reports/CANONICAL_MIGRATION_CUTOFFS.MD` (explicit module retirement checkpoints)

### 5.3 Exact refactor targets
- `decision_engine/run_step.py` (switch from legacy direct pipeline outputs to state->trigger->auction->risk->execution-confidence flow)
- `decision_engine/pipeline.py` (replace or split into canonical domain modules)
- `risk_engine/engine.py` (canonical risk/sizing constraints)
- `execution/service.py` (expand beyond submit-only behavior)
- `app/config/settings.py` and `app/config/default.yaml` (canonical config domains)
- `backtesting/replay.py` (canonical replay run contract + event family outputs)
- `observability/metrics.py` and dashboards under `infra/` for canonical metrics
- `.github/workflows/ci.yml` and `scripts/` gates to enforce canonical checks

## 6) Migration risks

1. **Dual-path drift risk:** keeping both legacy and canonical engines simultaneously increases divergence and hidden behavior.
2. **Config migration risk:** breaking environment compatibility without staged mapping can stall runtime.
3. **Replay parity risk:** without strict event contracts, live/replay mismatch can increase during transition.
4. **Execution safety risk:** replacing execution logic without preserving signing/intent gates could weaken safety controls.
5. **Test coverage risk:** removing legacy tests before canonical replacements are ready may create blind spots.
6. **Operator doc risk:** stale docs can cause incorrect operational assumptions during rollout.

## 7) Ordered implementation sequence (hard replacement)

1. FB-CAN-001: publish this audit and precise migration map.
2. FB-CAN-002: canonical docs adoption and explicit de-canonicalization of conflicting docs.
3. FB-CAN-003: canonical config schema + immutable versioning scaffolding.
4. FB-CAN-004: state/regime engine.
5. FB-CAN-005: trigger engine.
6. FB-CAN-006: auction engine.
7. FB-CAN-007: risk replacement.
8. FB-CAN-008: execution replacement.
9. FB-CAN-009: replay/simulation contract alignment.
10. FB-CAN-010: monitoring/alerting domain completion.
11. FB-CAN-011: release gating + experiment registry.
12. FB-CAN-013: tests/CI canonical gate replacement.
13. FB-CAN-012: hard delete obsolete architecture paths.
14. FB-CAN-014: final repo structure + docs cleanup.

## 8) Queue mapping (IDs -> domains)

- FB-CAN-001: Audit baseline + migration map
- FB-CAN-002: Docs/spec authority
- FB-CAN-003: Config/release-object schema base
- FB-CAN-004: State/regime/degradation
- FB-CAN-005: Trigger/timing
- FB-CAN-006: Opportunity auction
- FB-CAN-007: Risk/sizing/exposure
- FB-CAN-008: Execution confidence/style/stress/feedback
- FB-CAN-009: Replay/simulation interface
- FB-CAN-010: Monitoring/alerting
- FB-CAN-011: Governance + experiment registry
- FB-CAN-012: Deletion of obsolete architecture
- FB-CAN-013: Tests/CI replacement
- FB-CAN-014: End-state cleanup and module map
- FB-CAN-015: Canonical input contract implementation
- FB-CAN-016: Canonical normalization + confidence/freshness field conformance
- FB-CAN-017: Structure output contract replacement from ForecastPacket-centric shape
- FB-CAN-018: Carry sleeve implementation
- FB-CAN-019: Decision module legacy deletions
- FB-CAN-020: Risk module legacy deletions
- FB-CAN-021: Execution module legacy deletions
- FB-CAN-022: Legacy config key/env deletion and migration cleanup
- FB-CAN-023: Legacy doc/spec deletion and archival redirects
- FB-CAN-024: Legacy behavior test deletion/replacement
- FB-CAN-025: Canonical CI gates + deletion-completion checks
- FB-CAN-026: Release evidence bundle implementation
- FB-CAN-027: Experiment registry implementation
- FB-CAN-028: Monitoring alert-policy implementation
- FB-CAN-029: Canonical runtime orchestration refactor
- FB-CAN-030: Live-vs-replay deterministic equivalence gating
- FB-CAN-031: Missed-move and false-positive memory penalties
- FB-CAN-032: Signal-family confidence/decay domain completion
- FB-CAN-033: Hard override taxonomy + degradation transition accounting
- FB-CAN-034: Auction correlation/thesis-clustering constraints
- FB-CAN-035: Execution feedback memory integration
- FB-CAN-036: Canonical decision-record and suppression/safety event outputs
- FB-CAN-037: Fault-injection replay profiles and gating evidence
- FB-CAN-038: Shadow promotion thresholds/probation controls
- FB-CAN-039: Calibration/drift monitoring and alerting completion
- FB-CAN-040: Canonical glossary and terminology normalization
- FB-CAN-041: Full 5-class regime vector and transition confidence math
- FB-CAN-042: Novelty/heat/reflexivity propagation stack
- FB-CAN-043: Trigger stage telemetry and latency instrumentation
- FB-CAN-044: Auction throughput bounds and top-N saturation controls
- FB-CAN-045: Canonical sizing formula stack details
- FB-CAN-046: Thesis-aware concentration/overlap risk constraints
- FB-CAN-047: Execution style selection policy implementation
- FB-CAN-048: Partial-fill reconciliation and residual-intent logic
- FB-CAN-049: Structural signal family ingestion completion
- FB-CAN-050: Options-context and stablecoin-flow proxy families
- FB-CAN-051: Release-object schema implementation
- FB-CAN-052: Environment-stage enforcement implementation
- FB-CAN-053: Rollback playbook and rollback-target validation
- FB-CAN-054: Experiment predeclared metrics/failure-mode validation
- FB-CAN-055: Canonical replay-mode and event-family coverage checks
- FB-CAN-056: Monitoring-domain coverage assertions in CI
- FB-CAN-057: Immutable config diff and semantic guardrails
- FB-CAN-058: Tombstone index for removed modules/docs
- FB-CAN-059: Runtime dual-path cutover guard
- FB-CAN-060: Final shim/legacy remnant prune
- FB-CAN-061: Canonical config metadata field implementation
- FB-CAN-062: Common contract convention enforcement
- FB-CAN-063: Suppression/safety reason taxonomy stabilization
- FB-CAN-064: Carry-sleeve monitoring domain completion
- FB-CAN-065: Governance/operator monitoring domain completion
- FB-CAN-066: Promotion evidence-package schema and completeness gating
- FB-CAN-067: Magic-constant guardrails for canonical modules
- FB-CAN-068: Replay/simulation deterministic seed provenance policy
- FB-CAN-069: Post-release probation monitoring and abort triggers
- FB-CAN-070: Canonical spec section coverage matrix and closure gate
- FB-CAN-071: Service Configuration Snapshot contract
- FB-CAN-072: Processing lag/event-lag/cycle-duration monitoring details
- FB-CAN-073: Weekend/low-liquidity occupancy handling
- FB-CAN-074: Exchange-risk/data-integrity safety input integration
- FB-CAN-075: Execution latency/reliability quality term implementation
- FB-CAN-076: Edge-budget monitoring threshold implementation
- FB-CAN-077: Immutable run-binding guard implementation
- FB-CAN-078: Automated canonical acceptance audit gate

## 8.1 Comprehensive reconfiguration/deletion matrix (additional pass)

To explicitly capture **all** replacement work, the following reconfiguration and deletion buckets are now tracked in the queue:

### Decision domain
- Reconfigure to canonical state/trigger/auction sequence (FB-CAN-004/005/006/029).
- Delete legacy single-proposal pipeline modules after cutover (FB-CAN-019).

### Risk domain
- Reconfigure risk sizing/exposure logic to canonical model (FB-CAN-007).
- Delete legacy hard-gate-only code paths/contracts that conflict post-cutover (FB-CAN-020).

### Execution domain
- Reconfigure service to execution-confidence + style + stress + partial-fill feedback (FB-CAN-008).
- Delete thin submit-only logic branches after cutover (FB-CAN-021).

### Data + contract domain
- Reconfigure ingest/feature outputs to canonical normalized contracts (FB-CAN-015/016/017).
- Remove ad-hoc boundary payload usage once canonical typed contracts are active (FB-CAN-015/016).

### Config domain
- Reconfigure AppSettings/default YAML into canonical domain/version model (FB-CAN-003).
- Delete superseded settings/env surfaces after migration window (FB-CAN-022).

### Replay/simulation domain
- Reconfigure replay interface to canonical event families + profiles (FB-CAN-009/030).
- Delete replay assumptions/tests that validate non-canonical event semantics (FB-CAN-024/030).

### Monitoring/governance domain
- Reconfigure metrics and alerts to canonical families (FB-CAN-010/028).
- Reconfigure release gating and experiment registry artifacts (FB-CAN-011/026/027).
- Add CI gates that fail when legacy modules remain past cutover (FB-CAN-025).

### Docs/specs/tests domain
- Reconfigure docs authority to canonical set and cross-link precedence (FB-CAN-002).
- Delete/archive conflicting docs and legacy architecture test expectations (FB-CAN-023/024).

## 9) Proposed end-state module structure

```
app/
  config/
    canonical_schema.py
    version_registry.py
  contracts/
    canonical/
      decision_snapshot.py
      trigger_record.py
      auction_candidate.py
      execution_feedback.py
      replay_event.py

data_plane/
  ingest/
  normalize/
  features/

decision_engine/
  state_engine.py
  structure_engine.py
  trigger_engine.py
  auction_engine.py
  decision_orchestrator.py

risk_engine/
  canonical_risk_engine.py
  exposure_model.py
  sizing_model.py

execution/
  execution_confidence.py
  execution_policy.py
  partial_fill.py
  feedback.py
  service.py
  adapters/

carry_sleeve/
  carry_allocator.py
  carry_risk.py

backtesting/
  replay_runner.py
  simulation_runner.py
  fault_injection.py

observability/
  canonical_metrics.py
  alerts/

governance/
  release_gates.py
  promotion_registry.py

research/
  experiment_registry.py
  evidence_linking.py
```

### Old->new mapping
- `decision_engine/pipeline.py` -> split into `state_engine`, `structure_engine`, `trigger_engine`, `auction_engine`, orchestrator.
- `risk_engine/engine.py` -> `canonical_risk_engine.py` + `sizing_model.py` + `exposure_model.py`.
- `execution/service.py` -> kept as entrypoint but backed by new execution modules.
- `backtesting/replay.py` -> `replay_runner.py` contractized output model.
- `docs/Specs/*.MD` -> removed or archived as non-canonical historical references.

### Modules expected to disappear
- Legacy single-path decision modules tightly coupled to `ForecastPacket -> PolicySystem -> single proposal` without canonical state/trigger/auction layers.
- Legacy docs in `docs/Specs/` that assert conflicting architecture.
- Legacy tests that encode removed behavior.

## 10) Preserve-only exceptions

Preserve these foundations during replacement (with refactor, not deletion):
- Kraken-only market data principle and compliance checks.
- Risk signing + intent gate requirements.
- Execution adapter routing abstraction (`execution/adapters/`) as transport boundary.
- Shared live/replay philosophy (`run_decision_tick` equivalent), updated to canonical event contracts.
