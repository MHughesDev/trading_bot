"""FB-CAN-038: shadow replay comparison and policy validation."""

from __future__ import annotations

from app.config.shadow_comparison import (
    ShadowComparisonPolicy,
    validate_shadow_comparison_domain,
)
from orchestration.shadow_comparison import (
    compare_shadow_replay_rows,
    extract_shadow_vector,
)


def test_validate_shadow_domain_ok():
    assert validate_shadow_comparison_domain({}) == []


def test_validate_shadow_domain_bad_threshold():
    errs = validate_shadow_comparison_domain(
        {"enabled": True, "thresholds": {"trigger_divergence_max": 2.0}}
    )
    assert errs


def test_identical_rows_zero_divergence():
    row = {
        "canonical_events": [
            {
                "event_family": "decision_output_event",
                "payload": {
                    "trigger": {"trigger_valid": True},
                    "auction": {
                        "selected_direction": 0,
                        "selected_score": 0.1,
                        "records": [{"eligible": True, "direction": 0, "auction_score": 0.1}],
                    },
                    "decision_record": {"outcome": "no_trade", "trade_intent": None, "suppression": None},
                },
            }
        ]
    }
    pol = ShadowComparisonPolicy()
    rep = compare_shadow_replay_rows([row], [row], policy=pol)
    assert rep.bars_compared == 1
    assert rep.trigger_divergence_rate == 0.0
    assert rep.within_thresholds is True


def test_trigger_divergence_detected():
    a = {
        "canonical_events": [
            {
                "event_family": "decision_output_event",
                "payload": {
                    "trigger": {"trigger_valid": True},
                    "auction": {"selected_direction": None, "selected_score": None, "records": []},
                    "decision_record": {"outcome": "no_trade"},
                },
            }
        ]
    }
    b = {
        "canonical_events": [
            {
                "event_family": "decision_output_event",
                "payload": {
                    "trigger": {"trigger_valid": False},
                    "auction": {"selected_direction": None, "selected_score": None, "records": []},
                    "decision_record": {"outcome": "no_trade"},
                },
            }
        ]
    }
    pol = ShadowComparisonPolicy()
    rep = compare_shadow_replay_rows([a], [b], policy=pol)
    assert rep.trigger_divergence_rate == 1.0
    assert rep.within_thresholds is False


def test_extract_shadow_vector_keys():
    p = extract_shadow_vector(
        {
            "trigger": {"trigger_valid": True},
            "auction": {
                "selected_direction": 1,
                "selected_score": 0.5,
                "records": [{"eligible": True, "direction": 1, "auction_score": 0.5}],
            },
            "decision_record": {"outcome": "trade_intent", "trade_intent": {"side": "long"}},
        }
    )
    assert p["trigger_valid"] is True
    assert "1:" in p["candidate_key"]
