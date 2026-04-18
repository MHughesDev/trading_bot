"""Shadow vs baseline replay comparison (FB-CAN-038).

Compares per-bar canonical ``decision_output_event`` payloads from two replay runs
(baseline / live reference vs candidate / shadow logic) and aggregates divergence rates.

See APEX Config Management spec §9.3–9.4 and Replay spec §11.2.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from typing import Any

from app.config.shadow_comparison import ShadowComparisonPolicy, shadow_policy_from_settings


def _stable_json(obj: Any) -> str:
    return json.dumps(obj, sort_keys=True, default=str, separators=(",", ":"))


def _payload_from_row(row: dict[str, Any]) -> dict[str, Any]:
    evs = row.get("canonical_events")
    if not isinstance(evs, list):
        return {}
    for ev in evs:
        if isinstance(ev, dict) and ev.get("event_family") == "decision_output_event":
            p = ev.get("payload")
            return p if isinstance(p, dict) else {}
    return {}


def _stable_vec(v: dict[str, Any]) -> str:
    return _stable_json(v)


def extract_shadow_vector(payload: dict[str, Any]) -> dict[str, Any]:
    """Compact, comparable view of one decision tick for divergence counting."""
    trig = payload.get("trigger") if isinstance(payload.get("trigger"), dict) else {}
    auct = payload.get("auction") if isinstance(payload.get("auction"), dict) else {}
    dr = payload.get("decision_record") if isinstance(payload.get("decision_record"), dict) else {}
    route = payload.get("route") if isinstance(payload.get("route"), dict) else {}

    trigger_valid = bool(trig.get("trigger_valid"))

    sel_dir = auct.get("selected_direction")
    sel_score = auct.get("selected_score")
    auction_key = f"{sel_dir}:{sel_score!s}"

    recs = auct.get("records") if isinstance(auct.get("records"), list) else []
    cand_parts: list[str] = []
    for r in recs:
        if not isinstance(r, dict):
            continue
        if not r.get("eligible"):
            continue
        cand_parts.append(f"{r.get('direction')}:{round(float(r.get('auction_score', 0.0)), 6)}")
    candidate_key = "|".join(cand_parts[:8])

    outcome = str(dr.get("outcome") or "")
    ti = dr.get("trade_intent") if isinstance(dr.get("trade_intent"), dict) else None
    intent_side = str(ti.get("side") or "") if ti else ""
    sup = dr.get("suppression")
    suppressed = bool(sup)

    _ = route  # reserved for future route-divergence metric

    return {
        "trigger_valid": trigger_valid,
        "candidate_key": candidate_key,
        "auction_key": auction_key,
        "decision_outcome": outcome,
        "trade_intent_side": intent_side,
        "suppressed": suppressed,
    }


@dataclass
class ShadowComparisonReport:
    """Structured result for release evidence + governance APIs."""

    schema_version: int = 1
    generated_at: str = ""
    baseline_replay_run_id: str = ""
    candidate_replay_run_id: str = ""
    config_version: str = ""
    logic_version_baseline: str | None = None
    logic_version_candidate: str | None = None
    bars_compared: int = 0
    trigger_divergence_rate: float = 0.0
    candidate_divergence_rate: float = 0.0
    auction_divergence_rate: float = 0.0
    suppression_divergence_rate: float = 0.0
    trade_intent_divergence_rate: float = 0.0
    mismatch_indices: list[int] = field(default_factory=list)
    probation_passed: bool = False
    within_thresholds: bool = False
    rollback_recommended: bool = False
    policy: dict[str, Any] = field(default_factory=dict)
    notes: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "generated_at": self.generated_at,
            "baseline_replay_run_id": self.baseline_replay_run_id,
            "candidate_replay_run_id": self.candidate_replay_run_id,
            "config_version": self.config_version,
            "logic_version_baseline": self.logic_version_baseline,
            "logic_version_candidate": self.logic_version_candidate,
            "bars_compared": self.bars_compared,
            "rates": {
                "trigger_divergence": self.trigger_divergence_rate,
                "candidate_divergence": self.candidate_divergence_rate,
                "auction_divergence": self.auction_divergence_rate,
                "suppression_divergence": self.suppression_divergence_rate,
                "trade_intent_divergence": self.trade_intent_divergence_rate,
            },
            "mismatch_indices": self.mismatch_indices[:64],
            "probation_passed": self.probation_passed,
            "within_thresholds": self.within_thresholds,
            "rollback_recommended": self.rollback_recommended,
            "policy": self.policy,
            "notes": self.notes,
        }


def compare_shadow_replay_rows(
    baseline_rows: list[dict[str, Any]],
    candidate_rows: list[dict[str, Any]],
    *,
    policy: ShadowComparisonPolicy,
    baseline_replay_run_id: str = "",
    candidate_replay_run_id: str = "",
    config_version: str = "",
    logic_version_baseline: str | None = None,
    logic_version_candidate: str | None = None,
) -> ShadowComparisonReport:
    """Pairwise compare replay outputs (same bar count)."""
    thr = policy.thresholds
    prob = policy.probation
    rb = policy.rollback

    n = min(len(baseline_rows), len(candidate_rows))
    if len(baseline_rows) != len(candidate_rows):
        note = f"length mismatch: baseline={len(baseline_rows)} candidate={len(candidate_rows)}; compared first {n}"
    else:
        note = ""

    trig_m = cand_m = auct_m = sup_m = intent_m = 0
    mismatches: list[int] = []

    for i in range(n):
        bp = _payload_from_row(baseline_rows[i])
        cp = _payload_from_row(candidate_rows[i])
        bv = extract_shadow_vector(bp)
        cv = extract_shadow_vector(cp)

        if bv.get("trigger_valid") != cv.get("trigger_valid"):
            trig_m += 1
        if bv.get("candidate_key") != cv.get("candidate_key"):
            cand_m += 1
        if bv.get("auction_key") != cv.get("auction_key"):
            auct_m += 1
        if bv.get("suppressed") != cv.get("suppressed"):
            sup_m += 1
        if bv.get("decision_outcome") != cv.get("decision_outcome") or bv.get(
            "trade_intent_side"
        ) != cv.get("trade_intent_side"):
            intent_m += 1

        if _stable_vec(bv) != _stable_vec(cv):
            mismatches.append(i)

    def rate(count: int) -> float:
        return float(count) / float(n) if n else 0.0

    tr = rate(trig_m)
    cr = rate(cand_m)
    ar = rate(auct_m)
    sr = rate(sup_m)
    ir = rate(intent_m)

    within = (
        tr <= thr.trigger_divergence_max
        and cr <= thr.candidate_divergence_max
        and ar <= thr.auction_divergence_max
        and sr <= thr.suppression_divergence_max
        and ir <= thr.trade_intent_divergence_max
    )
    probation_ok = n >= prob.min_bars

    severe = (
        tr > thr.trigger_divergence_max * rb.severe_rate_multiplier
        or cr > thr.candidate_divergence_max * rb.severe_rate_multiplier
        or ar > thr.auction_divergence_max * rb.severe_rate_multiplier
        or sr > thr.suppression_divergence_max * rb.severe_rate_multiplier
        or ir > thr.trade_intent_divergence_max * rb.severe_rate_multiplier
    )
    rollback = severe or (not within and n >= prob.min_bars)

    return ShadowComparisonReport(
        generated_at=datetime.now(UTC).replace(microsecond=0).isoformat(),
        baseline_replay_run_id=baseline_replay_run_id,
        candidate_replay_run_id=candidate_replay_run_id,
        config_version=config_version,
        logic_version_baseline=logic_version_baseline,
        logic_version_candidate=logic_version_candidate,
        bars_compared=n,
        trigger_divergence_rate=tr,
        candidate_divergence_rate=cr,
        auction_divergence_rate=ar,
        suppression_divergence_rate=sr,
        trade_intent_divergence_rate=ir,
        mismatch_indices=mismatches[:128],
        probation_passed=probation_ok,
        within_thresholds=within,
        rollback_recommended=rollback,
        policy=json.loads(policy.model_dump_json()),
        notes=note,
    )


def run_shadow_replay_pair_comparison(
    *,
    settings: Any,
    bars: int,
    symbol: str,
    baseline_replay_run_id: str,
    candidate_replay_run_id: str,
    baseline_logic_version: str,
    candidate_logic_version: str,
) -> dict[str, Any]:
    """Run two full replays and return JSON-serializable report + passed flag."""
    import polars as pl

    from app.config.settings import AppSettings
    from app.contracts.replay_events import ReplayRunContract
    from backtesting.replay import replay_decisions
    from decision_engine.pipeline import DecisionPipeline
    from risk_engine.engine import RiskEngine

    policy = shadow_policy_from_settings(settings)
    base = datetime(2026, 6, 1, 0, 0, 0, tzinfo=UTC)
    rows_out: list[dict[str, Any]] = []
    for i in range(bars):
        t = base + timedelta(seconds=i)
        p = 100.0 + i * 0.05
        rows_out.append(
            {"timestamp": t, "open": p, "high": p + 0.02, "low": p - 0.02, "close": p, "volume": 1.0}
        )
    df = pl.DataFrame(rows_out)

    cr = getattr(settings, "canonical", None)
    md = getattr(cr, "metadata", None) if cr is not None else None
    cfg_v = str(getattr(md, "config_version", "1.0.0"))

    base_c = ReplayRunContract(
        replay_run_id=baseline_replay_run_id,
        dataset_id="shadow-compare",
        config_version=cfg_v,
        logic_version=baseline_logic_version,
        instrument_scope=[symbol],
        replay_mode="shadow_comparison",
    )
    cand_c = ReplayRunContract(
        replay_run_id=candidate_replay_run_id,
        dataset_id="shadow-compare",
        config_version=cfg_v,
        logic_version=candidate_logic_version,
        instrument_scope=[symbol],
        replay_mode="shadow_comparison",
    )

    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    base_out = replay_decisions(
        df,
        pipe,
        eng,
        symbol=symbol,
        spread_bps=5.0,
        replay_contract=base_c,
        emit_canonical_events=True,
    )
    pipe2 = DecisionPipeline()
    eng2 = RiskEngine(AppSettings())
    cand_out = replay_decisions(
        df,
        pipe2,
        eng2,
        symbol=symbol,
        spread_bps=5.0,
        replay_contract=cand_c,
        emit_canonical_events=True,
    )

    rep = compare_shadow_replay_rows(
        base_out,
        cand_out,
        policy=policy,
        baseline_replay_run_id=baseline_replay_run_id,
        candidate_replay_run_id=candidate_replay_run_id,
        config_version=cfg_v,
        logic_version_baseline=baseline_logic_version,
        logic_version_candidate=candidate_logic_version,
    )
    out = rep.to_dict()
    out["shadow_comparison_passed"] = bool(rep.within_thresholds and rep.probation_passed)
    return out


__all__ = [
    "ShadowComparisonReport",
    "compare_shadow_replay_rows",
    "extract_shadow_vector",
    "run_shadow_replay_pair_comparison",
]
