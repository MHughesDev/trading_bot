"""Strategy builder — sentence/block mode (FB-AP-XXX).

Lets a user assemble a trading strategy from plain-language pieces — *"WHEN the 7 EMA
crosses above the 21 EMA THEN buy 2% of equity, EXIT WHEN down 1.5% or up 4%"* — without
writing code. Every choice compiles into the same universal
:class:`strategies.rule_spec.RuleStrategySpec` JSON that the node-graph and code-mode
builders will target too (see ``strategies/rule_spec.py``), and that the single shared
:class:`strategies.rule_based_strategy.RuleBasedStrategy` runs at backtest/live time — so a
strategy built here slots into the existing catalogue (``/strategies``), backtest
(``/assets/backtest/{symbol}``), and live-assignment (``/assets/strategy/{symbol}``)
endpoints exactly like a hand-written one.

The bottom of the page always shows a plain-English explanation of the strategy as currently
configured — "every strategy should generate a human-readable explanation" was an explicit
product requirement, so a user can sanity-check what they're about to risk money on before
they save it.
"""

from __future__ import annotations

from typing import Any

import httpx
import streamlit as st

from control_plane.streamlit_util import api_delete_json, api_get_json, api_post_json, api_put_json
from strategies.rule_spec import CONDITION_TYPES, EXIT_TYPES, INDICATOR_KINDS, SIDES, SIZE_TYPES

_INDICATORS_KEY = "sb_indicators"
_CONDITIONS_KEY = "sb_conditions"
_EXITS_KEY = "sb_exits"

_INDICATOR_LABELS = {k: v for k, v in INDICATOR_KINDS.items()}
_CONDITION_LABELS = {k: v.capitalize() for k, v in CONDITION_TYPES.items()}
_EXIT_LABELS = {k: v.replace("_", " ").title() for k, v in EXIT_TYPES.items()}
_SIZE_LABELS = {"percent_of_equity": "Percent of account equity", "fixed_quantity": "Fixed quantity"}


def _reset_builder_state() -> None:
    st.session_state[_INDICATORS_KEY] = [
        {"id": "ema_fast", "kind": "ema", "period": 7},
        {"id": "ema_slow", "kind": "ema", "period": 21},
    ]
    st.session_state[_CONDITIONS_KEY] = [
        {"type": "cross_above", "left": "ema_fast", "right_mode": "indicator", "right": "ema_slow"}
    ]
    st.session_state[_EXITS_KEY] = [
        {"type": "stop_loss", "value": 1.5},
        {"type": "take_profit", "value": 4.0},
    ]
    st.session_state["sb_name"] = "My new strategy"
    st.session_state["sb_side"] = "buy"
    st.session_state["sb_size_type"] = "percent_of_equity"
    st.session_state["sb_size_value"] = 2.0
    st.session_state["sb_editing_id"] = None


def _ensure_state() -> None:
    if _INDICATORS_KEY not in st.session_state:
        _reset_builder_state()


def _load_into_builder(record: dict[str, Any]) -> None:
    spec = record["spec"]
    st.session_state[_INDICATORS_KEY] = [dict(i) for i in spec.get("indicators", [])]
    conditions = []
    entry = spec.get("entry") or {}
    for cond in [*entry.get("all", []), *entry.get("any", [])]:
        if "right_id" in cond:
            conditions.append({"type": cond["type"], "left": cond["left"], "right_mode": "indicator", "right": cond["right_id"]})
        else:
            conditions.append(
                {"type": cond["type"], "left": cond["left"], "right_mode": "value", "right": cond.get("right_value", 0.0)}
            )
    st.session_state[_CONDITIONS_KEY] = conditions or [
        {"type": "cross_above", "left": "price", "right_mode": "value", "right": 0.0}
    ]
    size = spec.get("size") or {}
    st.session_state["sb_size_type"] = size.get("type", "percent_of_equity")
    st.session_state["sb_size_value"] = float(size.get("value", 0.02)) * (
        100.0 if size.get("type", "percent_of_equity") == "percent_of_equity" else 1.0
    )
    st.session_state[_EXITS_KEY] = [
        {"type": e["type"], "value": float(e["value"]) * 100.0} for e in spec.get("exits", [])
    ] or [{"type": "stop_loss", "value": 1.5}]
    st.session_state["sb_name"] = spec.get("name", "")
    st.session_state["sb_side"] = entry.get("side", "buy")
    st.session_state["sb_editing_id"] = record["id"]


def _indicator_options() -> list[str]:
    return ["price"] + [ind["id"] for ind in st.session_state[_INDICATORS_KEY] if ind.get("id")]


def _build_spec_dict() -> dict[str, Any]:
    indicators = [
        {"id": ind["id"].strip(), "kind": ind["kind"], "period": int(ind["period"])}
        for ind in st.session_state[_INDICATORS_KEY]
        if ind.get("id", "").strip()
    ]
    conditions = []
    for cond in st.session_state[_CONDITIONS_KEY]:
        entry: dict[str, Any] = {"type": cond["type"], "left": cond["left"]}
        if cond["type"] not in ("rising", "falling"):
            if cond["right_mode"] == "indicator":
                entry["right_id"] = cond["right"]
            else:
                entry["right_value"] = float(cond["right"])
        conditions.append(entry)
    size_value = float(st.session_state["sb_size_value"])
    if st.session_state["sb_size_type"] == "percent_of_equity":
        size_value = size_value / 100.0
    return {
        "name": st.session_state["sb_name"].strip(),
        "indicators": indicators,
        "entry": {"side": st.session_state["sb_side"], "all": conditions, "any": []},
        "size": {"type": st.session_state["sb_size_type"], "value": size_value},
        "exits": [
            {"type": e["type"], "value": float(e["value"]) / 100.0} for e in st.session_state[_EXITS_KEY]
        ],
    }


def _render_indicators_editor() -> None:
    st.markdown("##### 1 · Indicators")
    st.caption("Name each indicator you want to track — you'll reference these names in your conditions below.")
    indicators: list[dict[str, Any]] = st.session_state[_INDICATORS_KEY]
    remove_at: int | None = None
    for i, ind in enumerate(indicators):
        cols = st.columns([3, 3, 2, 1])
        ind["id"] = cols[0].text_input("Name", value=ind["id"], key=f"sb_ind_id_{i}")
        ind["kind"] = cols[1].selectbox(
            "Type",
            options=list(_INDICATOR_LABELS),
            index=list(_INDICATOR_LABELS).index(ind["kind"]) if ind["kind"] in _INDICATOR_LABELS else 0,
            format_func=lambda k: _INDICATOR_LABELS[k],
            key=f"sb_ind_kind_{i}",
        )
        ind["period"] = cols[2].number_input(
            "Period (bars)", min_value=1, max_value=1000, value=int(ind["period"]), step=1, key=f"sb_ind_period_{i}"
        )
        if cols[3].button("✕", key=f"sb_ind_remove_{i}", help="Remove this indicator"):
            remove_at = i
    if remove_at is not None:
        indicators.pop(remove_at)
        st.rerun()
    if st.button("+ Add indicator", key="sb_add_indicator"):
        n = len(indicators) + 1
        indicators.append({"id": f"indicator_{n}", "kind": "ema", "period": 14})
        st.rerun()


def _render_conditions_editor() -> None:
    st.markdown("##### 2 · WHEN should it enter a trade?")
    st.caption("All of these conditions must be true together (AND) for the strategy to enter.")
    options = _indicator_options()
    conditions: list[dict[str, Any]] = st.session_state[_CONDITIONS_KEY]
    remove_at: int | None = None
    for i, cond in enumerate(conditions):
        cols = st.columns([3, 3, 3, 2, 1])
        left_options = options or ["price"]
        cond["left"] = cols[0].selectbox(
            "When",
            options=left_options,
            index=left_options.index(cond["left"]) if cond["left"] in left_options else 0,
            key=f"sb_cond_left_{i}",
        )
        cond["type"] = cols[1].selectbox(
            "Condition",
            options=list(_CONDITION_LABELS),
            index=list(_CONDITION_LABELS).index(cond["type"]) if cond["type"] in _CONDITION_LABELS else 0,
            format_func=lambda k: _CONDITION_LABELS[k],
            key=f"sb_cond_type_{i}",
        )
        if cond["type"] not in ("rising", "falling"):
            mode = cols[2].radio(
                "Compare to",
                options=["indicator", "value"],
                index=0 if cond["right_mode"] == "indicator" else 1,
                format_func=lambda m: "Another indicator" if m == "indicator" else "A number",
                key=f"sb_cond_mode_{i}",
                horizontal=True,
            )
            cond["right_mode"] = mode
            if mode == "indicator":
                right_options = options or ["price"]
                current = cond["right"] if cond["right"] in right_options else right_options[0]
                cond["right"] = cols[3].selectbox(
                    "Indicator", options=right_options, index=right_options.index(current), key=f"sb_cond_right_ind_{i}"
                )
            else:
                cond["right"] = cols[3].number_input(
                    "Value", value=float(cond["right"]) if isinstance(cond["right"], (int, float)) else 0.0,
                    key=f"sb_cond_right_val_{i}",
                )
        else:
            cols[2].caption("(no comparison needed)")
        if cols[4].button("✕", key=f"sb_cond_remove_{i}", help="Remove this condition"):
            remove_at = i
    if remove_at is not None:
        conditions.pop(remove_at)
        st.rerun()
    if st.button("+ Add condition", key="sb_add_condition"):
        conditions.append(
            {"type": "greater_than", "left": options[0] if options else "price", "right_mode": "value", "right": 0.0}
        )
        st.rerun()


def _render_action_and_size() -> None:
    st.markdown("##### 3 · THEN do what?")
    cols = st.columns([2, 2, 2])
    st.session_state["sb_side"] = cols[0].selectbox(
        "Action",
        options=list(SIDES),
        index=list(SIDES).index(st.session_state["sb_side"]) if st.session_state["sb_side"] in SIDES else 0,
        format_func=lambda s: SIDES[s].capitalize(),
        key="sb_side_select",
    )
    st.session_state["sb_size_type"] = cols[1].selectbox(
        "Size by",
        options=list(_SIZE_LABELS),
        index=list(_SIZE_LABELS).index(st.session_state["sb_size_type"]),
        format_func=lambda k: _SIZE_LABELS[k],
        key="sb_size_type_select",
    )
    suffix = "%" if st.session_state["sb_size_type"] == "percent_of_equity" else "units"
    st.session_state["sb_size_value"] = cols[2].number_input(
        f"Amount ({suffix})",
        min_value=0.0,
        value=float(st.session_state["sb_size_value"]),
        key="sb_size_value_input",
    )


def _render_exits_editor() -> None:
    st.markdown("##### 4 · EXIT when…")
    st.caption("Risk controls that close the position automatically — pick at least one.")
    exits: list[dict[str, Any]] = st.session_state[_EXITS_KEY]
    remove_at: int | None = None
    for i, ex in enumerate(exits):
        cols = st.columns([3, 3, 1])
        ex["type"] = cols[0].selectbox(
            "Rule",
            options=list(_EXIT_LABELS),
            index=list(_EXIT_LABELS).index(ex["type"]) if ex["type"] in _EXIT_LABELS else 0,
            format_func=lambda k: _EXIT_LABELS[k],
            key=f"sb_exit_type_{i}",
        )
        ex["value"] = cols[1].number_input(
            "Move (%)", min_value=0.0, max_value=100.0, value=float(ex["value"]), key=f"sb_exit_value_{i}"
        )
        if cols[2].button("✕", key=f"sb_exit_remove_{i}", help="Remove this exit rule"):
            remove_at = i
    if remove_at is not None:
        exits.pop(remove_at)
        st.rerun()
    if st.button("+ Add exit rule", key="sb_add_exit"):
        exits.append({"type": "stop_loss", "value": 2.0})
        st.rerun()


def _render_preview_and_save() -> None:
    st.markdown("##### Preview")
    spec_dict = _build_spec_dict()
    try:
        preview = api_post_json("/strategies/custom/preview", spec_dict, require_key=False)
    except Exception as e:
        st.warning(f"Couldn't reach the preview service: {e}")
        return

    if preview.get("explanation"):
        st.info(preview["explanation"])
    if preview.get("valid"):
        st.success("This strategy is valid and ready to save.")
    else:
        for err in preview.get("errors", []):
            st.error(err)

    editing_id = st.session_state.get("sb_editing_id")
    label = "Save changes" if editing_id else "Save strategy"
    if st.button(label, type="primary", disabled=not preview.get("valid"), key="sb_save"):
        try:
            if editing_id:
                saved = api_put_json(f"/strategies/custom/{editing_id}", spec_dict)
            else:
                saved = api_post_json("/strategies/custom", spec_dict)
            st.session_state["sb_editing_id"] = saved["id"]
            st.success(f"Saved “{saved['name']}” — it now appears in the strategy catalogue as `{saved['registry_key']}`.")
            st.rerun()
        except httpx.HTTPStatusError as e:
            st.error(f"Save failed ({e.response.status_code}): {e.response.text}")
        except Exception as e:
            st.error(f"Save failed: {e}")


def _render_saved_strategies() -> None:
    st.markdown("#### Your saved strategies")
    try:
        listing = api_get_json("/strategies/custom")
    except Exception as e:
        st.caption(f"Couldn't load saved strategies: {e}")
        return
    records = listing.get("strategies") or []
    if not records:
        st.caption("Nothing saved yet — build one above and click **Save strategy**.")
        return
    for record in records:
        with st.expander(f"{record['name']}  ·  `{record['registry_key']}`", expanded=False):
            st.write(record.get("explanation") or "")
            cols = st.columns([1, 1, 4])
            if cols[0].button("Edit", key=f"sb_edit_{record['id']}"):
                _load_into_builder(record)
                st.rerun()
            if cols[1].button("Delete", key=f"sb_delete_{record['id']}"):
                try:
                    api_delete_json(f"/strategies/custom/{record['id']}")
                    st.rerun()
                except Exception as e:
                    st.error(f"Delete failed: {e}")


def render_strategy_builder_page() -> None:
    _ensure_state()

    st.caption(
        "Build a trading strategy from plain-language building blocks — no code required. "
        "Pick the indicators you care about, describe when to enter and exit, and the "
        "explanation below shows exactly what you've built before you save it. Once saved, "
        "it shows up in the strategy catalogue and can be backtested or assigned to any "
        "asset just like a built-in strategy."
    )

    if st.session_state.get("sb_editing_id"):
        st.info(f"Editing **{st.session_state['sb_name']}** — saving will update it in place.")
    cols = st.columns([4, 1])
    st.session_state["sb_name"] = cols[0].text_input("Strategy name", value=st.session_state["sb_name"], key="sb_name_input")
    if cols[1].button("Start over", key="sb_reset"):
        _reset_builder_state()
        st.rerun()

    st.divider()
    _render_indicators_editor()
    st.divider()
    _render_conditions_editor()
    st.divider()
    _render_action_and_size()
    st.divider()
    _render_exits_editor()
    st.divider()
    _render_preview_and_save()

    st.divider()
    _render_saved_strategies()
