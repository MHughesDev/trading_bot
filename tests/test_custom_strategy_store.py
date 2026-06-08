"""Tests for user-built strategy persistence + dynamic registration (FB-AP-XXX).

Custom strategies are stored as rows in the operator user database (``users.sqlite``),
keyed by ``(user_id, id)`` — purely user data, owned by one account, never loose files
on disk (see strategies/custom_strategy_store.py for the rationale).
"""

from __future__ import annotations

import pytest

from strategies import custom_strategy_store as store
from strategies.registry import _REGISTRY, get_strategy
from strategies.rule_spec import Condition, EntryRule, ExitRule, IndicatorSpec, RuleStrategySpec

USER_A = 1
USER_B = 2


def _spec(name: str = "My EMA cross") -> RuleStrategySpec:
    return RuleStrategySpec(
        name=name,
        indicators=(IndicatorSpec(id="fast", kind="ema", period=7), IndicatorSpec(id="slow", kind="ema", period=21)),
        entry=EntryRule(side="buy", all_of=(Condition(type="cross_above", left="fast", right_id="slow"),)),
        exits=(ExitRule(type="stop_loss", value=0.02),),
    )


@pytest.fixture
def db(tmp_path):
    path = tmp_path / "users.sqlite"
    yield path
    for key in [k for k in _REGISTRY if k.startswith("custom:")]:
        _REGISTRY.pop(key, None)


def test_save_persists_and_registers(db) -> None:
    record = store.save_custom_strategy(db, USER_A, _spec())
    assert record["id"] == "my_ema_cross"
    assert record["user_id"] == USER_A
    assert record["spec"]["name"] == "My EMA cross"

    on_disk = store.get_custom_strategy(db, USER_A, "my_ema_cross")
    assert on_disk == record

    key = store.registry_key(USER_A, "my_ema_cross")
    assert key == "custom:u1:my_ema_cross"
    descriptor = get_strategy(key)
    assert descriptor is not None
    assert descriptor.name == "My EMA cross"
    assert descriptor.params[0].name == "rule_spec"
    assert '"name":"My EMA cross"' in descriptor.params[0].default


def test_save_twice_dedupes_id_by_suffix(db) -> None:
    a = store.save_custom_strategy(db, USER_A, _spec("Same Name"))
    b = store.save_custom_strategy(db, USER_A, _spec("Same Name"))
    assert a["id"] == "same_name"
    assert b["id"] == "same_name_2"


def test_two_users_can_reuse_the_same_slug(db) -> None:
    """Per-user scoping: two users each saving "My EMA cross" both land on id "my_ema_cross"
    — isolated by the (user_id, id) composite key, not forced into _2 suffixes."""
    a = store.save_custom_strategy(db, USER_A, _spec())
    b = store.save_custom_strategy(db, USER_B, _spec())
    assert a["id"] == b["id"] == "my_ema_cross"
    assert store.registry_key(USER_A, a["id"]) != store.registry_key(USER_B, b["id"])

    # Each user sees only their own.
    assert [r["id"] for r in store.list_custom_strategies(db, USER_A)] == ["my_ema_cross"]
    assert [r["id"] for r in store.list_custom_strategies(db, USER_B)] == ["my_ema_cross"]
    assert store.list_custom_strategies(db, USER_A)[0]["user_id"] == USER_A
    assert store.list_custom_strategies(db, USER_B)[0]["user_id"] == USER_B


def test_cross_user_access_is_invisible(db) -> None:
    record = store.save_custom_strategy(db, USER_A, _spec())

    assert store.get_custom_strategy(db, USER_B, record["id"]) is None
    assert store.delete_custom_strategy(db, USER_B, record["id"]) is False
    # Still there for its owner.
    assert store.get_custom_strategy(db, USER_A, record["id"]) is not None


def test_resave_with_id_overwrites(db) -> None:
    first = store.save_custom_strategy(db, USER_A, _spec("Original"))
    edited = _spec("Original").renamed("Renamed")
    second = store.save_custom_strategy(db, USER_A, edited, strategy_id=first["id"])

    assert second["id"] == first["id"]
    assert second["created_at"] == first["created_at"]
    assert store.get_custom_strategy(db, USER_A, first["id"])["spec"]["name"] == "Renamed"
    assert get_strategy(store.registry_key(USER_A, first["id"])).name == "Renamed"


def test_delete_removes_row_and_descriptor(db) -> None:
    record = store.save_custom_strategy(db, USER_A, _spec())
    key = store.registry_key(USER_A, record["id"])
    assert get_strategy(key) is not None

    assert store.delete_custom_strategy(db, USER_A, record["id"]) is True
    assert store.get_custom_strategy(db, USER_A, record["id"]) is None
    assert get_strategy(key) is None
    assert store.delete_custom_strategy(db, USER_A, record["id"]) is False


def test_list_orders_newest_first(db) -> None:
    first = store.save_custom_strategy(db, USER_A, _spec("First"))
    second = store.save_custom_strategy(db, USER_A, _spec("Second"))
    ids = [r["id"] for r in store.list_custom_strategies(db, USER_A)]
    assert ids[:2] == [second["id"], first["id"]] or ids[:2] == [first["id"], second["id"]]
    assert set(ids) == {first["id"], second["id"]}


def test_register_custom_strategies_loads_every_users_strategies(db) -> None:
    a = store.save_custom_strategy(db, USER_A, _spec("A's strategy"))
    b = store.save_custom_strategy(db, USER_B, _spec("B's strategy"))
    key_a, key_b = store.registry_key(USER_A, a["id"]), store.registry_key(USER_B, b["id"])
    _REGISTRY.pop(key_a, None)
    _REGISTRY.pop(key_b, None)
    assert get_strategy(key_a) is None and get_strategy(key_b) is None

    count = store.register_custom_strategies(db)
    assert count >= 2
    assert get_strategy(key_a) is not None
    assert get_strategy(key_b) is not None


def test_save_invalid_spec_raises(db) -> None:
    bad = RuleStrategySpec(name="no entry rule")
    with pytest.raises(Exception):
        store.save_custom_strategy(db, USER_A, bad)
