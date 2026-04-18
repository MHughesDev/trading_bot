"""Canonical suppression / safety / no-trade reason codes (FB-CAN-063).

Stable ``snake_case`` identifiers with layer prefixes for monitoring and replay:

- ``trg_*`` — trigger engine
- ``auc_*`` — opportunity auction
- ``risk_*`` — risk engine blocks (unchanged public codes)
- ``exe_*`` — execution guidance / stress / style
- ``pip_*`` — pipeline / policy / carry overlay
- ``ovr_*`` — hard safety override classification
- ``state_*`` — canonical state / novelty decomposition (optional on apex)

Legacy ad-hoc strings are normalized via :func:`normalize_reason_code`.
"""

from __future__ import annotations

import re

_CODE_RE = re.compile(r"^[a-z][a-z0-9_]*$")

# --- Canonical primary strings (emit these from new code paths) ---

# Pipeline / orchestrator
PIP_NO_TRADE_SELECTED = "pip_no_trade_selected"
PIP_BINDING_ABSTAIN = "pip_binding_abstain"
PIP_CARRY_SLEEVE_BLOCKED = "pip_carry_sleeve_blocked"
PIP_TRADE_SELECTED = "pip_trade_selected"

# Trigger (subset; see ALIASES for full mapping)
TRG_NOVELTY_BLOCK = "trg_novelty_block"
TRG_DEGRADATION_BLOCK = "trg_degradation_block"
TRG_LOW_SETUP_SCORE = "trg_low_setup_score"
TRG_POOR_EXECUTION_CONTEXT = "trg_poor_execution_context"
TRG_PRESSURE_NOT_BUILDING = "trg_pressure_not_building"
TRG_STALE_PRETRIGGER_INPUTS = "trg_stale_pretrigger_inputs"
TRG_MOVE_ALREADY_EXTENDED = "trg_move_already_extended"
TRG_INSUFFICIENT_REMAINING_EDGE = "trg_insufficient_remaining_edge"
TRG_EXECUTION_TOO_DEGRADED = "trg_execution_too_degraded"
TRG_TRIGGER_STRENGTH_LOW = "trg_trigger_strength_low"

# Auction
AUC_DEGRADATION_NO_TRADE = "auc_degradation_no_trade"
AUC_MISSED_MOVE = "auc_missed_move"
AUC_TRIGGER_INVALID = "auc_trigger_invalid"
AUC_TRIGGER_CONF_BELOW_MIN = "auc_trigger_confidence_below_min"
AUC_DECISION_CONF_BELOW_MIN = "auc_decision_confidence_below_min"
AUC_EXEC_CONF_BELOW_MIN = "auc_execution_confidence_below_min"
AUC_THESIS_OVERLAP_CAP = "auc_thesis_overlap_cap"
AUC_NOTIONAL_BUDGET = "auc_notional_budget"
AUC_OUTRANKED = "auc_outranked"
AUC_TOP_N_SHORTFALL = "auc_top_n_throughput_shortfall"

# Execution guidance (prefix exe_; map legacy stress_* / style_branch_*)
EXE_STRESS_SPREAD = "exe_stress_spread_widening"
EXE_STRESS_HEAT = "exe_stress_heat_extreme"
EXE_STRESS_VENUE = "exe_stress_venue_degradation"
EXE_STRESS_VOL = "exe_stress_volatility"
EXE_STRESS_LIQ = "exe_stress_liquidity_collapse"
EXE_WORST_CASE_EDGE = "exe_worst_case_edge_below_min"
EXE_CONF_FLOOR = "exe_execution_confidence_floor"
EXE_STYLE_PASSIVE = "exe_style_branch_passive_high_conf_tight_spread"
EXE_STYLE_STAGGERED = "exe_style_branch_staggered_medium_conf"
EXE_STYLE_AGGRESSIVE = "exe_style_branch_aggressive_urgency_remaining_edge"
EXE_STYLE_TWAP = "exe_style_branch_default_twap"
EXE_STYLE_TWAP_STRESS = "exe_style_branch_stress_low_exec_conf_twap"
EXE_STYLE_SUPPRESS = "exe_style_branch_suppress"

# Safety override envelope (ovr_ + kind)
OVR_PREFIX = "ovr_"

# Risk block strings: defined in ``risk_engine.engine`` (``RISK_BLOCK_*``); keep stable there.

# State / novelty decomposition (optional on apex)
STATE_HIGH_OOD = "state_high_ood"
STATE_HMM_AMBIGUOUS = "state_hmm_ambiguous"
STATE_STRUCTURE_FRAGILE = "state_structure_fragile"
STATE_ELEVATED_TRANSITION = "state_elevated_transition_risk"

# Legacy string -> canonical (trigger)
_ALIASES: dict[str, str] = {
    # Trigger
    "novelty_block": TRG_NOVELTY_BLOCK,
    "degradation_block": TRG_DEGRADATION_BLOCK,
    "low_setup_score": TRG_LOW_SETUP_SCORE,
    "poor_execution_context": TRG_POOR_EXECUTION_CONTEXT,
    "pressure_not_building": TRG_PRESSURE_NOT_BUILDING,
    "stale_pretrigger_inputs": TRG_STALE_PRETRIGGER_INPUTS,
    "move_already_extended": TRG_MOVE_ALREADY_EXTENDED,
    "insufficient_remaining_edge": TRG_INSUFFICIENT_REMAINING_EDGE,
    "execution_too_degraded": TRG_EXECUTION_TOO_DEGRADED,
    "trigger_strength_low": TRG_TRIGGER_STRENGTH_LOW,
    # Auction
    "degradation_no_trade": AUC_DEGRADATION_NO_TRADE,
    "missed_move": AUC_MISSED_MOVE,
    "trigger_invalid": AUC_TRIGGER_INVALID,
    "trigger_confidence_below_min": AUC_TRIGGER_CONF_BELOW_MIN,
    "decision_confidence_below_min": AUC_DECISION_CONF_BELOW_MIN,
    "execution_confidence_below_min": AUC_EXEC_CONF_BELOW_MIN,
    "thesis_overlap_cap": AUC_THESIS_OVERLAP_CAP,
    "notional_budget": AUC_NOTIONAL_BUDGET,
    "outranked": AUC_OUTRANKED,
    "top_n_throughput_shortfall": AUC_TOP_N_SHORTFALL,
    # Pipeline
    "pipeline_no_trade_selected": PIP_NO_TRADE_SELECTED,
    "pipeline_binding_abstain": PIP_BINDING_ABSTAIN,
    "carry_sleeve_directional_blocked": PIP_CARRY_SLEEVE_BLOCKED,
    "pipeline_trade_selected": PIP_TRADE_SELECTED,
    # Execution / style
    "stress_spread_widening": EXE_STRESS_SPREAD,
    "stress_heat_extreme": EXE_STRESS_HEAT,
    "stress_venue_degradation": EXE_STRESS_VENUE,
    "stress_volatility": EXE_STRESS_VOL,
    "stress_liquidity_collapse": EXE_STRESS_LIQ,
    "worst_case_edge_below_min": EXE_WORST_CASE_EDGE,
    "execution_confidence_floor": EXE_CONF_FLOOR,
    "style_branch_passive_high_conf_tight_spread": EXE_STYLE_PASSIVE,
    "style_branch_staggered_medium_conf": EXE_STYLE_STAGGERED,
    "style_branch_aggressive_urgency_remaining_edge": EXE_STYLE_AGGRESSIVE,
    "style_branch_default_twap": EXE_STYLE_TWAP,
    "style_branch_stress_low_exec_conf_twap": EXE_STYLE_TWAP_STRESS,
    "style_branch_suppress": EXE_STYLE_SUPPRESS,
    # Reduce exposure / system
    "flatten_all": "risk_flatten_all",
    "system_power_off": "pip_system_power_off",
    "no_trade_unknown": "pip_no_trade_unknown",
    # State / novelty
    "high_ood": STATE_HIGH_OOD,
    "hmm_ambiguous": STATE_HMM_AMBIGUOUS,
    "structure_fragile": STATE_STRUCTURE_FRAGILE,
    "elevated_transition_risk": STATE_ELEVATED_TRANSITION,
}


def normalize_reason_code(code: str) -> str:
    """Map legacy reason strings to canonical stable codes; pass through if already canonical."""
    if code is None:
        return "unknown"
    s = str(code).strip()
    if not s:
        return "unknown"
    if s in _ALIASES:
        return _ALIASES[s]
    if s.startswith("hard_override_"):
        rest = s[len("hard_override_") :].strip("_")
        return f"{OVR_PREFIX}hard_{rest}" if rest else f"{OVR_PREFIX}hard_unknown"
    if s.startswith("risk_"):
        return s
    if s.startswith(("trg_", "auc_", "exe_", "pip_", "ovr_", "state_")):
        return s
    # Unknown freeform: keep but validate shape for metrics safety
    if _CODE_RE.match(s):
        return s
    return "unknown_malformed_reason"


def normalize_reason_codes(codes: list[str] | None) -> list[str]:
    """Normalize, de-duplicate preserving order."""
    if not codes:
        return []
    out: list[str] = []
    seen: set[str] = set()
    for c in codes:
        n = normalize_reason_code(c)
        if n not in seen:
            seen.add(n)
            out.append(n)
    return out
