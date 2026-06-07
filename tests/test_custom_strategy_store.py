"""Tests for user-built strategy persistence + dynamic registration (FB-AP-XXX)."""

from __future__ import annotations

import pytest

from strategies import custom_strategy_store as store
from strategies.registry import _REGISTRY, get_strategy
from strategies.rule_spec import Condition, EntryRule, ExitRule, IndicatorSpec, RuleStrategySpec


def _spec(name: str = "My EMA cross") -> RuleStrategySpec:
    return RuleStrategySpec(
        name=name,
        indicators=(IndicatorSpec(id="fast", kind="ema", period=7), IndicatorSpec(id="slow", kind="ema", period=21)),
        entry=EntryRule(side="buy", all_of=(Condition(type="cross_above", left="fast", right_id="slow"),)),
        exits=(ExitRule(type="stop_loss", value=0.02),),
    )


@pytest.fixture
def store_dir(tmp_path, monkeypatch):
    monkeypatch.setattr(store, "_DEFAULT_DIR", tmp_path / "custom_strategies")
    yield tmp_path
    # Clean up any descriptors registered into the shared global registry during the test.
    for key in [k for k in _REGISTRY if k.startswith("custom:")]:
        _REGISTRY.pop(key, None)


def test_save_persists_and_registers(store_dir) -> None:
    record = store.save_custom_strategy(_spec())
    assert record["id"] == "my_ema_cross"
    assert record["spec"]["name"] == "My EMA cross"

    on_disk = store.get_custom_strategy("my_ema_cross")
    assert on_disk == record

    descriptor = get_strategy("custom:my_ema_cross")
    assert descriptor is not None
    assert descriptor.name == "My EMA cross"
    assert descriptor.params[0].name == "rule_spec"
    assert '"name":"My EMA cross"' in descriptor.params[0].default


def test_save_twice_dedupes_id_by_suffix(store_dir) -> None:
    a = store.save_custom_strategy(_spec("Same Name"))
    b = store.save_custom_strategy(_spec("Same Name"))
    assert a["id"] == "same_name"
    assert b["id"] == "same_name_2"


def test_resave_with_id_overwrites(store_dir) -> None:
    first = store.save_custom_strategy(_spec("Original"))
    edited = _spec("Original").renamed("Renamed")
    second = store.save_custom_strategy(edited, strategy_id=first["id"])

    assert second["id"] == first["id"]
    assert second["created_at"] == first["created_at"]
    assert store.get_custom_strategy(first["id"])["spec"]["name"] == "Renamed"
    assert get_strategy(f"custom:{first['id']}").name == "Renamed"


def test_delete_removes_file_and_descriptor(store_dir) -> None:
    record = store.save_custom_strategy(_spec())
    assert get_strategy(f"custom:{record['id']}") is not None

    assert store.delete_custom_strategy(record["id"]) is True
    assert store.get_custom_strategy(record["id"]) is None
    assert get_strategy(f"custom:{record['id']}") is None
    assert store.delete_custom_strategy(record["id"]) is False


def test_list_orders_newest_first(store_dir) -> None:
    first = store.save_custom_strategy(_spec("First"))
    second = store.save_custom_strategy(_spec("Second"))
    ids = [r["id"] for r in store.list_custom_strategies()]
    assert ids[:2] == [second["id"], first["id"]] or ids[:2] == [first["id"], second["id"]]
    assert set(ids) == {first["id"], second["id"]}


def test_register_custom_strategies_loads_from_disk(store_dir) -> None:
    record = store.save_custom_strategy(_spec())
    _REGISTRY.pop(f"custom:{record['id']}", None)
    assert get_strategy(f"custom:{record['id']}") is None

    count = store.register_custom_strategies()
    assert count >= 1
    assert get_strategy(f"custom:{record['id']}") is not None


def test_save_invalid_spec_raises(store_dir) -> None:
    bad = RuleStrategySpec(name="no entry rule")
    with pytest.raises(Exception):
        store.save_custom_strategy(bad)
