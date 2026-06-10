# Canonical tombstone index (removed modules & docs)

**Purpose (FB-CAN-058):** Single **lookup table** for paths removed during the **FB-CAN-*** migration, with **GitHub commit links** (last change to that path before removal or the removal itself) and **canonical replacement** pointers. Use this when a bookmark, issue, or old branch still references a deleted file.

**Narrative log:** [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) — same removals with fuller notes per slice.

**Gap audit context:** [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md) §5.1 (deletion candidates vs as-built Specs retained).

**Repo:** [MHughesDev/trading_bot](https://github.com/MHughesDev/trading_bot) — commit links below use `https://github.com/MHughesDev/trading_bot/commit/<hash>`.

---

## Index (path → replacement → commit)

| Slice | Removed path | Canonical / current replacement | Commit (removal or last touch) |
|-------|----------------|----------------------------------|--------------------------------|
| FB-CAN-012 | `docs/Specs/MASTER_SPEC_RISK_STATE_GAP.MD` | Gap audit + APEX risk/state specs; see [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) | [`ef39b9b`](https://github.com/MHughesDev/trading_bot/commit/ef39b9bb0a35c8e67cc25d72a374f5863ba950d5) |
| FB-CAN-019 | `decision_engine/action_generator.py` | [`decision_engine/spec_policy_proposal.py`](../decision_engine/spec_policy_proposal.py) (PolicySystem + trigger + auction); [`run_decision_tick`](../decision_engine/run_step.py) | [`276bb8c`](https://github.com/MHughesDev/trading_bot/commit/276bb8cbd51b5b7855ac57efec22d4dba0d5b1b8) |
| FB-CAN-019 | `tests/test_action_generator_routes.py` | Covered by pipeline / policy tests; see [`Specs/TESTING_AND_CI.MD`](Specs/TESTING_AND_CI.MD) | *(same removal commit as `action_generator.py`)* |
| FB-CAN-020 | `risk_engine/equal_weight.py` | [`risk_engine/canonical_sizing.py`](../risk_engine/canonical_sizing.py) | [`6394bc8`](https://github.com/MHughesDev/trading_bot/commit/6394bc895b183f0417e98b7c81b00ba7d7da14da) |
| FB-CAN-020 | `tests/test_equal_weight.py` | *(same removal commit as `equal_weight.py`)* | [`6394bc8`](https://github.com/MHughesDev/trading_bot/commit/6394bc895b183f0417e98b7c81b00ba7d7da14da) |
| FB-CAN-021 | `execution/router.py` — duplicate `get_execution_adapter` | **`create_execution_adapter`** only — [`execution/router.py`](../execution/router.py) | [`e17dc97`](https://github.com/MHughesDev/trading_bot/commit/e17dc972a19fae223ce584e5e0354e5ed271d584) |
| FB-CAN-022 | Top-level `risk:` in `app/config/default.yaml` | **`apex_canonical.domains.risk_sizing`** + `AppSettings` / `NM_RISK_*`; loader rejects legacy `risk:` | [`c0204a8`](https://github.com/MHughesDev/trading_bot/commit/c0204a80d5d85ede764a35c11611ba3fac953e29) |
| FB-CAN-024 | `tests/test_spec_pipeline_mode.py` | [`tests/test_pipeline_forecast_packet.py`](../tests/test_pipeline_forecast_packet.py), [`tests/test_decision_risk.py`](../tests/test_decision_risk.py) | [`d6ef542`](https://github.com/MHughesDev/trading_bot/commit/d6ef5422c05aa6705b973642ebf6b281e8c25263) |

---

## Not removed (clarification)

- **`docs/Specs/*.MD` (folder):** **Not** wholesale-deleted. **FB-CAN-023** added precedence + per-file tombstones; as-built Specs remain the **current implementation** map. See [`Specs/CANONICAL_PRECEDENCE.MD`](Specs/CANONICAL_PRECEDENCE.MD) and gap audit §5.1.
- **Individual Specs files** listed as *candidates* in the gap audit are removed only when a **future queue item** explicitly deletes them; until then they stay with header notes.

---

## How to add a row

1. Record the removal in [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) (slice ID, what, why).
2. Append a row here with `git log -1 -- <path>` on the removed path (or the merge commit that deleted it).
3. Link the replacement module or doc under `docs/Human Provided Specs/new_specs/canonical/` where applicable.
