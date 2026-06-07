"""Tests for the universal strategy-builder schema (FB-AP-XXX) — pure-Python, no nautilus."""

from __future__ import annotations

import pytest

from strategies.rule_spec import (
    Condition,
    EntryRule,
    ExitRule,
    IndicatorSpec,
    RuleSpecError,
    RuleStrategySpec,
    SizeRule,
)


def _ema_cross_spec() -> RuleStrategySpec:
    return RuleStrategySpec(
        name="EMA 7/21 cross",
        indicators=(
            IndicatorSpec(id="ema_fast", kind="ema", period=7),
            IndicatorSpec(id="ema_slow", kind="ema", period=21),
        ),
        entry=EntryRule(
            side="buy",
            all_of=(Condition(type="cross_above", left="ema_fast", right_id="ema_slow"),),
        ),
        size=SizeRule(type="percent_of_equity", value=0.02),
        exits=(ExitRule(type="stop_loss", value=0.015), ExitRule(type="take_profit", value=0.04)),
    )


def test_valid_spec_passes_validation() -> None:
    _ema_cross_spec().validate()


def test_explain_is_human_readable() -> None:
    text = _ema_cross_spec().explain()
    assert "Buy when" in text
    assert "7-period EMA" in text
    assert "crosses above" in text
    assert "stop loss at 1.5%" in text
    assert "take profit at 4%" in text


def test_round_trips_through_dict_and_json() -> None:
    spec = _ema_cross_spec()
    again = RuleStrategySpec.from_dict(spec.to_dict())
    assert again == spec
    assert RuleStrategySpec.from_json(spec.to_json()) == spec


def test_rejects_missing_name() -> None:
    spec = _ema_cross_spec().renamed("")
    with pytest.raises(RuleSpecError, match="needs a name"):
        spec.validate()


def test_rejects_no_indicators() -> None:
    spec = RuleStrategySpec(name="x", entry=EntryRule(side="buy", all_of=(Condition(type="rising", left="price"),)))
    with pytest.raises(RuleSpecError, match="at least one indicator"):
        spec.validate()


def test_rejects_duplicate_indicator_ids() -> None:
    spec = RuleStrategySpec(
        name="x",
        indicators=(IndicatorSpec(id="a", kind="ema", period=5), IndicatorSpec(id="a", kind="sma", period=10)),
        entry=EntryRule(side="buy", all_of=(Condition(type="rising", left="a"),)),
        exits=(ExitRule(type="stop_loss", value=0.01),),
    )
    with pytest.raises(RuleSpecError, match="must be unique"):
        spec.validate()


def test_condition_referencing_unknown_indicator_rejected() -> None:
    spec = RuleStrategySpec(
        name="x",
        indicators=(IndicatorSpec(id="a", kind="ema", period=5),),
        entry=EntryRule(side="buy", all_of=(Condition(type="cross_above", left="a", right_id="ghost"),)),
        exits=(ExitRule(type="stop_loss", value=0.01),),
    )
    with pytest.raises(RuleSpecError, match="unknown indicator"):
        spec.validate()


def test_entry_requires_at_least_one_condition() -> None:
    spec = RuleStrategySpec(
        name="x",
        indicators=(IndicatorSpec(id="a", kind="ema", period=5),),
        entry=EntryRule(side="buy"),
        exits=(ExitRule(type="stop_loss", value=0.01),),
    )
    with pytest.raises(RuleSpecError, match="at least one condition"):
        spec.validate()


def test_requires_exit_rule() -> None:
    spec = RuleStrategySpec(
        name="x",
        indicators=(IndicatorSpec(id="a", kind="ema", period=5),),
        entry=EntryRule(side="buy", all_of=(Condition(type="rising", left="a"),)),
    )
    with pytest.raises(RuleSpecError, match="at least one exit rule"):
        spec.validate()


def test_percent_of_equity_must_be_fraction() -> None:
    rule = SizeRule(type="percent_of_equity", value=2.0)
    with pytest.raises(RuleSpecError, match="fraction"):
        rule.validate()


def test_exit_value_must_be_fraction_between_0_and_1() -> None:
    with pytest.raises(RuleSpecError, match="fraction"):
        ExitRule(type="stop_loss", value=1.5).validate()


def test_unknown_condition_type_rejected() -> None:
    cond = Condition(type="bogus", left="price")
    with pytest.raises(RuleSpecError, match="unknown condition type"):
        cond.validate({"price"})
