"""FB-CAN-063: canonical suppression / safety reason code normalization."""

from __future__ import annotations

from app.contracts.reason_codes import (
    AUC_MISSED_MOVE,
    PIP_NO_TRADE_SELECTED,
    TRG_DEGRADATION_BLOCK,
    normalize_reason_code,
    normalize_reason_codes,
)
from risk_engine.engine import RISK_BLOCK_FEED_STALE


def test_normalize_legacy_trigger_strings():
    assert normalize_reason_code("degradation_block") == TRG_DEGRADATION_BLOCK
    assert normalize_reason_code("missed_move") == AUC_MISSED_MOVE


def test_normalize_pipeline_and_risk():
    assert normalize_reason_code("pipeline_no_trade_selected") == PIP_NO_TRADE_SELECTED
    assert normalize_reason_code(RISK_BLOCK_FEED_STALE) == RISK_BLOCK_FEED_STALE


def test_normalize_hard_override_prefix():
    assert normalize_reason_code("hard_override_spread_wide") == "ovr_hard_spread_wide"


def test_normalize_reason_codes_dedupes():
    assert normalize_reason_codes(
        ["degradation_block", TRG_DEGRADATION_BLOCK, "degradation_block"]
    ) == [TRG_DEGRADATION_BLOCK]
